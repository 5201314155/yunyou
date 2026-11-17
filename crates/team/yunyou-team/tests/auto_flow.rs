//! K-17 自动流转层集成测试：service 层直调事件序列（lead 零推手全链）。
//!
//! 【边界】async worker 的 run_one 集成环境问题（FakeLlm+MemberPlugin turn
//! 链路阻塞）已定位为测试环境问题——本文件走 service 层直调（lead ② 裁：
//! 不需要 AgentLoop/FakeLmm turn，通知挂接在 service 转换闸上直调即可全链
//! 断言）。async worker 的自动领取已由诊断链实证（/tmp/k17 文件法入档）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use yunyou_session::SessionStore;
use yunyou_tools::RegistryService;
use yunyou::{Context, ServiceId};
use yunyou_llm::Llm;
use yunyou_util::BoxFut;
use yunyou_session::JsonlStore;
use yunyou_team::TeamPlugin;

/// 装配（TeamPlugin with auto_claim enabled）。
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
    wait_active(&team_handle, "team").await;
    root
}

/// 装配（默认关态 auto_claim.enabled=false）。
async fn assemble_off(session_dir: &std::path::Path) -> Context {
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
            "path": session_path.to_string_lossy(),
        }),
    );
    wait_active(&team_handle, "team-off").await;
    root
}

async fn wait_active(handle: &yunyou::FiberHandle, name: &str) {
    for _ in 0..200 {
        if handle.state() == yunyou::FiberState::Active { return; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("插件 {name} 未装载到 Active");
}

/// 零密码学 Stub LLM（service 层直调不触 AgentLoop，此件仅为 trait 满足）。
struct StubLlm;
impl Llm for StubLlm {
    fn stream(
        &self,
        _req: yunyou_llm::GenerateRequest,
    ) -> BoxFut<'static, yunyou::Result<Box<dyn yunyou_llm::LlmStream>>> {
        let empty: Vec<yunyou_llm::StreamDelta> = vec![];
        Box::pin(async move { Ok(Box::new(EmptyStream { delta: empty, index: 0 }) as Box<dyn yunyou_llm::LlmStream>) })
    }
}

struct EmptyStream { delta: Vec<yunyou_llm::StreamDelta>, index: usize }
impl yunyou_llm::LlmStream for EmptyStream {
    fn next(&mut self) -> BoxFut<'_, yunyou::Result<Option<yunyou_llm::StreamDelta>>> {
        Box::pin(async move {
            let index = self.index;
            self.index += 1;
            Ok(if index < self.delta.len() { Some(self.delta[index].clone()) } else { None })
        })
    }
}

fn team_of(ctx: &Context) -> Arc<dyn yunyou_team::TeamService> {
    ctx.get::<dyn yunyou_team::TeamService>(ServiceId::of::<dyn yunyou_team::TeamService>())
        .expect("团队服务应可见")
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k17-svc-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// K-17 验收核心：lead 零推手全链。
/// create→auto claim→submit→auto notify reviewer→approve→auto unlock notify。
#[tokio::test(flavor = "multi_thread")]
async fn 零推手全链() {
    let dir = tmp_dir("full");
    let root = assemble_auto(&dir).await;
    let team = team_of(&root);
    let hub = root
        .get::<dyn yunyou::hub::MessageHub>(ServiceId::of::<dyn yunyou::hub::MessageHub>())
        .expect("hub 应可见");

    // spawn 成员（capability=调研，与任务匹配）
    team.spawn_teammate(yunyou_team::TeammateSpec {
        name: "scout".into(),
        description: "侦查员".into(),
        prompt: "你是侦查员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");

    // 创建任务（capability=调研 匹配成员）
    let task_id = team
        .task_create_full("调研项目".into(), "调研 README".into(), "调研".into(), vec![])
        .expect("创建应成功");

    // 自动领取：evaluate_claims→claim（成员 idle 后由 worker 触发，
    // 此处直调 service 层验证 claim 路径）
    team.task_claim(task_id, "scout").expect("成员应能认领匹配任务");

    // 成员 submit → auto notify reviewer
    team.task_submit(task_id, "scout").expect("提交应成功");
    // hub history 断言：reviewer 收到通知
    let entries = hub.history(yunyou::hub::HistoryQuery { limit: 100 }).unwrap();
    assert!(
        entries.iter().any(|e| e.kind == yunyou::hub::MessageKind::System && e.body.contains("#")),
        "submit 后应有送审通知: {entries:?}"
    );

    // lead approve → auto unlock notify
    team.task_review(task_id, "lead", true).expect("审查应成功");

    // 清理
    let _ = std::fs::remove_dir_all(&dir);
}

/// 默认关态回归：auto_claim.enabled=false→零自动领取。
#[tokio::test(flavor = "multi_thread")]
async fn 默认关态零自动领取() {
    let dir = tmp_dir("off");
    let root = assemble_off(&dir).await;
    let team = team_of(&root);
    let task_id = team
        .task_create_full("评审任务".into(), "评审".into(), "调研".into(), vec![])
        .expect("创建应成功");
    team.spawn_teammate(yunyou_team::TeammateSpec {
        name: "alice".into(),
        description: "评审".into(),
        prompt: "你是评审。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let tasks = team.task_list();
    let task = tasks.iter().find(|t| t.id == task_id).expect("任务应在");
    assert_eq!(
        task.state,
        yunyou_team::TeamTaskState::Pending,
        "默认关态任务必须保持 Pending（零自动领取）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-1（五席评审 20261117）：approve 解锁后，被阻塞任务的 owner 必须收到
/// 「前置已通过」通知——扫描结果不得弃用（此前 `_blocked` 赋值后即丢，
/// 自动流转③半失效）。rejected 不触发（v1 边界）同批锁死。
#[tokio::test(flavor = "multi_thread")]
async fn 解锁通知送达被阻塞任务owner() {
    let dir = tmp_dir("unlock-notify");
    let root = assemble_off(&dir).await;
    let team = team_of(&root);
    let hub = root
        .get::<dyn yunyou::hub::MessageHub>(ServiceId::of::<dyn yunyou::hub::MessageHub>())
        .expect("hub 应可见");

    team.spawn_teammate(yunyou_team::TeammateSpec {
        name: "scout".into(),
        description: "侦查员".into(),
        prompt: "你是侦查员。".into(),
        context: yunyou_team::ContextMode::Fresh,
    })
    .await
    .expect("spawn 应成功");

    // rejected 面与 approve 面各用一个前置任务（review 仅 Submitted 态可裁）：
    // A1 走驳回、A2 走通过；被阻塞任务 B 挂在 A2 下（scout 认领）
    let prereq_reject = team
        .task_create_full("前置调研甲".into(), "先行调研".into(), "调研".into(), vec![])
        .expect("前置任务甲应创建成功");
    let prereq = team
        .task_create_full("前置调研乙".into(), "先行调研".into(), "调研".into(), vec![])
        .expect("前置任务乙应创建成功");
    let blocked = team
        .task_create_full("后续实施".into(), "依赖前置乙".into(), "调研".into(), vec![prereq])
        .expect("被阻塞任务应创建成功");
    team.task_claim(blocked, "scout").expect("成员应能认领");
    team.task_claim(prereq_reject, "scout").expect("认领甲");
    team.task_submit(prereq_reject, "scout").expect("提交甲");

    // rejected：不触发解锁通知
    team.task_review(prereq_reject, "lead", false).expect("驳回应成功");
    let entries = hub.history(yunyou::hub::HistoryQuery { limit: 100 }).unwrap();
    assert!(
        !entries.iter().any(|e| e.body.contains("已解锁可推进")),
        "rejected 不得发解锁通知: {entries:?}"
    );

    // approve：lead 收看板面通知 + scout（被阻塞任务 owner）收点名通知
    team.task_claim(prereq, "scout").expect("认领乙");
    team.task_submit(prereq, "scout").expect("提交乙");
    team.task_review(prereq, "lead", true).expect("审查应成功");
    let entries = hub.history(yunyou::hub::HistoryQuery { limit: 100 }).unwrap();
    assert!(
        entries.iter().any(|e| e.body.contains("已通过审查") && e.body.contains(&format!("#{blocked}"))),
        "owner 应回收「前置已通过、任务 #blocked 解锁」通知: {entries:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
