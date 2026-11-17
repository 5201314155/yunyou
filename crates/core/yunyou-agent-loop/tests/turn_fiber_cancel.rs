//! S7-R1 取消源合并·集成验收（方案 §5.1 R1 + §5.3 验收 2）：
//! fiber 卸载取消线（插件被禁用/卸载）独立中断在途轮，且卸载后无孤儿服务。
//!
//! 与 `turn_cancel.rs`（调用方令牌线）对偶：两源各自独立触发都能收流；
//! 本文件补第二条线——`FiberHandle::dispose()` 即 fiber 卸载取消，
//! 断言：①在途轮 Err(loop/cancelled) 且不发 TurnComplete；②卸载后
//! `dyn AgentLoop` 服务不可见（provide 随 effect 逆序清空）；③fiber 状态
//! 正确回落 Disposed（RunService 状态机收口）。

mod common;

use common::{ensure_commands, wait_active, FakeLlm, FakeLlmPlugin, HangingLlmPlugin};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use yunyou::{Context, FiberState, ServiceId};
use yunyou_agent_loop::{AgentLoop, AgentLoopPlugin, CollectingObserver, TurnEvent};
use yunyou_session::SessionPlugin;
use yunyou_tools::ToolsPlugin;

/// 装配一套最小环境（会话/工具/挂起 LLM/系统提示），返回 root 与 agent-loop 句柄。
///
/// `tag` 进临时目录名：同二进制内多用例并行，目录必须互不重名（防互清会话文件）。
async fn mount(tag: &str) -> (Context, yunyou::FiberHandle) {
    let dir = std::env::temp_dir().join(format!("dsh-loop-fiber-cancel-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = Context::new();
    wait_active(
        &root.plugin(SessionPlugin, serde_json::json!({ "path": dir.join("s.jsonl").to_string_lossy() })),
        "session",
    )
    .await;
    ensure_commands(&root).await;
wait_active(
        &root.plugin(ToolsPlugin, serde_json::json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(&root.plugin(HangingLlmPlugin, serde_json::json!({})), "hanging-llm").await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})),
        "system-prompt",
    )
    .await;
    let loop_handle = root.plugin(AgentLoopPlugin, serde_json::json!({}));
    wait_active(&loop_handle, "agent-loop").await;
    (root, loop_handle)
}

/// 场景一（R1 主线）：fiber 卸载取消 → 在途轮收流。
///
/// 模拟「插件被禁用」：挂起流对话进行中 `dispose()` agent-loop fiber，
/// 取消令牌经合并点传播 → 本轮 Err(loop/cancelled)、无 TurnComplete。
#[tokio::test(flavor = "multi_thread")]
async fn fiber卸载取消打断在途轮() {
    let (root, loop_handle) = mount("unload").await;
    let collector = CollectingObserver::new();
    let cancel = CancellationToken::new();
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let task = tokio::spawn(loop_svc.run_turn_observed("慢慢来", cancel.clone(), collector.clone()));

    tokio::time::sleep(Duration::from_millis(150)).await;
    // 计数器证据：轮次确已进入流式等待（UserMessage 已落盘并外露）
    assert!(
        collector
            .snapshot()
            .iter()
            .any(|e| matches!(e, TurnEvent::UserMessage { .. })),
        "卸载前应已进入对话轮"
    );
    // 模拟插件禁用：卸载 fiber（FiberShared.cancel 触发，方案 §5.1 R1 第二源）
    loop_handle.dispose().await.expect("卸载应成功");

    let err = task.await.expect("任务不应 panic").expect_err("卸载取消应返回 Err");
    assert!(err.message().contains("loop/cancelled"), "{err}");
    assert!(
        !collector
            .snapshot()
            .iter()
            .any(|e| matches!(e, TurnEvent::TurnComplete { .. })),
        "Err 路径不发 TurnComplete"
    );
}

/// 场景二（§5.3 验收 2·无残尸）：卸载后无孤儿服务 + 状态正确回落。
///
/// ①`dyn AgentLoop` 服务随 effect 逆序清空（根上下文再取不到）；
/// ②agent-loop fiber 状态回落 `Disposed`（RunService 状态机收口）；
/// ③同根其余插件（session）不受牵连、仍在 Active（只拔一张卡，不断电）。
#[tokio::test(flavor = "multi_thread")]
async fn 卸载后无孤儿服务且状态回落() {
    let (root, loop_handle) = mount("orphan").await;
    let collector = CollectingObserver::new();
    let cancel = CancellationToken::new();
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let task = tokio::spawn(loop_svc.run_turn_observed("慢慢来", cancel.clone(), collector.clone()));
    tokio::time::sleep(Duration::from_millis(150)).await;
    loop_handle.dispose().await.expect("卸载应成功");
    let _ = task.await.expect("任务不应 panic").expect_err("卸载取消应返回 Err");

    // ① 无孤儿服务：AgentLoop 已随 effect 逆序清空，根上下文再取不到
    let gone = root.clone().get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>());
    assert!(gone.is_err(), "卸载后 AgentLoop 服务应不可见（无孤儿服务）");
    // ② 状态回落：agent-loop fiber → Disposed
    assert_eq!(loop_handle.state(), FiberState::Disposed, "fiber 状态应回落 Disposed");
    // ③ 未牵连旁路：同一根上的 session 插件仍 Active（逆序撤销只清本 fiber）
    let session = root
        .clone()
        .get::<dyn yunyou_session::SessionStore>(ServiceId::of::<dyn yunyou_session::SessionStore>())
        .expect("session 服务应仍在");
    let events = session.load_all().await.expect("会话可读");
    assert!(
        events.iter().any(|e| matches!(e, yunyou_session::SessionEvent::UserMessage { .. })),
        "卸载前落盘的 UserMessage 应保留（事实源不丢）"
    );
}

/// 场景三（不回归镜像）：调用方令牌线在合并逻辑下照样独立中断。
///
/// turn_cancel.rs 已覆盖同场景；此处确认合并改造未削弱原线（回归护栏）。
#[tokio::test(flavor = "multi_thread")]
async fn 调用方取消线不回归() {
    let (root, _loop_handle) = mount("caller").await;
    let collector = CollectingObserver::new();
    let cancel = CancellationToken::new();
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let task = tokio::spawn(loop_svc.run_turn_observed("慢慢来", cancel.clone(), collector.clone()));
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    let err = task.await.expect("任务不应 panic").expect_err("调用方取消应返回 Err");
    assert!(err.message().contains("loop/cancelled"), "{err}");
    assert!(
        !collector
            .snapshot()
            .iter()
            .any(|e| matches!(e, TurnEvent::TurnComplete { .. })),
        "Err 路径不发 TurnComplete"
    );
}

/// 场景三·补（§5.3 验收 2 第一条）：fiber 卸载取消打进**工具执行中**的在途轮。
///
/// 慢命令执行中被卸载 → 合一令牌传到 execute_tool → 工具收取消
/// （ToolResult is_error=tool/cancelled，executor 等其静止）→ 循环顶部退出
/// → Err 且无 TurnComplete；服务随 effect 清空、fiber 回落 Disposed。
#[tokio::test(flavor = "multi_thread")]
async fn 工具执行中fiber卸载取消() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-fiber-cancel-tool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = Context::new();
    wait_active(
        &root.plugin(SessionPlugin, serde_json::json!({ "path": dir.join("s.jsonl").to_string_lossy() })),
        "session",
    )
    .await;
    ensure_commands(&root).await;
wait_active(
        &root.plugin(ToolsPlugin, serde_json::json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(
        &root.plugin(
            FakeLlmPlugin {
                llm: Arc::new(FakeLlm::new(vec![vec![
                    yunyou_llm::StreamDelta::ToolCall(yunyou_llm::ToolCallDelta {
                        index: 0,
                        id: Some("call-slow".into()),
                        name: Some("bash".into()),
                        arguments_delta: "{\"cmd\":\"sleep 30\"}".into(),
                    }),
                    yunyou_llm::StreamDelta::Done { finish_reason: Some("tool_calls".into()), usage: None },
                ]])),
            },
            serde_json::json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})),
        "system-prompt",
    )
    .await;
    let loop_handle = root.plugin(AgentLoopPlugin, serde_json::json!({}));
    wait_active(&loop_handle, "agent-loop").await;

    let collector = CollectingObserver::new();
    let task = tokio::spawn(
        root.clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见")
            .run_turn_observed("跑个慢命令", CancellationToken::new(), collector.clone()),
    );
    // 等 ToolCall 事件到达（工具已进入执行）
    let mut saw_tool_call = false;
    for _ in 0..100 {
        if collector
            .snapshot()
            .iter()
            .any(|e| matches!(e, TurnEvent::ToolCall { .. }))
        {
            saw_tool_call = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(saw_tool_call, "应先收到 ToolCall 事件");
    tokio::time::sleep(Duration::from_millis(50)).await; // 进入 bash 执行
    loop_handle.dispose().await.expect("卸载应成功");

    let err = task.await.expect("任务不应 panic").expect_err("卸载取消应返回 Err");
    assert!(err.message().contains("loop/cancelled"), "{err}");
    match collector.snapshot().last() {
        Some(TurnEvent::ToolResult { is_error, content, .. }) => {
            assert!(*is_error, "被取消的工具结果应为错误态");
            assert_eq!(content["code"], "tool/cancelled", "取消错误码: {content}");
        }
        other => panic!("末事件应为 ToolResult: {other:?}"),
    }
    // 无孤儿服务 + 状态回落（与场景二同口径）
    assert!(root.clone().get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>()).is_err());
    assert_eq!(loop_handle.state(), FiberState::Disposed);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 场景四（两源静默不误伤）：正常轮在两源均未触发时照常完成。
///
/// 合并只做「任一触发即收流」，不得给正常轮引入伪取消——TurnComplete 到场。
#[tokio::test(flavor = "multi_thread")]
async fn 两源静默时正常轮不受影响() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-fiber-cancel-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = Context::new();
    wait_active(
        &root.plugin(SessionPlugin, serde_json::json!({ "path": dir.join("s.jsonl").to_string_lossy() })),
        "session",
    )
    .await;
    ensure_commands(&root).await;
wait_active(
        &root.plugin(ToolsPlugin, serde_json::json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(
        &root.plugin(
            FakeLlmPlugin {
                llm: Arc::new(FakeLlm::new(vec![vec![
                    yunyou_llm::StreamDelta::Text("收到。".into()),
                    yunyou_llm::StreamDelta::Done { finish_reason: Some("stop".into()), usage: None },
                ]])),
            },
            serde_json::json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})),
        "system-prompt",
    )
    .await;
    let loop_handle = root.plugin(AgentLoopPlugin, serde_json::json!({}));
    wait_active(&loop_handle, "agent-loop").await;

    let collector = CollectingObserver::new();
    let outcome = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见")
        .run_turn_observed("你好", CancellationToken::new(), collector.clone())
        .await
        .expect("两源静默时正常轮应成功");
    assert_eq!(outcome.tool_calls, 0);
    assert!(
        collector
            .snapshot()
            .iter()
            .any(|e| matches!(e, TurnEvent::TurnComplete { .. })),
        "正常轮应发 TurnComplete"
    );
    // 本轮结束后 fiber 仍 Active（桥接任务已随守卫 drop 中止，无跨轮监听）
    assert_eq!(loop_handle.state(), FiberState::Active, "正常轮后 fiber 应仍 Active");
    let _ = std::fs::remove_dir_all(&dir);
}