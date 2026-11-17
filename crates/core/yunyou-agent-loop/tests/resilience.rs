//! 续跑健壮性批集成测试（五席评审 20261117 L-3/L-9/L-10）：
//! 畸形 arguments 自锁、悬空 ToolCall 恢复兜底、finish_reason=length 截断标记。

mod common;

use common::{ensure_commands, wait_active, FakeStream};
use std::sync::{Arc, Mutex};
use yunyou::{Context, ServiceId};
use yunyou_agent_loop::{AgentLoop, AgentLoopPlugin};
use yunyou_llm::{GenerateRequest, Llm, LlmStream, StreamDelta, ToolCallDelta, Usage};
use yunyou_session::{SessionEvent, SessionPlugin, SessionStore};
use yunyou_tools::ToolsPlugin;
use yunyou_util::BoxFut;

/// 捕获请求的脚本 LLM：记录每次收到的 GenerateRequest（L-10 回发面断言用），
/// 按脚本队列吐增量（耗尽回退空收尾）。
struct RecordingLlm {
    scripts: Mutex<Vec<Vec<StreamDelta>>>,
    pub requests: Mutex<Vec<GenerateRequest>>,
}

impl RecordingLlm {
    fn new(scripts: Vec<Vec<StreamDelta>>) -> Arc<Self> {
        Arc::new(Self { scripts: Mutex::new(scripts), requests: Mutex::new(Vec::new()) })
    }
}

impl Llm for RecordingLlm {
    fn stream(&self, req: GenerateRequest) -> BoxFut<'static, yunyou::Result<Box<dyn LlmStream>>> {
        self.requests.lock().unwrap().push(req.clone());
        let script = {
            let mut queue = self.scripts.lock().unwrap();
            if queue.is_empty() {
                vec![
                    StreamDelta::Text("(脚本已耗尽)".into()),
                    StreamDelta::Done { finish_reason: Some("stop".into()), usage: None },
                ]
            } else {
                queue.remove(0)
            }
        };
        Box::pin(async move { Ok(Box::new(FakeStream::new(script)) as Box<dyn LlmStream>) })
    }
}

/// 标准装配（session+tools+llm+agent-loop；prompt 服务同 tests/turn.rs 口径）。
async fn assemble(dir: &std::path::Path, llm: Arc<dyn Llm>) -> Context {
    let root = Context::new();
    let session_path = dir.join("session.jsonl");
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
        &root.plugin(common::FakeLlmPlugin { llm }, serde_json::json!({})),
        "fake-llm",
    )
    .await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})),
        "system-prompt",
    )
    .await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;
    root
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ya-resilience-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn done(reason: &str) -> StreamDelta {
    StreamDelta::Done {
        finish_reason: Some(reason.into()),
        usage: Some(Usage { prompt_tokens: 5, completion_tokens: 3 }),
    }
}

/// L-10：畸形 arguments——执行侧落人话错误 ToolResult（不再静默 {} 兜底），
/// 回发侧归一为 "{}"（下轮请求不再携带非法 JSON，端点不会永久 400）。
#[tokio::test(flavor = "multi_thread")]
async fn 畸形参数落错误回执且回发归一() {
    let dir = tmp_dir("bad-args");
    let llm = RecordingLlm::new(vec![
        vec![
            StreamDelta::ToolCall(ToolCallDelta {
                index: 0,
                id: Some("call-bad".into()),
                name: Some("bash".into()),
                arguments_delta: "{\"cmd\":".into(), // 被 length 截断的非法 JSON
            }),
            done("tool_calls"),
        ],
        vec![StreamDelta::Text("参数修好了。".into()), done("stop")],
    ]);
    let root = assemble(&dir, llm.clone()).await;
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let cancel = tokio_util::sync::CancellationToken::new();
    loop_svc
        .run_turn("跑个命令", cancel)
        .await
        .expect("本轮应成功（畸形参数走错误回执，不炸轮）");

    // 落盘面：ToolResult is_error 且人话指路
    let store = root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()).unwrap();
    let events = store.load_all().await.unwrap();
    let bad_result = events
        .iter()
        .find_map(|e| match e {
            SessionEvent::ToolResult { call_id, is_error, content, .. } if call_id == "call-bad" => {
                Some((*is_error, content.clone()))
            }
            _ => None,
        })
        .expect("畸形参数调用应有 ToolResult");
    assert!(bad_result.0, "畸形参数必须 is_error");
    assert!(bad_result.1.contains("合法 JSON"), "错误回执应人话指路: {}", bad_result.1);

    // 回发面：第二轮请求里该调用的 arguments 已归一 "{}"（不再原样回发非法 JSON）
    let requests = llm.requests.lock().unwrap();
    assert!(requests.len() >= 2, "应有两轮请求: {}", requests.len());
    let resent = &requests[1];
    let bad_call_args = resent
        .messages
        .iter()
        .flat_map(|m| &m.tool_calls)
        .find(|c| c.id == "call-bad")
        .expect("第二轮请求应携带原调用声明");
    assert_eq!(bad_call_args.arguments, "{}", "回发 arguments 必须归一为空对象");
    let _ = std::fs::remove_dir_all(&dir);
}

/// L-3：会话残留悬空 ToolCall（ToolResult 落盘失败/硬杀）→ 回合起点恢复
/// 扫描补合成 is_error ToolResult，下一轮请求配对完整不再被端点拒收。
#[tokio::test(flavor = "multi_thread")]
async fn 悬空工具调用恢复补配对() {
    let dir = tmp_dir("dangling");
    let llm = Arc::new(common::FakeLlm::new(vec![
        vec![StreamDelta::Text("续跑完成。".into()), done("stop")],
    ]));
    let root = assemble(&dir, llm).await;

    // 预埋悬空态：只落 ToolCall 不落 ToolResult（模拟落盘失败/进程被杀残留）
    {
        let store = root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()).unwrap();
        store
            .append(&SessionEvent::ToolCall {
                call_id: "call-orphan".into(),
                name: "bash".into(),
                arguments: "{\"cmd\":\"echo hi\"}".into(),
                parent_tool_call_id: None,
                batch_id: None,
            })
            .await
            .unwrap();
    }

    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let cancel = tokio_util::sync::CancellationToken::new();
    loop_svc.run_turn("继续", cancel).await.expect("续跑轮应成功");

    let store = root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()).unwrap();
    let events = store.load_all().await.unwrap();
    let healed = events
        .iter()
        .find_map(|e| match e {
            SessionEvent::ToolResult { call_id, is_error, content, .. } if call_id == "call-orphan" => {
                Some((*is_error, content.clone()))
            }
            _ => None,
        })
        .expect("悬空调用必须被合成 ToolResult 配对");
    assert!(healed.0, "合成件必须 is_error");
    assert!(healed.1.contains("中断"), "合成件应说明中断: {}", healed.1);
    // 配对完整性：序列里 ToolCall 与 ToolResult 一一对应（不再有悬空）
    let mut open = 0usize;
    for e in &events {
        match e {
            SessionEvent::ToolCall { .. } => open += 1,
            SessionEvent::ToolResult { .. } => open = open.saturating_sub(1),
            _ => {}
        }
    }
    assert_eq!(open, 0, "恢复扫描后不应再有悬空 ToolCall");
    let _ = std::fs::remove_dir_all(&dir);
}

/// L-9：finish_reason=length——半截回复不再当完整回复：落截断 SystemMessage
/// 注记 + TurnOutcome.truncated=true（诚实性标记）；stop 收尾不受影响。
#[tokio::test(flavor = "multi_thread")]
async fn 输出截断落诚实标记() {
    let dir = tmp_dir("length");
    let llm = Arc::new(common::FakeLlm::new(vec![
        vec![StreamDelta::Text("说到一半就被掐断".into()), done("length")],
    ]));
    let root = assemble(&dir, llm).await;
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome = loop_svc.run_turn("讲讲架构", cancel).await.expect("本轮应成功");
    assert!(outcome.truncated, "length 收尾必须置 truncated=true");

    let store = root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()).unwrap();
    let events = store.load_all().await.unwrap();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::SystemMessage { text } if text.contains("截断"))),
        "length 收尾应落截断注记 SystemMessage: {events:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// L-9 回归面：stop 正常收尾不落截断注记、truncated 保持 false。
#[tokio::test(flavor = "multi_thread")]
async fn 正常收尾无截断标记() {
    let dir = tmp_dir("stop");
    let llm = Arc::new(common::FakeLlm::new(vec![
        vec![StreamDelta::Text("说完了。".into()), done("stop")],
    ]));
    let root = assemble(&dir, llm).await;
    let loop_svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome = loop_svc.run_turn("你好", cancel).await.expect("本轮应成功");
    assert!(!outcome.truncated, "stop 收尾不得误标截断");
    let store = root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()).unwrap();
    let events = store.load_all().await.unwrap();
    assert!(
        !events.iter().any(|e| matches!(e, SessionEvent::SystemMessage { text } if text.contains("截断"))),
        "stop 收尾不得落截断注记"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
