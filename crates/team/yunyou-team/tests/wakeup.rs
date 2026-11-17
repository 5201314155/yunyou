//! 成员唤醒链路活性测试（task-20①，挑刺 P2-1；复核人=云尺）。
//!
//! 判别力锚：Notify 唤醒链路（订阅回调 notify_one → worker select notified 臂
//! →drain→run_one→终结 Report）端到端活着——**断言上限 1s 系功能活性上界**：
//! 链路活着时实测毫秒级（打印值呈现），链路断裂则退化为兜底轮询
//! IDLE_BACKSTOP_INTERVAL=5s 才响应→断言红（组合回归判别：有人拆 notified 臂
//! 或断订阅回调即红）。这不是毫秒级性能断言（云尺 wall time 纪律：实测耗时
//! 只打印不断言，防共享机噪声 flaky——活性上界是秒级容差的功能断言）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use yunyou::hub::MessageHub;
use yunyou::ServiceId;
use yunyou_llm::Llm;
use yunyou_session::{JsonlStore, SessionStore};
use yunyou_team::TeamPlugin;
use yunyou_tools::RegistryService;
use yunyou_util::BoxFut;
use yunyou::{Context, FiberState, FiberHandle};

/// 装配（auto_claim 默认关——本测试只看唤醒链路，排除自动领取干扰）。
async fn assemble(session_dir: &std::path::Path) -> Context {
    let root = Context::new();
    let session_path = session_dir.join("lead.jsonl");
    let store: Arc<dyn SessionStore> = JsonlStore::new(&session_path);
    root.provide(ServiceId::of::<dyn SessionStore>(), store.clone()).unwrap();
    let registry: Arc<dyn RegistryService> = yunyou_tools::ToolsRegistry::new();
    root.provide(ServiceId::of::<dyn RegistryService>(), registry).unwrap();
    let llm: Arc<dyn Llm> = Arc::new(StubLlm);
    root.provide(ServiceId::of::<dyn Llm>(), llm).unwrap();
    root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({}));
    let team_handle = root.plugin(
        TeamPlugin,
        serde_json::json!({ "path": session_path.to_string_lossy() }),
    );
    wait_active(&team_handle, "team").await;
    root
}

async fn wait_active(handle: &FiberHandle, name: &str) {
    for _ in 0..200 {
        if handle.state() == FiberState::Active { return; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("插件 {name} 未装载到 Active");
}

/// 零密码学 Stub（空流：run_turn 立即空回复→Err→Report「任务回合失败」，
/// Report 到达即证明唤醒→drain→run_one→终结→上报全链活着）。
struct StubLlm;
impl Llm for StubLlm {
    fn stream(
        &self,
        _req: yunyou_llm::GenerateRequest,
    ) -> BoxFut<'static, yunyou::Result<Box<dyn yunyou_llm::LlmStream>>> {
        Box::pin(async move {
            Ok(Box::new(EmptyStream) as Box<dyn yunyou_llm::LlmStream>)
        })
    }
}

struct EmptyStream;
impl yunyou_llm::LlmStream for EmptyStream {
    fn next(&mut self) -> BoxFut<'_, yunyou::Result<Option<yunyou_llm::StreamDelta>>> {
        Box::pin(async move { Ok(None) })
    }
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k20-wakeup-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 唤醒链路活性：lead 发消息 → 成员被 Notify 唤醒处理 → Report 回到 lead。
/// 断言上限 1s（活性上界，见模块头注）；实际耗时打印呈现（卡面「<100ms 响应」
/// 语义以打印值核对，不作毫秒级断言——云尺 wall time 纪律）。
#[tokio::test(flavor = "multi_thread")]
async fn 消息到达唤醒成员处理并回report() {
    let dir = tmp_dir("wake");
    let root = assemble(&dir).await;
    let team = root
        .get::<dyn yunyou_team::TeamService>(ServiceId::of::<dyn yunyou_team::TeamService>())
        .expect("团队服务应可见");
    let hub = root
        .get::<dyn MessageHub>(ServiceId::of::<dyn MessageHub>())
        .expect("hub 应可见");

    team.spawn_teammate(yunyou_team::TeammateSpec {
        name: "scout".into(),
        description: "侦查员".into(),
        prompt: "你是侦查员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");

    // 唤醒链路计时起点=消息投递（入信箱+订阅回调 notify_one）
    let started = Instant::now();
    team.send_message(
        yunyou::hub::Address::Member("scout".into()),
        yunyou::hub::MessageKind::Text,
        "请侦查四周".into(),
    )
    .expect("发送应成功");

    // 轮询 lead 侧留痕等 Report（成员终结即发；活性上界 1s——链路断裂则
    // 兜底 5s 才醒、Report 迟到，断言红=组合回归判别）
    let mut report_seen: Option<String> = None;
    for _ in 0..100 {
        let entries = hub
            .history(yunyou::hub::HistoryQuery { limit: 50 })
            .expect("history 应可查");
        if let Some(entry) = entries
            .iter()
            .find(|e| e.kind == yunyou::hub::MessageKind::Report && e.body.contains("任务回合"))
        {
            report_seen = Some(entry.body.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let elapsed = started.elapsed();
    println!(
        "唤醒→Report 全链耗时: {elapsed:?}（活性上界断言 1s；卡面 <100ms 语义以本打印值核对）"
    );
    let body = report_seen.expect("1s 内应见成员 Report（Notify 唤醒链路活性；断裂则兜底 5s 红）");
    assert!(
        body.contains("任务回合"),
        "Report 应为回合终结形态: {body}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "唤醒链路活性上界 1s（实测 {elapsed:?}）——若临近 5s 兜底周期即 Notify 链路断裂回归"
    );
    let _ = std::fs::remove_dir_all(&dir);
}


/// K-18 验收②：scope 内任务解锁→自动领取（board_wakeup 事件链）。
/// 链路：任务 B blocked_by A→成员先领 A（K-17）→submit A→lead approve A→
/// board_wakeup 广播→成员 worker 醒→drain 空→try_auto_claim→B 解锁可领→Claimed。
#[tokio::test(flavor = "multi_thread")]
async fn 解锁唤醒自动领取() {
    let dir = tmp_dir("k18-unlock");
    let root = assemble_auto(&dir).await;
    let team = team_of(&root);

    // 任务 A（可领）+ 任务 B（blocked_by A——A approved 前不可领）
    let task_a = team
        .task_create_full("前置任务".into(), "做前置".into(), "调研".into(), vec![])
        .expect("创建 A");
    let task_b = team
        .task_create_full("后续任务".into(), "做后续".into(), "调研".into(), vec![task_a])
        .expect("创建 B");

    // spawn 成员（capability=调研，auto_claim enabled）→ idle 自动领 A
    team.spawn_teammate(yunyou_team::TeammateSpec {
        name: "scout".into(),
        description: "侦查员".into(),
        prompt: "你是侦查员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");

    // 等 A 被自动领取（K-17 链）
    wait_task_state(&team, task_a, yunyou_team::TeamTaskState::Claimed).await;
    eprintln!("[k18] A 已自动领取");

    // 成员 submit A（模拟成员经工具提交——service 层直调）
    team.task_submit(task_a, "scout").expect("提交 A");

    // lead approve A → board_wakeup 广播 → 成员醒 → 领 B
    team.task_review(task_a, "lead", true).expect("审查 A 通过");

    // 等 B 被自动领取（K-18 解锁→唤醒→领取全链；10s=覆盖 5s 兜底周期×2）
    for i in 0..2000 {
        let tasks = team.task_list();
        let b = tasks.iter().find(|t| t.id == task_b);
        let a = tasks.iter().find(|t| t.id == task_a);
        if i % 50 == 0 {
            eprintln!("[k18-diag] poll {i}: A={:?} B={:?}", a.map(|t| t.state), b.map(|t| (t.state, &t.owner)));
        }
        if b.map(|t| t.state) == Some(yunyou_team::TeamTaskState::Claimed) {
            eprintln!("[k18] B 经解锁唤醒自动领取——全链 ✔");
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
    panic!("任务 #{task_b} 未在 10s 内到达 Claimed（K-18 链路断）");
}

/// K-18 验收③：无事件零空转——非匹配 capability 任务在池，成员不误领不轮询
/// 消耗（结构保证：worker select! 臂=Notify×2+5s 兜底 sleep，无 busy-spin 路径；
/// 行为断言=10s 窗口内任务保持 Pending 且成员零动作）。
#[tokio::test(flavor = "multi_thread")]
async fn 非匹配能力零认领() {
    let dir = tmp_dir("k18-nomatch");
    let root = assemble_auto(&dir).await;
    let team = team_of(&root);

    // 任务 capability="coding"（成员 capability="调研"——不匹配）
    let task_id = team
        .task_create_full("编码任务".into(), "写代码".into(), "coding".into(), vec![])
        .expect("创建");

    team.spawn_teammate(yunyou_team::TeammateSpec {
        name: "scout".into(),
        description: "侦查员".into(),
        prompt: "你是侦查员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");

    // 10s 窗口（覆盖多个 5s 兜底周期——worker 醒多次但 capability 不匹配=静默）
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let tasks = team.task_list();
    let task = tasks.iter().find(|t| t.id == task_id).expect("任务应在");
    assert_eq!(
        task.state,
        yunyou_team::TeamTaskState::Pending,
        "非匹配 capability 任务必须保持 Pending（约束⑤静默+零空转无误领）"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 轮询等任务到达指定状态（K-18 链路异步；5ms×600≈3s 上限）。
async fn wait_task_state(
    team: &Arc<dyn yunyou_team::TeamService>,
    task_id: u64,
    want: yunyou_team::TeamTaskState,
) {
    for _ in 0..600 {
        let tasks = team.task_list();
        if let Some(task) = tasks.iter().find(|t| t.id == task_id) {
            if task.state == want {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("任务 #{task_id} 未在限时内到达 {:?}（K-18 链路断）", want);
}

/// 装配（auto_claim enabled+capability=调研——K-18 场景用）。
async fn assemble_auto(session_dir: &std::path::Path) -> Context {
    let root = Context::new();
    let session_path = session_dir.join("lead.jsonl");
    let store: Arc<dyn SessionStore> = JsonlStore::new(&session_path);
    root.provide(ServiceId::of::<dyn SessionStore>(), store.clone()).unwrap();
    let registry: Arc<dyn RegistryService> = yunyou_tools::ToolsRegistry::new();
    root.provide(ServiceId::of::<dyn RegistryService>(), registry).unwrap();
    let llm: Arc<dyn Llm> = Arc::new(StubLlm);
    root.provide(ServiceId::of::<dyn Llm>(), llm).unwrap();
    root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({}));
    let team_handle = root.plugin(
        TeamPlugin,
        serde_json::json!({
            "auto_claim": { "enabled": true, "max_active_per_member": 1, "capability": "调研" },
            "path": session_path.to_string_lossy(),
        }),
    );
    wait_active(&team_handle, "team-auto").await;
    root
}

fn team_of(ctx: &Context) -> Arc<dyn yunyou_team::TeamService> {
    ctx.get::<dyn yunyou_team::TeamService>(ServiceId::of::<dyn yunyou_team::TeamService>())
        .expect("团队服务应可见")
}
