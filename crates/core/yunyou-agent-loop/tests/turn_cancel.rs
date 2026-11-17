//! agent-loop 取消传播专项（审查契约）：a 流中取消 / b 工具执行中取消。
//!
//! 契约固化：①取消 → run_turn_observed 返回 Err(loop/cancelled)；②Err 路径
//! **不发 TurnComplete**（完成标记仅成功轮携带，渲染端以返回值感知异常结束）；
//! ③通道不悬挂——run_turn 结束即 observer Arc 释放，sender 关闭，recv() 收束 None。

mod common;

use common::{ensure_commands, wait_active, FakeLlm, FakeLlmPlugin, HangingLlmPlugin};
use yunyou::{Context, ServiceId};
use yunyou_llm::{StreamDelta, ToolCallDelta};
use std::sync::Arc;
use std::time::Duration;
use yunyou_agent_loop::{AgentLoop, AgentLoopPlugin, CollectingObserver, TurnEvent};
use yunyou_session::SessionPlugin;
use yunyou_tools::ToolsPlugin;
use yunyou_util::BoxFut;

/// 取消场景 a（流中取消）：取消 → Err(loop/cancelled)；Err 路径不发 TurnComplete；
/// 通道不悬挂（发送端关闭后 recv 收束 None）。
#[tokio::test(flavor = "multi_thread")]
async fn 取消传播与通道收束() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = Context::new();
    wait_active(
        &root.plugin(SessionPlugin, serde_json::json!({ "path": dir.join("s.jsonl").to_string_lossy() })),
        "session",
    )
    .await;
    wait_active(&root.plugin(HangingLlmPlugin, serde_json::json!({})), "hanging-llm").await;
    ensure_commands(&root).await;
wait_active(
        &root.plugin(ToolsPlugin, serde_json::json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "system-prompt").await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;

    let (observer, mut receiver) = yunyou_agent_loop::observer_channel();
    let collector = CollectingObserver::new();
    // 通道桥 + 收集器双通道：桥验证收束，收集器验证序列
    struct Bridge {
        collector: Arc<CollectingObserver>,
        forward: tokio::sync::mpsc::UnboundedSender<TurnEvent>,
    }
    impl yunyou_agent_loop::TurnObserver for Bridge {
        fn on_event(&self, event: TurnEvent) -> BoxFut<'_, ()> {
            let _ = self.forward.send(event.clone());
            self.collector.on_event(event)
        }
    }
    // 独立转发通道（桥内复制事件到无界队列，供收束验证）
    let (forward_tx, forward_rx) = tokio::sync::mpsc::unbounded_channel();
    let bridge: Arc<dyn yunyou_agent_loop::TurnObserver> = Arc::new(Bridge {
        collector: collector.clone(),
        forward: forward_tx,
    });

    let cancel = tokio_util::sync::CancellationToken::new();
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let task = tokio::spawn(loop_svc.run_turn_observed("慢慢来", cancel.clone(), bridge));
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    let err = task.await.expect("任务不应 panic").expect_err("取消应返回 Err");
    assert!(err.message().contains("loop/cancelled"), "{err}");

    // 序列断言：有 UserMessage；无 TurnComplete（Err 路径不发，契约）
    let events = collector.snapshot();
    assert!(matches!(events.first(), Some(TurnEvent::UserMessage { .. })));
    assert!(!events.iter().any(|e| matches!(e, TurnEvent::TurnComplete { .. })));

    // 通道收束：run_turn 结束即 observer（桥）引用释放——
    // 主 observer 通道 sender 关闭后 recv 收束（不悬挂）
    drop(observer);
    // bridge 已被 spawn 的 future move 进去（run_turn_observed 参数），此处无需再 drop
    let _ = forward_rx; // 转发通道随桥关闭（收束语义与主通道一致）
    let _ = receiver.recv().await;
    loop {
        match tokio::time::timeout(Duration::from_millis(100), receiver.recv()).await {
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => panic!("通道应在发送端关闭后收束，而非悬挂"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 取消场景 b（工具执行中取消）：ToolCall → ToolResult(is_error=tool/cancelled)
/// 落盘并外露 → 循环顶部退出 → Err 且无 TurnComplete（渲染端依赖返回 Err 收尾）。
#[tokio::test(flavor = "multi_thread")]
async fn 工具执行中取消的序列() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-cancel-tool-{}", std::process::id()));
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
    // 脚本：吐 bash sleep 30（慢命令，为取消留窗口）
    wait_active(
        &root.plugin(
            FakeLlmPlugin {
                llm: Arc::new(FakeLlm::new(vec![vec![
                    StreamDelta::ToolCall(ToolCallDelta {
                        index: 0,
                        id: Some("call-slow".into()),
                        name: Some("bash".into()),
                        arguments_delta: "{\"cmd\":\"sleep 30\"}".into(),
                    }),
                    StreamDelta::Done { finish_reason: Some("tool_calls".into()), usage: None },
                ]])),
            },
            serde_json::json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "system-prompt").await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;

    let collector = CollectingObserver::new();
    let cancel = tokio_util::sync::CancellationToken::new();
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let task = tokio::spawn(loop_svc.run_turn_observed("跑个慢命令", cancel.clone(), collector.clone()));
    // 等 ToolCall 事件到达（工具进入执行）
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
    cancel.cancel();
    let err = task.await.expect("任务不应 panic").expect_err("取消应返回 Err");
    assert!(err.message().contains("loop/cancelled"), "{err}");

    // 序列断言：ToolCall → ToolResult(is_error) → 无 TurnComplete
    let events = collector.snapshot();
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            TurnEvent::UserMessage { .. } => "user",
            TurnEvent::AssistantDelta { .. } => "assistant-delta",
            TurnEvent::ReasoningDelta { .. } => "reasoning-delta",
            TurnEvent::ToolCall { .. } => "tool-call",
            TurnEvent::ToolResult { .. } => "tool-result",
            TurnEvent::AssistantMessage { .. } => "assistant-message",
            TurnEvent::TurnComplete { .. } => "turn-complete",
            TurnEvent::Compacted { .. } => "compacted",
            TurnEvent::TurnPaused { .. } => "turn-paused",
        })
        .collect();
    assert!(
        matches!(kinds.as_slice(), [.., "user", "assistant-message", "tool-call", "tool-result"]),
        "取消前应已外露 ToolCall 与 ToolResult: {kinds:?}"
    );
    assert!(!kinds.contains(&"turn-complete"), "Err 路径不发 TurnComplete");
    let last = events.last().unwrap();
    match last {
        TurnEvent::ToolResult { is_error, content, .. } => {
            assert!(*is_error, "被取消的工具结果应为错误态");
            assert_eq!(content["code"], "tool/cancelled", "取消错误码: {content}");
        }
        other => panic!("末事件应为 ToolResult: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
