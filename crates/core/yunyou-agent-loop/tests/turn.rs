//! agent-loop 主链路集成测试：全链路工具循环、纯文本轮、事件外露（task-10 A 线）。

mod common;

use common::{ensure_commands, wait_active, FakeLlm, FakeLlmPlugin, tool_then_answer_script};
use yunyou::{Context, ServiceId};
use yunyou_llm::StreamDelta;
use yunyou_agent_loop::{AgentLoop, AgentLoopPlugin, CollectingObserver, TurnEvent};
use yunyou_session::{SessionEvent, SessionStore, SessionPlugin};
use yunyou_tools::{RegistryService, ToolsPlugin};
use std::sync::Arc;

/// 全链路：装配 → 一轮带工具的对话 → 事件落盘校验 → 重启恢复校验。
#[tokio::test(flavor = "multi_thread")]
async fn 全链路工具循环与会话恢复() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let session_path = dir.join("session.jsonl");

    let root = Context::new();
    wait_active(
        &root.plugin(SessionPlugin, serde_json::json!({ "path": session_path.to_string_lossy() })),
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
            FakeLlmPlugin { llm: Arc::new(FakeLlm::new(tool_then_answer_script())) },
            serde_json::json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "system-prompt").await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;

    let registry = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("注册表应可见");
    assert_eq!(
        registry.schemas().len(),
        10,
        "十内置工具应已注册（注册面演进史：7=六内置+apply_patch → 8=+write_verify（task-70）→ 10=+task_output/task_stop/task_list（池 task-20 后台任务三件，默认装配=后台注册表在位））"
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome = {
        let loop_svc = root
            .clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见");
        loop_svc.run_turn("请跑命令并告诉我输出", cancel).await.expect("本轮应成功")
    };
    assert!(outcome.assistant_text.contains("hello-from-tool"), "最终回答应引用工具输出: {}", outcome.assistant_text);
    assert_eq!(outcome.tool_calls, 1);
    assert!(!outcome.truncated);

    let store = root
        .get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>())
        .expect("存储应可见");
    let events = store.load_all().await.expect("恢复应成功");
    let kinds: Vec<&str> = events.iter().map(SessionEvent::kind).collect();
    assert_eq!(
        kinds,
        vec!["user-message", "assistant-message", "tool-call", "tool-result", "assistant-message"],
        "事件序列: {kinds:?}"
    );
    assert!(matches!(&events[3], SessionEvent::ToolResult { is_error: false, content, .. }
        if content.contains("hello-from-tool")));
    assert!(matches!(&events[2], SessionEvent::ToolCall { arguments, .. }
        if arguments.contains("hello-from-tool")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 纯文本轮：无工具步（事件只有 user+assistant）。
#[tokio::test(flavor = "multi_thread")]
async fn 纯文本轮() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-plain-{}", std::process::id()));
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
                    StreamDelta::Text("好的。".into()),
                    StreamDelta::Done { finish_reason: Some("stop".into()), usage: None },
                ]])),
            },
            serde_json::json!({}),
        ),
        "plain-llm",
    )
    .await;
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "system-prompt").await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;

    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("应可见")
        .run_turn("hi", cancel)
        .await
        .expect("纯文本轮应成功");
    assert_eq!(outcome.tool_calls, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 观察者形态全链路（task-10 A 线）：事件序列、流式增量、详情数据面（canonical value）、
/// usage 透传、树归属字段、TurnComplete 同源；并验证无观察者兼容形态照常工作。
#[tokio::test(flavor = "multi_thread")]
async fn 观察者事件序列() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-obs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let session_path = dir.join("session.jsonl");

    let root = Context::new();
    wait_active(
        &root.plugin(SessionPlugin, serde_json::json!({ "path": session_path.to_string_lossy() })),
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
            FakeLlmPlugin { llm: Arc::new(FakeLlm::new(tool_then_answer_script())) },
            serde_json::json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "system-prompt").await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;

    let collector = CollectingObserver::new();
    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见")
        .run_turn_observed("请跑命令", cancel, collector.clone())
        .await
        .expect("本轮应成功");
    assert_eq!(outcome.tool_calls, 1);

    let events = collector.snapshot();
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| match event {
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
    assert_eq!(kinds.first(), Some(&"user"), "首事件为用户输入: {kinds:?}");
    assert_eq!(kinds.last(), Some(&"turn-complete"), "末事件为本轮完成: {kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "tool-call").count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == "tool-result").count(), 1);
    // 增量先于落盘（过程态先于终态）
    let first_delta = kinds.iter().position(|k| *k == "assistant-delta").unwrap();
    let first_message = kinds.iter().position(|k| *k == "assistant-message").unwrap();
    assert!(first_delta < first_message, "流式增量应先于聚合落盘事件");
    // reasoning 流（池卡 A 线要求）：先于对应 AssistantMessage 且文本透传
    assert!(kinds.contains(&"reasoning-delta"), "应外露思考增量: {kinds:?}");
    let first_reasoning = kinds.iter().position(|k| *k == "reasoning-delta").unwrap();
    assert!(first_reasoning < first_message, "思考增量应先于聚合落盘事件（先思考后输出）");
    assert!(
        events.iter().any(
            |e| matches!(e, TurnEvent::ReasoningDelta { text } if text == "先想好用哪个命令。")
        ),
        "思考文本应透传"
    );

    for event in &events {
        if let TurnEvent::ToolResult { name, exit_code, is_error, summary, content, .. } = event {
            assert_eq!(name, "bash");
            assert!(!is_error);
            assert_eq!(*exit_code, Some(0), "bash 工具应携带退出码");
            assert!(summary.contains("hello-from-tool"), "摘要含工具输出: {summary}");
            // 详情数据面：canonical value 原样，Bash 详情三件套齐备
            assert_eq!(content["exit_code"], 0, "详情块: {content}");
            assert!(content["output"].as_str().unwrap().contains("hello-from-tool"));
        }
        if let TurnEvent::ToolCall { name, arguments_summary, .. } = event {
            assert_eq!(name, "bash");
            assert!(arguments_summary.contains("hello-from-tool"));
        }
        if let TurnEvent::TurnComplete { outcome: done, prompt_tokens, completion_tokens, .. } = event {
            assert_eq!(done.tool_calls, 1);
            assert_eq!(done.assistant_text, outcome.assistant_text, "完成事件与返回值同源");
            // usage 多步累计（10+7 / 5+3）
            assert_eq!(*prompt_tokens, Some(17));
            assert_eq!(*completion_tokens, Some(8));
        }
    }

    // 向后兼容：无观察者形态照常工作
    let outcome_plain = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .unwrap()
        .run_turn("再来一句", tokio_util::sync::CancellationToken::new())
        .await
        .expect("兼容形态应正常");
    assert!(!outcome_plain.assistant_text.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// usage 语义（云核口径）：任一步端点未报 → TurnComplete 的 token 为 None（不部分估算）。
/// 数据面：脚本第二轮（收尾回答）的 Done.usage 置 None，第一轮有值——整体退化为 None。
#[tokio::test(flavor = "multi_thread")]
async fn usage任一步缺失则整体为空() {
    let dir = std::env::temp_dir().join(format!("dsh-loop-usage-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = yunyou::Context::new();
    wait_active(
        &root.plugin(yunyou_session::SessionPlugin, serde_json::json!({ "path": dir.join("s.jsonl").to_string_lossy() })),
        "session",
    )
    .await;
    ensure_commands(&root).await;
    wait_active(
        &root.plugin(yunyou_tools::ToolsPlugin, serde_json::json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    // 第一轮带 usage；第二轮（收尾回答）不带
    let mut scripts = tool_then_answer_script();
    if let Some(last) = scripts.last_mut() {
        if let Some(StreamDelta::Done { usage, .. }) = last.last_mut() {
            *usage = None;
        }
    }
    wait_active(
        &root.plugin(
            FakeLlmPlugin { llm: Arc::new(FakeLlm::new(scripts)) },
            serde_json::json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "system-prompt").await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;

    let collector = CollectingObserver::new();
    let outcome = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见")
        .run_turn_observed("跑命令", tokio_util::sync::CancellationToken::new(), collector.clone())
        .await
        .expect("本轮应成功");
    assert_eq!(outcome.tool_calls, 1);
    let completion = collector.snapshot().into_iter().last().unwrap();
    match completion {
        TurnEvent::TurnComplete { prompt_tokens, completion_tokens, .. } => {
            assert_eq!(prompt_tokens, None, "任一步缺失 → 整体 None（云核口径）");
            assert_eq!(completion_tokens, None);
        }
        other => panic!("末事件应为 TurnComplete: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
