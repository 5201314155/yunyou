//! 团队插件（K-7）集成测试矩阵：spawn 上线/完工 Report/成员互联（双身份派发）/
//! kick 续投（覆盖 spawn 窗口缓冲语义）/任务板四操作越权拒/成员工具面裁剪。
//!
//! 装配最小化：lead 会话=临时目录 JsonlStore、注册表=空壳 ToolsRegistry（成员
//! 轮无需内置工具）、LLM=脚本化 Fake（不烧真 token）、SystemPrompt=真插件。
//! 断言面以 hub history（真账）与成员会话落盘为准，不做时序硬假设（轮询等待）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use yunyou::hub::{Address, HistoryQuery, MessageHub, MessageKind};
use yunyou_session::SessionStore;
use yunyou_tools::RegistryService;
use yunyou::{Context, Result, ServiceId};
use yunyou_llm::{GenerateRequest, Llm, LlmStream, StreamDelta, ToolCallDelta, Usage};
use yunyou_session::JsonlStore;
use yunyou_team::{TeamPlugin, TeamService, TeammateSpec};
use yunyou_util::BoxFut;

// ── 脚本化 Fake LLM（与 agent-loop tests/common 同构的最小裁剪） ──

#[derive(Clone)]
struct FakeLlm {
    scripts: Arc<std::sync::Mutex<Vec<Vec<StreamDelta>>>>,
}

impl FakeLlm {
    /// 便捷脚本：单段纯文本回复。
    fn answer(text: &str) -> Vec<StreamDelta> {
        vec![
            StreamDelta::Text(text.to_string()),
            StreamDelta::Done { finish_reason: Some("stop".into()), usage: None },
        ]
    }

    /// 便捷脚本：先调一次工具再文本收尾（成员互联用——工具调用是模型面行为，
    /// send_message 的 from 由 hub 身份派发非脚本可控=防伪造断言面）。
    fn tool_then_answer(tool: &str, args: &str, answer: &str) -> Vec<StreamDelta> {
        let mut script = vec![
            StreamDelta::ToolCall(ToolCallDelta {
                index: 0,
                id: Some("call-1".into()),
                name: Some(tool.to_string()),
                arguments_delta: args.to_string(),
            }),
            StreamDelta::Done {
                finish_reason: Some("tool_calls".into()),
                usage: Some(Usage { prompt_tokens: 1, completion_tokens: 1 }),
            },
        ];
        script.extend(Self::answer(answer));
        script
    }
}

impl Llm for FakeLlm {
    fn stream(&self, _req: GenerateRequest) -> BoxFut<'static, Result<Box<dyn LlmStream>>> {
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
        Box::pin(async move { Ok(Box::new(FakeStream { script, index: 0 }) as Box<dyn LlmStream>) })
    }
}

struct FakeStream {
    script: Vec<StreamDelta>,
    index: usize,
}

impl LlmStream for FakeStream {
    fn next(&mut self) -> BoxFut<'_, Result<Option<StreamDelta>>> {
        Box::pin(async move {
            let index = self.index;
            self.index += 1;
            Ok(if index < self.script.len() { Some(self.script[index].clone()) } else { None })
        })
    }
}

// ── 装配与断言 helper ──

async fn assemble(llm: FakeLlm, session_dir: &std::path::Path) -> Context {
    assemble_with(llm, session_dir, serde_json::json!({})).await
}

/// 带额外 [team] 配置键的装配（task-30③ 挂死熔断用例注入 tool_timeout_secs）。
async fn assemble_with(llm: FakeLlm, session_dir: &std::path::Path, extra: serde_json::Value) -> Context {
    let root = Context::new();
    let store: Arc<dyn SessionStore> = JsonlStore::new(session_dir.join("lead.jsonl"));
    root.provide(ServiceId::of::<dyn SessionStore>(), store).expect("provide session");
    // 显式擦除为 trait object（泛型推断会把 P 定为具体类型 ToolsRegistry，
    // 导致 get::<dyn RegistryService> 类型不符——tools plugin.rs 同款注释纪律）
    let registry: Arc<dyn RegistryService> = yunyou_tools::ToolsRegistry::new();
    root.provide(ServiceId::of::<dyn RegistryService>(), registry).expect("provide registry");
    let llm: Arc<dyn Llm> = Arc::new(llm);
    root.provide(ServiceId::of::<dyn Llm>(), llm).expect("provide llm");
    wait_active(&root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({})), "system-prompt").await;
    // path 直传模拟 [session] 段值（slots merge 的装配外等价形态）
    let mut cfg = serde_json::json!({ "path": session_dir.join("lead.jsonl") });
    if let (Some(base), Some(extra)) = (cfg.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    wait_active(&root.plugin(TeamPlugin, cfg), "team").await;
    root
}

async fn wait_active(handle: &yunyou::FiberHandle, name: &str) {
    for _ in 0..400 {
        if handle.state() == yunyou::FiberState::Active {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let err = handle.await_ready().await.err().map(|e| e.to_string()).unwrap_or_default();
    panic!("插件 {name} 未能在限时内装载到 Active，当前 {:?}，装载错误={err}", handle.state());
}

/// 轮询等待（成员回合与 Report 投递是异步链；5ms×800≈4s 上限）。
async fn wait_for(cond: impl Fn() -> bool) {
    for _ in 0..800 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等待条件超时（约 4s）");
}

fn team_of(ctx: &Context) -> Arc<dyn TeamService> {
    ctx.get::<dyn TeamService>(ServiceId::of::<dyn TeamService>()).expect("团队服务应可见")
}

fn hub_of(ctx: &Context) -> Arc<dyn MessageHub> {
    ctx.get::<dyn MessageHub>(ServiceId::of::<dyn MessageHub>()).expect("hub 缝应可见")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dsh-team-it-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 留痕过滤：from→to 且 kind 匹配的条目数。
fn count_history(hub: &dyn MessageHub, from: &str, to: &str, kind: MessageKind) -> usize {
    hub.history(HistoryQuery { limit: 1000 })
        .expect("留痕查询")
        .iter()
        .filter(|entry| entry.from == from && entry.to == to && entry.kind == kind)
        .count()
}

// ── 测试矩阵 ──

/// ①spawn 上线与名册投影：成员 fiber Active、hub 双在线（lead+成员）、名册描述。
#[tokio::test(flavor = "multi_thread")]
async fn spawn_上线与名册投影() {
    let dir = tmp_dir("spawn");
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let team = team_of(&root);
    let spawned = team
        .spawn_teammate(TeammateSpec {
            name: "alice".into(),
            description: "评审员".into(),
            prompt: "你是评审员。".into(),
            context: yunyou_team::ContextMode::Fresh,
        })
        .await
        .expect("spawn 应成功");
    assert_eq!(spawned.name, "alice");
    // hub 双在线：lead（自发成员）+alice（认领预注册信箱）
    let online: Vec<String> =
        hub_of(&root).members().iter().filter(|m| m.online).map(|m| m.name.clone()).collect();
    assert!(online.contains(&"lead".to_string()) && online.contains(&"alice".to_string()), "{online:?}");
    // 名册描述投影
    let roster = team.roster();
    let alice = roster.iter().find(|entry| entry.name == "alice").expect("alice 应在名册");
    assert_eq!(alice.description, "评审员");
    assert!(alice.online);
}

/// ②完工自动汇报：lead 发消息→成员消费→run_turn 终结→成员 fiber 自身发 Report
/// 给 lead（history 真账：from=alice→to=lead kind=report）；成员会话双事件落盘。
#[tokio::test(flavor = "multi_thread")]
async fn 完工自动汇报投递lead() {
    let dir = tmp_dir("report");
    let scripts = Arc::new(std::sync::Mutex::new(vec![FakeLlm::answer("收到，马上评审。")]));
    let root = assemble(FakeLlm { scripts }, &dir).await;
    let team = team_of(&root);
    team.spawn_teammate(TeammateSpec {
        name: "alice".into(),
        description: "评审员".into(),
        prompt: "你是评审员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");
    // lead 身份发消息（receipt 真账：即时受理非缓冲）
    let receipt = team
        .send_message(Address::Member("alice".into()), MessageKind::Text, "请评审 docs/设计".into())
        .expect("发送应成功");
    assert_eq!((receipt.recipients, receipt.buffered), (1, 0));
    // 等 Report（成员 fiber 自身发送；from 由 hub 派发非自报）
    let hub = hub_of(&root);
    wait_for(|| count_history(&*hub, "alice", "lead", MessageKind::Report) >= 1).await;
    let reports = count_history(&*hub, "alice", "lead", MessageKind::Report);
    assert_eq!(reports, 1, "恰好一条完工汇报");
    let entry = hub
        .history(HistoryQuery { limit: 100 })
        .expect("查询")
        .into_iter()
        .find(|entry| entry.from == "alice" && entry.kind == MessageKind::Report)
        .expect("Report 条目应存在");
    assert!(entry.body.contains("收到，马上评审。"), "汇报应带最终回答: {}", entry.body);
    // 成员自有会话落盘（teammate-alice.jsonl：UserMessage+AssistantMessage）
    let member_session = dir.join("teammate-alice.jsonl");
    wait_for(|| member_session.exists()).await;
    let content = std::fs::read_to_string(&member_session).expect("成员会话可读");
    assert!(content.contains("请评审 docs/设计"), "输入应落盘");
    assert!(content.contains("收到，马上评审。"), "回复应落盘");
}

/// ③成员互联（双身份派发）：alice 轮内调 send_message(to=bob)——工具参数只有
/// to/body，from=alice 由 hub 身份派发（脚本无法伪造发送者）；bob 消费后回
/// Report 给 lead。
#[tokio::test(flavor = "multi_thread")]
async fn 成员互联与身份派发() {
    let dir = tmp_dir("peer");
    let scripts = Arc::new(std::sync::Mutex::new(vec![
        FakeLlm::tool_then_answer(
            "send_message",
            r#"{"to":"bob","body":"帮我看看接口签名"}"#,
            "已经请 bob 帮忙了。",
        ),
        FakeLlm::answer("接口签名看完了，没问题。"),
    ]));
    let root = assemble(FakeLlm { scripts }, &dir).await;
    let team = team_of(&root);
    for name in ["alice", "bob"] {
        team.spawn_teammate(TeammateSpec {
            name: name.into(),
            description: "成员".into(),
            prompt: "你是成员。".into(),
            context: yunyou_team::ContextMode::Fresh,
        })
        .await
        .expect("spawn 应成功");
    }
    team.send_message(Address::Member("alice".into()), MessageKind::Text, "发起协作".into())
        .expect("发送应成功");
    let hub = hub_of(&root);
    // alice→bob 的互联消息（from=hub 派发=防伪造断言面）
    wait_for(|| count_history(&*hub, "alice", "bob", MessageKind::Text) >= 1).await;
    // bob 消费后向 lead 回 Report
    wait_for(|| count_history(&*hub, "bob", "lead", MessageKind::Report) >= 1).await;
}

/// ④kick 续投全链（覆盖信箱缓冲语义）：kick→Pending（信箱保留）→再发=缓冲
/// （receipt.buffered=1）→同名重拉（register 认领原信箱=hub 续投语义，hub 例⑧）
/// →首次消费取净全部缓冲（Report 到 lead 为证）。
#[tokio::test(flavor = "multi_thread")]
async fn kick_降级与同名续投() {
    let dir = tmp_dir("kick");
    let scripts = Arc::new(std::sync::Mutex::new(vec![FakeLlm::answer("缓冲消息已消费。")]));
    let root = assemble(FakeLlm { scripts }, &dir).await;
    let team = team_of(&root);
    let spec = || TeammateSpec {
        name: "carol".into(),
        description: "执行者".into(),
        prompt: "你是执行者。".into(),
        context: yunyou_team::ContextMode::Fresh,
    };
    team.spawn_teammate(spec()).await.expect("首拉应成功");
    // kick：fiber dispose→hub 降级 Pending（名册条目同步清）
    team.kick("carol").await.expect("kick 应成功");
    let hub = hub_of(&root);
    wait_for(|| {
        hub.members()
            .iter()
            .find(|m| m.name == "carol")
            .map(|m| !m.online)
            .unwrap_or(false)
    })
    .await;
    // Pending 名 send=缓冲（receipt 真账）
    let receipt = team
        .send_message(Address::Member("carol".into()), MessageKind::Text, "离线第一条".into())
        .expect("缓冲受理应成功");
    assert_eq!((receipt.recipients, receipt.buffered), (1, 1));
    // 同名重拉：Pending 在册=跳过 open，成员 register 认领原信箱（续投）
    team.spawn_teammate(spec()).await.expect("重拉应成功（续投路径）");
    wait_for(|| {
        hub.members()
            .iter()
            .find(|m| m.name == "carol")
            .map(|m| m.online)
            .unwrap_or(false)
    })
    .await;
    // 首次消费取净全部缓冲：Report 到 lead 为证（含 kick 期间的缓冲消息）
    wait_for(|| count_history(&*hub, "carol", "lead", MessageKind::Report) >= 1).await;
    let report = hub
        .history(HistoryQuery { limit: 100 })
        .expect("查询")
        .into_iter()
        .find(|entry| entry.from == "carol" && entry.kind == MessageKind::Report)
        .expect("Report 条目应存在");
    assert!(report.body.contains("缓冲消息已消费"), "续投消息应被消费: {}", report.body);
}

/// ⑤写审分离状态机（K-8）：create→claim→submit→review approve 全链+越权拒
/// （submit 非 owner 拒/review 提交人本人拒=SelfReview/非法迁移拒/submitted 后
/// 不可改派）。
#[tokio::test(flavor = "multi_thread")]
async fn 写审分离全链与越权拒() {
    let dir = tmp_dir("board");
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let team = team_of(&root);
    team.spawn_teammate(TeammateSpec {
        name: "alice".into(),
        description: "评审员".into(),
        prompt: "你是评审员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");
    let task_id = team.task_create("评审接口文档".into(), "验收=三条意见".into()).expect("创建应成功");
    team.task_claim(task_id, "alice").expect("认领应成功");
    // submit 非 owner=越权拒
    assert!(team.task_submit(task_id, "lead").is_err(), "非本人不可 submit");
    // submit 本人=提交
    team.task_submit(task_id, "alice").expect("本人提交应成功");
    // 重复提交=状态非法
    assert!(team.task_submit(task_id, "alice").is_err(), "已提交不可重复提交");
    // submitted 后不可改派（审查关系锁定）
    assert!(team.task_reassign(task_id, "bob").is_err(), "提交后不可改派");
    // **系统级自审拒绝（K-8 最值钱断言）**：lead 代审 alice 的活=合法（审查人≠提交人），
    // 但审查人=提交人本人时一律拒——见 ⑧ lead 自审用例
    team.task_review(task_id, "lead", true).expect("lead 审查 alice 应通过");
    // 终态后一切迁移拒
    assert!(team.task_submit(task_id, "alice").is_err(), "终态不可再提交");
    assert!(team.task_review(task_id, "lead", false).is_err(), "终态不可再审");
    let tasks = team.task_list();
    assert_eq!(tasks[0].owner.as_deref(), Some("alice"));
}

/// ⑧lead 自审拒绝（写审分离最值钱一条的直证）：lead 自己 create→claim→submit
/// 自己的活，再试图自审——系统拒绝，lead 不可绕过（板层强制非工具层自觉）。
#[tokio::test(flavor = "multi_thread")]
async fn lead自审系统拒绝() {
    let dir = tmp_dir("selfreview");
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let team = team_of(&root);
    let task_id = team.task_create("lead 自己的活".into(), "自审拒绝直证".into()).expect("创建应成功");
    team.task_claim(task_id, "lead").expect("lead 认领应成功");
    team.task_submit(task_id, "lead").expect("lead 提交应成功");
    let err = team.task_review(task_id, "lead", true).expect_err("lead 自审必须被系统拒绝");
    assert!(
        err.message().contains("写审分离拒绝") && err.message().contains("不得自审自过"),
        "自审拒绝人话: {err}"
    );
}

/// ⑨驳回路径：review reject→rejected 终态（重做=新建任务）；驳回后不可再提交。
#[tokio::test(flavor = "multi_thread")]
async fn 驳回终态与重做边界() {
    let dir = tmp_dir("reject");
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let team = team_of(&root);
    team.spawn_teammate(TeammateSpec {
        name: "alice".into(),
        description: "执行者".into(),
        prompt: "你是执行者。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");
    let task_id = team.task_create("有缺陷的活".into(), "验收=无缺陷".into()).expect("创建应成功");
    team.task_claim(task_id, "alice").expect("认领应成功");
    team.task_submit(task_id, "alice").expect("提交应成功");
    // lead 驳回（审查人≠提交人 ✓）
    team.task_review(task_id, "lead", false).expect("驳回应成功");
    let tasks = team.task_list();
    assert!(tasks[0].owner.as_deref() == Some("alice"), "驳回不改变负责人记录");
    // 终态锁：驳回后不可再提交/再审
    assert!(team.task_submit(task_id, "alice").is_err(), "驳回终态不可再提交");
    assert!(team.task_review(task_id, "lead", true).is_err(), "驳回终态不可再审");
}

/// ⑦lead 侧信箱消费缝（task-53 P1）：成员发 lead→drain_lead_inbox 取净
/// （身份=团队 fiber）→compose_turn_input 注入形态（【来自 reviewer】前缀+
/// 原输入；空信箱=原样直通）。验收基准=业主 live 实证「hub 受理≠对话看见」
/// 非对称缝的根除。
#[tokio::test(flavor = "multi_thread")]
async fn lead侧信箱消费缝() {
    let dir = tmp_dir("drain");
    let scripts = Arc::new(std::sync::Mutex::new(vec![FakeLlm::tool_then_answer(
        "send_message",
        r#"{"to":"lead","body":"评审结论：签名合理，建议 checked_add 防溢出"}"#,
        "已把评审结论同步给 lead。",
    )]));
    let root = assemble(FakeLlm { scripts }, &dir).await;
    let team = team_of(&root);
    team.spawn_teammate(TeammateSpec {
        name: "reviewer".into(),
        description: "代码评审员".into(),
        prompt: "你是代码评审员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");
    // 触发成员轮（成员消费 lead 指派并在轮内 send_message 回 lead——业主场景）
    team.send_message(Address::Member("reviewer".into()), MessageKind::Text, "请评审 fn add".into())
        .expect("派发应成功");
    let hub = hub_of(&root);
    // 等 hub 真账：reviewer→lead 受理（「hub 受理」半边，业主实证已有）
    wait_for(|| count_history(&*hub, "reviewer", "lead", MessageKind::Text) >= 1).await;
    // 「对话看见」半边：drain 缝取净（身份=团队 fiber）
    let inbox = team.drain_lead_inbox().expect("lead 信箱 drain 应成功");
    assert!(
        inbox.iter().any(|m| m.from == "reviewer" && m.body.contains("checked_add")),
        "成员报告应在 lead 信箱：{inbox:?}"
    );
    // 注入形态：批量前缀+原输入保留
    let composed = yunyou_team::compose_turn_input(&inbox, "继续推进任务");
    assert!(composed.contains("【来自 reviewer】"), "注入应带成员名前缀：{composed:?}");
    assert!(composed.contains("checked_add"), "消息正文应注入：{composed:?}");
    assert!(composed.ends_with("继续推进任务"), "原输入应保留在后：{composed:?}");
    // 再 drain=空（取净语义）+空信箱合成=原样直通
    assert!(team.drain_lead_inbox().expect("再取应空").is_empty(), "取净语义");
    assert_eq!(
        yunyou_team::compose_turn_input(&[], "纯输入"),
        "纯输入",
        "空信箱合成应零包装直通"
    );
}

/// ⑥成员工具面裁剪：deny 四件 lead 专属（spawn/create/reassign/kick）不投影、
/// send_message 覆写在位、全局存量工具直通（无清单漂移）。
#[tokio::test(flavor = "multi_thread")]
async fn 成员工具面裁剪与覆写() {
    let dir = tmp_dir("tools");
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let global = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("全局注册表应可见");
    let member_view = yunyou_team::test_support::member_registry(&root, "alice", global)
        .expect("成员注册表应可构造");
    for lead_only in ["spawn_teammate", "team_task_create", "team_task_reassign", "team_kick"] {
        assert!(member_view.get(lead_only).is_none(), "{lead_only} 对成员不可见");
    }
    assert!(member_view.get("send_message").is_some(), "send_message 覆写在位");
    assert!(member_view.get("team_task_claim").is_some(), "认领覆写在位");
    let names: Vec<String> =
        member_view.schemas().iter().map(|schema| schema.name.clone()).collect();
    assert!(!names.contains(&"spawn_teammate".to_string()), "schemas 投影过 deny 闸");
}

/// ⑦b spawn 工具面名字白名单前置闸（task-18，挑刺 P1-1 同源收口）：成员名
/// 流向会话文件路径（teammate-{name}.jsonl），穿越形态在 hub 层之前以
/// ToolOutcome::Err 人话拒绝（与 hub validate_member_name 同一规则单一源）。
/// 探针三形态（../、绝对路径、裸 ..）全拒+合法名过闸。
#[tokio::test(flavor = "multi_thread")]
async fn spawn工具面名字白名单前置拒() {
    let dir = tmp_dir("spawn-name-guard");
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let registry = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("全局注册表应可见");
    let tool = registry
        .get("spawn_teammate")
        .expect("spawn_teammate 应已注册");
    let exec = yunyou_tools::ToolExec {
        call_id: "t18".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        workspace: None,
    };
    for bad in ["../evil", "/etc/evil", "..", "Bad-Name", "has space"] {
        let outcome = tool
            .execute(
                serde_json::json!({"name": bad, "description": "d", "prompt": "p"}),
                exec.clone(),
            )
            .await;
        match &outcome {
            yunyou_tools::ToolOutcome::Err(err) => {
                assert_eq!(err.code, "team/args", "{bad:?} 应以 team/args 拒绝");
                assert!(
                    err.message.contains("成员名非法"),
                    "{bad:?} 人话应点名非法：{err:?}"
                );
                assert!(
                    err.hint.as_deref().unwrap_or_default().contains("kebab-case"),
                    "{bad:?} hint 应含合法形态指引：{err:?}"
                );
            }
            other => panic!("{bad:?} 应拒绝: {other:?}"),
        }
    }
    // 合法名过闸（白名单字符集内）：到达 hub 层后走正常 spawn 流（重名/装配
    // 语义不在本用例面——以「非 team/args 名字类拒绝」为过闸判据）
    let outcome = tool
        .execute(
            serde_json::json!({"name": "probe-ok", "description": "d", "prompt": "p"}),
            exec,
        )
        .await;
    if let yunyou_tools::ToolOutcome::Err(err) = &outcome {
        assert!(
            !err.message.contains("成员名非法"),
            "合法名不得被名字闸拦下（应走后续 spawn 链）: {err:?}"
        );
    }
}

/// ⑧广播三态（task-30①，lead 预裁=hub 寻址 All 承诺兑现）：工具面 to="All"
/// 大小写不敏感解析→真广播（全体成员信箱可见+history outcomes N 条）；
/// 零成员人话 Err；点对点 to 不受影响（回归锁）。
#[tokio::test(flavor = "multi_thread")]
async fn 广播all三态() {
    let dir = tmp_dir("broadcast");
    // 两成员在册（Pending 态即可收广播缓冲——All 含 Pending）
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let service = root
        .get::<dyn TeamService>(ServiceId::of::<dyn TeamService>())
        .expect("团队服务应可见");
    // 零成员态：只有 lead 在册→广播人话拒绝
    let zero_err = service
        .send_message(yunyou::hub::Address::All, yunyou::hub::MessageKind::Text, "无人".into())
        .err()
        .expect("零成员广播应拒绝");
    assert!(
        zero_err.to_string().contains("无在册成员"),
        "人话点名零收件人: {zero_err}"
    );
    // 两成员上线（Pending 信箱在册）
    for name in ["alice", "bob"] {
        service
            .spawn_teammate(TeammateSpec {
                name: name.into(),
                description: "广播用例".into(),
                prompt: "ping".into(),
                context: yunyou_team::ContextMode::Fresh,
            })
            .await
            .expect("成员应拉起");
    }
    // 广播：lead 身份发 to=All（Address 构造面；工具面解析由 send_message 工具复用）
    let receipt = service
        .send_message(yunyou::hub::Address::All, yunyou::hub::MessageKind::Text, "集合".into())
        .expect("两成员广播应受理");
    assert_eq!(receipt.recipients, 2, "All 应达除 from 外全体");
    // history 真账：outcomes 两行（名字序稳定）
    let history = root
        .get::<dyn MessageHub>(ServiceId::of::<dyn MessageHub>())
        .expect("hub 应可见")
        .history(HistoryQuery { limit: 100 })
        .expect("history 应可读");
    let last = history.last().expect("应有留痕");
    assert_eq!(last.outcomes.len(), 2, "广播留痕两目标");
    assert_eq!(last.to, "All", "留痕 to 标 All");
}

/// ⑧b 工具面广播解析（task-30①）：send_message 工具 to="ALL"（大小写不敏感）
/// →Address::All 真广播（描述与实现相反的票面在实现侧闭环）。
#[tokio::test(flavor = "multi_thread")]
async fn 工具面广播解析() {
    let dir = tmp_dir("broadcast-tool");
    let root = assemble(FakeLlm { scripts: Arc::new(std::sync::Mutex::new(vec![])) }, &dir).await;
    let registry = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("全局注册表应可见");
    let tool = registry.get("send_message").expect("send_message 应已注册");
    let exec = yunyou_tools::ToolExec {
        call_id: "t30".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        workspace: None,
    };
    service_send_as(&root, "alice").await;
    // alice/bob 在册后广播：工具面 to="ALL"（大写形态）→All 展开（不含 from=alice）
    service_send_as(&root, "bob").await;
    let outcome = tool
        .execute(
            serde_json::json!({"to": "ALL", "body": "全员注意"}),
            exec,
        )
        .await;
    match &outcome {
        yunyou_tools::ToolOutcome::Ok(value) => {
            assert_eq!(value["recipients"], 2, "ALL 广播应达 alice/bob 两成员: {value:?}");
        }
        other => panic!("工具面广播应受理: {other:?}"),
    }
}

/// 在册准备：拉起成员（Pending 信箱在册=广播可见）。
async fn service_send_as(root: &Context, name: &str) {
    let service = team_of(root);
    service
        .spawn_teammate(TeammateSpec {
            name: name.into(),
            description: "广播用例".into(),
            prompt: "ping".into(),
            context: yunyou_team::ContextMode::Fresh,
        })
        .await
        .expect("成员应拉起");
}

/// ⑧c 挂死成员超时熔断（task-30③，云垒 P2-8 解围闸验收）：成员跑 sleep 3（挂死
/// 形态）×tool_timeout_secs=1→1s 超时熔断（permit 释放）→回合继续收尾；整回合
/// 远短于挂死时长=不拖全队。
#[tokio::test(flavor = "multi_thread")]
async fn 挂死工具超时熔断() {
    let dir = tmp_dir("timeout-guard");
    let started = std::time::Instant::now();
    let scripts = Arc::new(std::sync::Mutex::new(vec![FakeLlm::tool_then_answer(
        "bash",
        r#"{"cmd":"sleep 3"}"#,
        "超时后收尾",
    )]));
    let root = assemble_with(
        FakeLlm { scripts },
        &dir,
        serde_json::json!({"tool_timeout_secs": 1}),
    )
    .await;
    let service = team_of(&root);
    service
        .spawn_teammate(TeammateSpec {
            name: "sleeper".into(),
            description: "挂死熔断用例".into(),
            prompt: "跑挂死命令".into(),
            context: yunyou_team::ContextMode::Fresh,
        })
        .await
        .expect("成员应拉起");
    // 回合完成判据：成员完工 Report 落 hub history（run_turn 终结=Report 发送点）
    let hub = hub_of(&root);
    wait_for(|| {
        hub.history(HistoryQuery { limit: 50 })
            .expect("history 应可读")
            .iter()
            .any(|entry| entry.from == "sleeper" && entry.body.contains("超时后收尾"))
    });
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "整回合 {elapsed:?} 不得拖到挂死时长（3s）——超时熔断未生效"
    );
}
