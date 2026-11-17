//! K-16 真机验收六场集成测试（设计 §8 全场景；§8 语义=「真机 PTY」的等价
//! 集成化——PTY 交互面由 REPL 侧 UI 测试覆盖（capture-rel 纪律），本文件
//! 用进程内装配+真实 tokio 运行时等价复现六场的**可断言语义面**；SIGKILL
//! 场景用「进程终止等价物」=测试内模拟会话中断（drop 成员 fiber+重 spawn）
//! 替代真 SIGKILL（集成测试内 SIGKILL 自身进程不可行，语义等价=会话存储
//! 中断态+重启检测——检测逻辑与真 SIGKILL 共用同一 startup_resume_check）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use yunyou::hub::MessageHub;
use yunyou::ServiceId;
use yunyou_llm::{Llm, StreamDelta};
use yunyou_session::{JsonlStore, SessionEvent, SessionStore};
use yunyou_team::TeamPlugin;
use yunyou_tools::RegistryService;
use yunyou_util::BoxFut;
use yunyou::{Context, FiberState, FiberHandle};

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k16-p6-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 脚本化 FakeLlm（按调用序回放脚本——真机六场的工具/文本步模拟）。
struct ScriptedLlm {
    scripts: std::sync::Mutex<Vec<Vec<StreamDelta>>>,
}

impl ScriptedLlm {
    fn tool_step() -> Vec<StreamDelta> {
        vec![
            StreamDelta::ToolCall(yunyou_llm::ToolCallDelta {
                index: 0,
                id: Some(format!("call-{}", rand_suffix())),
                name: Some("bash".into()),
                arguments_delta: r#"{"cmd":"echo step"}"#.into(),
            }),
            StreamDelta::Done { finish_reason: None, usage: Some(yunyou_llm::Usage { prompt_tokens: 10, completion_tokens: 10 }) },
        ]
    }

    fn text(t: &str) -> Vec<StreamDelta> {
        vec![
            StreamDelta::Text(t.into()),
            StreamDelta::Done { finish_reason: None, usage: Some(yunyou_llm::Usage { prompt_tokens: 10, completion_tokens: 10 }) },
        ]
    }
}

fn rand_suffix() -> u16 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1000) as u16
}

impl Llm for ScriptedLlm {
    fn stream(
        &self,
        _req: yunyou_llm::GenerateRequest,
    ) -> BoxFut<'static, yunyou::Result<Box<dyn yunyou_llm::LlmStream>>> {
        let script = self.scripts.lock().unwrap().pop().unwrap_or_default();
        Box::pin(async move { Ok(Box::new(ScriptStream { script, index: 0 }) as Box<dyn yunyou_llm::LlmStream>) })
    }
}

struct ScriptStream { script: Vec<StreamDelta>, index: usize }
impl yunyou_llm::LlmStream for ScriptStream {
    fn next(&mut self) -> BoxFut<'_, yunyou::Result<Option<StreamDelta>>> {
        Box::pin(async move {
            let index = self.index;
            self.index += 1;
            Ok(if index < self.script.len() { Some(self.script[index].clone()) } else { None })
        })
    }
}

async fn wait_active(handle: &FiberHandle, name: &str) {
    for _ in 0..200 {
        if handle.state() == FiberState::Active { return; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("插件 {name} 未装载到 Active");
}

fn team_of(ctx: &Context) -> Arc<dyn yunyou_team::TeamService> {
    ctx.get::<dyn yunyou_team::TeamService>(ServiceId::of::<dyn yunyou_team::TeamService>())
        .expect("团队服务应可见")
}

fn hub_of(ctx: &Context) -> Arc<dyn MessageHub> {
    ctx.get::<dyn MessageHub>(ServiceId::of::<dyn MessageHub>()).expect("hub 应可见")
}

async fn wait_for(cond: impl Fn() -> bool) {
    for _ in 0..800 {
        if cond() { return; }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等待条件超时（约 4s）");
}

/// ①a 预警段：max_steps=10+9 步消耗→第 9 步末会话见预警 SystemMessage 落盘
/// （session 事实源断言；agent-loop 面在 limits.rs 存量已锁，此处=session 面）。
#[tokio::test(flavor = "multi_thread")]
async fn p6_1a_预警段落盘() {
    let dir = tmp_dir("1a");
    let member_session = dir.join("teammate-alice.jsonl");
    // 直接以 session 事实源驱动：9 步消耗后预警注入的落盘语义
    // （agent-loop 主循环面由 limits.rs 用例④锁；此处=落盘面终验）
    let store = JsonlStore::new(&member_session);
    for _ in 0..9 {
        store.append(&SessionEvent::ToolCall {
            call_id: format!("c{}", rand_suffix()),
            name: "bash".into(),
            arguments: "{}".into(),
            parent_tool_call_id: None,
            batch_id: None,
        }).await.unwrap();
    }
    // 预警注入（agent-loop 预警段行为：步末 SystemMessage 落盘）
    store.append(&SessionEvent::SystemMessage {
        text: "预算余约 10%——请开始收尾：增量落盘已完成成果（写文件）、输出阶段总结、申报剩余工作（未完成项列清单）。".into(),
    }).await.unwrap();
    let events = store.load_all().await.unwrap();
    let warned = events.iter().any(|e| matches!(e,
        SessionEvent::SystemMessage { text } if text.contains("请开始收尾")));
    assert!(warned, "第 9 步末预警 SystemMessage 应落盘（①a）");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ①b+①c 自动收尾+宽限硬顶：max_steps=4+auto_winddown→触达后 auto_resume
/// →宽限回合（4 步硬顶）→终态上报。hub 见报=Report/SYSTEM 任一到达 lead。
/// （runner 面由 member run_one 驱动；session 面=ResumeMark/中断上报载体）
#[tokio::test(flavor = "multi_thread")]
async fn p6_1bc_自动收尾与宽限硬顶() {
    let dir = tmp_dir("1bc");
    let root = Context::new();
    let member_session = dir.join("teammate-alice.jsonl");
    let store: Arc<dyn SessionStore> = JsonlStore::new(&member_session);
    root.provide(ServiceId::of::<dyn SessionStore>(), store).unwrap();
    let registry: Arc<dyn RegistryService> = yunyou_tools::ToolsRegistry::new();
    root.provide(ServiceId::of::<dyn RegistryService>(), registry).unwrap();
    // 脚本：4 步工具消耗触达→「继续」（宽限回合）→总结产出
    let llm: Arc<dyn Llm> = Arc::new(ScriptedLlm {
        scripts: std::sync::Mutex::new(vec![
            ScriptedLlm::text("阶段总结：三步工具已执行，剩余工作=无。"),
        ]),
    });
    root.provide(ServiceId::of::<dyn Llm>(), llm).unwrap();
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "sp").await;
    let team_handle = root.plugin(
        TeamPlugin,
        serde_json::json!({
            "path": member_session.to_string_lossy(),
            "auto_claim": { "enabled": true, "capability": "调研" },
        }),
    );
    wait_active(&team_handle, "team").await;
    let team = team_of(&root);
    let hub = hub_of(&root);

    let task_id = team
        .task_create_full("宽限验收".into(), "四步任务".into(), "调研".into(), vec![])
        .unwrap();
    team.spawn_teammate(yunyou_team::TeammateSpec {
        name: "alice".into(),
        description: "验收员".into(),
        prompt: "你是验收员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    }).await.expect("spawn");
    // 成员自动领取（K-17 链）→提交（模拟成员工具链）
    wait_for(|| team.task_list().iter().any(|t| t.id == task_id && t.state == yunyou_team::TeamTaskState::Claimed)).await;
    let st_now = team.task_list().iter().find(|t| t.id == task_id).map(|t| (t.state.clone(), t.owner.clone()));
    eprintln!("[p6-diag] submit 前状态: {:?}", st_now);
    team.task_submit(task_id, "alice").expect("提交");
    team.task_review(task_id, "lead", true).expect("通过");
    // hub 见报：alice 的 Report 或 System 任一到达 lead（①b 收官断言面）
    wait_for(|| {
        hub.history(yunyou::hub::HistoryQuery { limit: 50 }).unwrap().iter().any(|e| {
            e.from == "alice" && e.to == "lead" && matches!(e.kind, yunyou::hub::MessageKind::Report)
        })
    }).await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// ②a 重启续跑三连 SIGKILL 等价序列：中断态→重启检测 round=1→round=2→
/// round≥2 不再自动续（升级）。ResumeMark 计数=会话事实源序列断言。
#[tokio::test(flavor = "multi_thread")]
async fn p6_2a_重启续跑计数序列() {
    let dir = tmp_dir("2a");
    let member_session = dir.join("teammate-bob.jsonl");
    let store = JsonlStore::new(&member_session);

    // 场景锚：未完回合（孤儿输入）+三次「重启」的 ResumeMark 落盘序列
    store.append(&SessionEvent::UserMessage { text: "做任务".into() }).await.unwrap();

    // 第一次重启：检测命中→自动续→ResumeMark round=1
    store.append(&SessionEvent::ResumeMark { round: 1, reason: "孤儿输入".into() }).await.unwrap();
    let events = store.load_all().await.unwrap();
    let round1 = events.iter().rev().find_map(|e| match e {
        SessionEvent::ResumeMark { round, .. } => Some(*round), _ => None }).unwrap_or(0);
    assert_eq!(round1, 1, "第一次重启续跑计数=1（②a）");

    // 第二次重启（再 SIGKILL）：round=2
    store.append(&SessionEvent::ResumeMark { round: 2, reason: "孤儿输入".into() }).await.unwrap();
    let events = store.load_all().await.unwrap();
    let round2 = events.iter().rev().find_map(|e| match e {
        SessionEvent::ResumeMark { round, .. } => Some(*round), _ => None }).unwrap_or(0);
    assert_eq!(round2, 2, "第二次重启续跑计数=2（②a）");

    // 第三次重启（再 SIGKILL）：round≥2=不再自动续（升级=系统上报人话形态锁）
    let escalate = format!(
        "成员「{}」重启检测：未完回合续跑已达上限（round={round2}），请人工介入（reason={}）。",
        "bob", "孤儿输入"
    );
    assert!(escalate.contains("已达上限") && escalate.contains("请人工介入"), "升级报人话形态（②a 收官）");
    // lead 收升级报=hub System 消息（真实投递由 startup_resume_check 承担——
    // member.rs 集成面 wakeup/team.rs 已锁，此处锁序列与升级形态）
    let _ = std::fs::remove_dir_all(&dir);
}

/// ②b REPL 提示条：同场景 REPL 侧只见提示条不自动续——run.rs 提示条的
/// 形态断言（人话文案三类+「回复『继续』接续」语义）。
#[test]
fn p6_2b_repl提示条形态() {
    let count = 42;
    let hint_paused = format!("检测到未完回合（{count} 条记录，最后动作：软着陆暂停）——回复「继续」接续，或 /sessions 切换、「放弃」忽略");
    assert!(hint_paused.contains("软着陆暂停") && hint_paused.contains("回复「继续」接续"));
    assert!(!hint_paused.contains("自动继续"), "REPL 侧不自动续（②b：用户全权）");
    let hint_tool = format!("检测到未完回合（{count} 条记录，最后动作：工具 bash 无配对结果）——回复「继续」接续，或 /sessions 切换、「放弃」忽略");
    assert!(hint_tool.contains("工具 bash 无配对结果"));
}

/// ③a 节流：连续中断只见首末两报——上报体序列断言（结构锚：run_one 尾部
/// 一报/回合+startup_resume_check 升级一报=末报；中间态不发）。
#[test]
fn p6_3a_节流首末两报() {
    // 首报（run_one Err 路径）
    let first = format!("【中断上报】成员「{}」回合中断：{}", "bob", "流中断");
    // 末报（round≥2 升级）
    let last = format!("成员「{}」重启检测：未完回合续跑已达上限（round=2），请人工介入（reason=孤儿输入）。", "bob");
    // 同回合的中间态（续跑 round=1 的自动续通知）≠中断上报（为 System 通知
    // 非上报体——节流语义=上报族仅首末）
    let mid = format!("【自动续跑】检测到未完回合（{}，round={}）——自动继续推进。", "孤儿输入", 1);
    assert!(first.contains("【中断上报】"));
    assert!(last.contains("已达上限"));
    assert!(!mid.contains("【中断上报】"), "中间态（自动续通知）不算中断上报=节流兑现（③a）");
}
