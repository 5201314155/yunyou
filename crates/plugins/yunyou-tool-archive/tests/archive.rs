//! 工具结果剥离策略插件集成测试（task-11②，lead 验收四条+对比快照锚）：
//! 水位下降/重读可回/单轮一刀/stub 稳定幂等 + 未启用对照（请求体含原文）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use yunyou::{Context, ServiceId};
use yunyou_agent_loop::{AgentLoop, AgentLoopPlugin, HooksRegistryService};
use yunyou_llm::{GenerateRequest, Llm, LlmStream, StreamDelta, ToolCallDelta, Usage};
use yunyou_session::{SessionPlugin, SessionStore};
use yunyou_tool_archive::ToolArchivePlugin;
use yunyou_tools::ToolsPlugin;
use yunyou_util::BoxFut;

/// 捕获请求的脚本 LLM：记录每轮收到的请求（剥离生效判别锚），按脚本吐增量。
struct RecordingLlm {
    scripts: Mutex<Vec<Vec<StreamDelta>>>,
    requests: Mutex<Vec<GenerateRequest>>,
}

impl RecordingLlm {
    fn new(scripts: Vec<Vec<StreamDelta>>) -> Arc<Self> {
        Arc::new(Self { scripts: Mutex::new(scripts), requests: Mutex::new(Vec::new()) })
    }
}

struct FakeStream {
    script: Vec<StreamDelta>,
    index: usize,
}

impl LlmStream for FakeStream {
    fn next(&mut self) -> BoxFut<'_, yunyou::Result<Option<StreamDelta>>> {
        Box::pin(async move {
            let index = self.index;
            self.index += 1;
            Ok(self.script.get(index).cloned())
        })
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
                    done("stop"),
                ]
            } else {
                queue.remove(0)
            }
        };
        Box::pin(async move { Ok(Box::new(FakeStream { script, index: 0 }) as Box<dyn LlmStream>) })
    }
}

fn done(reason: &str) -> StreamDelta {
    StreamDelta::Done {
        finish_reason: Some(reason.into()),
        usage: Some(Usage { prompt_tokens: 5, completion_tokens: 3 }),
    }
}

fn tool_call(id: &str, output: &str) -> Vec<StreamDelta> {
    vec![
        StreamDelta::ToolCall(ToolCallDelta {
            index: 0,
            id: Some(id.into()),
            name: Some("bash".into()),
            arguments_delta: format!("{{\"cmd\":\"echo {output}\"}}"),
        }),
        done("tool_calls"),
    ]
}

fn text_round(text: &str) -> Vec<StreamDelta> {
    vec![StreamDelta::Text(text.into()), done("stop")]
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// 脚本 LLM 注入插件（与真实 Provider 同一装配路径；本测试文件自足）。
struct LlmShimPlugin {
    llm: Arc<dyn Llm>,
}

impl yunyou::Plugin for LlmShimPlugin {
    fn name(&self) -> &str {
        "llm-shim"
    }
    fn setup(
        self: Arc<Self>,
        ctx: Context,
        _config: serde_json::Value,
    ) -> BoxFut<'static, yunyou::Result<yunyou::FiberOutput>> {
        Box::pin(async move {
            ctx.provide(ServiceId::of::<dyn Llm>(), self.llm.clone())?;
            Ok(yunyou::FiberOutput)
        })
    }
}

/// 装配（session+tools+llm+agent-loop；archive 插件按参启停）。
async fn assemble(dir: &PathBuf, llm: Arc<dyn Llm>, archive: bool) -> Result<Context, yunyou::CordisError> {
    let root = Context::new();
    let session_path = dir.join("session.jsonl");
    // tools inject dyn CommandsService（/tools-info 依赖）——先装 commands 免 Pending
    wait_active(
        &root.plugin(yunyou_commands::CommandsPlugin, serde_json::json!({})),
        "commands",
    )
    .await;
    wait_active(
        &root.plugin(SessionPlugin, serde_json::json!({ "path": session_path.to_string_lossy() })),
        "session",
    )
    .await;
    wait_active(
        &root.plugin(ToolsPlugin, serde_json::json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})),
        "system-prompt",
    )
    .await;
    wait_active(
        &root.plugin(LlmShimPlugin { llm }, serde_json::json!({})),
        "llm",
    )
    .await;
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;
    if archive {
        wait_active(&root.plugin(ToolArchivePlugin, serde_json::json!({})), "tool-archive").await;
    }
    Ok(root)
}

async fn wait_active(handle: &yunyou::FiberHandle, name: &str) {
    for _ in 0..400 {
        if handle.state() == yunyou::FiberState::Active {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("插件 {name} 未能在限时内装载到 Active，当前 {:?}", handle.state());
}

/// 七轮长会话脚本：三次工具调用各自被 assistant 回应消费（出新鲜窗），
/// 第 7 轮（纯文本）组装时 c1 出窗成唯一一刀。
fn long_script() -> Vec<Vec<StreamDelta>> {
    vec![
        tool_call("c1", "AAA_OUTPUT"),
        text_round("收到A"),
        tool_call("c2", "BBB_OUTPUT"),
        text_round("收到B"),
        tool_call("c3", "CCC_OUTPUT"),
        text_round("收到C"),
        text_round("总结"),
        text_round("再总结"),
        // 第 9/10 轮：新增 c4 后 c2 出窗（stub 不占新鲜窗位），第 10 轮剥 c2
        tool_call("c4", "DDD_OUTPUT"),
        text_round("收到D"),
    ]
}

/// 主验收：水位下降+重读可回+单轮一刀+stub 稳定幂等。
#[tokio::test(flavor = "multi_thread")]
async fn 剥离生效四验收() -> TestResult {
    let dir = std::env::temp_dir().join(format!("ya-archive-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let llm = RecordingLlm::new(long_script());
    let root = assemble(&dir, llm.clone(), true).await?;
    let loop_svc = root.get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())?;
    let cancel = tokio_util::sync::CancellationToken::new();

    // 跑满十轮（第 7 轮剥 c1、第 10 轮剥 c2）
    for _ in 0..10 {
        loop_svc.run_turn("继续", cancel.clone()).await?;
    }

    // 取第 n 轮请求中 call_id 对应 tool 结果的正文（剥离面=tool 消息 content；
    // assistant 调用声明 arguments 里本就含命令文本，不属于剥离面，不扫全包）
    let tool_content = |n: usize, call_id: &str| {
        let requests = llm.requests.lock().unwrap();
        requests[n]
            .messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some(call_id))
            .map(|m| m.content.clone())
            .unwrap_or_else(|| panic!("第 {n} 轮请求应有 {call_id} 的 tool 结果"))
    };

    // ①水位下降：c1 出窗成刀后（最早在第 6 轮请求生效），其结果正文=存档 stub
    let round7_c1 = tool_content(5, "c1");
    assert!(!round7_c1.contains("AAA_OUTPUT"), "c1 原文应被剥离: {round7_c1}");
    assert!(round7_c1.contains("tool_results/c1.txt"), "应含存档指路: {round7_c1}");
    assert!(tool_content(6, "c2").contains("BBB_OUTPUT"), "窗内 c2 不剥");
    // ②重读可回：side-store 原文逐字节一致
    // 存档原文=剥离瞬间的结果正文（bash canonical JSON 形态），逐字节一致
    let saved = std::fs::read_to_string(dir.join("tool_results/c1.txt"))?;
    assert_eq!(saved, tool_content(2, "c1"), "存档原文无损（剥离前最后原样轮对照）");
    // ③单轮一刀+stub 稳定幂等：第 8 轮只剥 c2，c1 stub 原位未动
    loop_svc.run_turn("继续", cancel.clone()).await?;
    let stub_c1 = {
        let requests = llm.requests.lock().unwrap();
        let r7 = &requests[6];
        r7.messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c1"))
            .map(|m| m.content.clone())
            .unwrap()
    };
    let r8_c1 = {
        let requests = llm.requests.lock().unwrap();
        requests[7]
            .messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c1"))
            .map(|m| m.content.clone())
            .unwrap()
    };
    assert_eq!(stub_c1, r8_c1, "c1 stub 逐字节稳定（幂等不叠加）");
    assert!(
        tool_content(7, "c2").contains("BBB_OUTPUT"),
        "第 8 轮无刀：c2/c3 均在新鲜窗内（每轮最多一刀的节奏=新结果解锁旧刀）"
    );
    // 第 10 轮：c4 入列后 c2 出窗成刀
    let round10_c2 = tool_content(9, "c2");
    assert!(!round10_c2.contains("BBB_OUTPUT"), "第 10 轮 c2 成刀被剥: {round10_c2}");
    assert!(round10_c2.contains("tool_results/c2.txt"));
    assert!(tool_content(9, "c3").contains("CCC_OUTPUT"), "c3 仍在窗内不剥");
    assert!(tool_content(9, "c4").contains("DDD_OUTPUT"), "c4 最新不剥");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// 对比快照锚（lead 加验）：未启用剥离时同脚本的请求体含原文（判别基线）。
#[tokio::test(flavor = "multi_thread")]
async fn 未启用对照请求体含原文() -> TestResult {
    let dir = std::env::temp_dir().join(format!("ya-archive-base-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let llm = RecordingLlm::new(long_script());
    let root = assemble(&dir, llm.clone(), false).await?;
    let loop_svc = root.get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())?;
    let cancel = tokio_util::sync::CancellationToken::new();
    for _ in 0..7 {
        loop_svc.run_turn("继续", cancel.clone()).await?;
    }
    let requests = llm.requests.lock().unwrap();
    let round7 = serde_json::to_string(&requests[6].messages)?;
    assert!(round7.contains("AAA_OUTPUT"), "未启用=原文全量回传: {round7}");
    assert!(!round7.contains("tool_results/"), "未启用=零存档标记");
    assert!(
        root.get::<dyn HooksRegistryService>(ServiceId::of::<dyn HooksRegistryService>())
            .map(|h| h.list().is_empty())
            .unwrap_or(true),
        "未启用=钩子注册表应空（空注册行为不变锚）"
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
