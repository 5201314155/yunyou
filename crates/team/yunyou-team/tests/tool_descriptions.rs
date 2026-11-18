//! 工具描述断言测试（池 task-19 快刀卡）：自动通知语义必须写进模型可见的
//! 工具描述——业主真机实证「模型答不会自动通知」实为描述缺失（描述是模型
//! 唯一语义源）。断言锁：核心流转工具的 description 含自动通知关键词，
//! 回潮即红。
//! 测试区 Result 风格（新文件零 expect/unwrap，K-14 观察面纪律）。

use std::sync::Arc;
use yunyou_team::test_support::member_registry;
use yunyou_tools::RegistryService;
use yunyou_util::BoxFut;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// 轻量装配：session+registry+TeamPlugin（缺省关态），取 ctx 供 member_registry。
async fn assemble(dir: &std::path::Path) -> Result<yunyou::Context, yunyou::CordisError> {
    let root = yunyou::Context::new();
    let session_path = dir.join("lead.jsonl");
    let store: Arc<dyn yunyou_session::SessionStore> =
        yunyou_session::JsonlStore::new(&session_path);
    root.provide(yunyou::ServiceId::of::<dyn yunyou_session::SessionStore>(), store)?;
    let registry: Arc<dyn RegistryService> = yunyou_tools::ToolsRegistry::new();
    root.provide(yunyou::ServiceId::of::<dyn RegistryService>(), registry)?;
    let llm: Arc<dyn yunyou_llm::Llm> = Arc::new(StubLlm);
    root.provide(yunyou::ServiceId::of::<dyn yunyou_llm::Llm>(), llm)?;
    root.plugin(yunyou_system_prompt::SystemPromptPlugin, serde_json::json!({}));
    let handle = root.plugin(yunyou_team::TeamPlugin, serde_json::json!({
        "path": session_path.to_string_lossy(),
    }));
    for _ in 0..400 {
        if handle.state() == yunyou::FiberState::Active {
            return Ok(root);
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    Err(yunyou::CordisError::new(
        yunyou::ErrorKind::Internal,
        "team 插件未能在限时内装载到 Active",
    ))
}

/// 恒空的 LLM stub（本测试不跑回合，仅满足装配依赖）。
struct StubLlm;
impl yunyou_llm::Llm for StubLlm {
    fn stream(
        &self,
        _req: yunyou_llm::GenerateRequest,
    ) -> BoxFut<'static, yunyou::Result<Box<dyn yunyou_llm::LlmStream>>> {
        Box::pin(async {
            Err(yunyou::CordisError::new(
                yunyou::ErrorKind::Internal,
                "本测试不消费 LLM",
            ))
        })
    }
}

/// 在注册表中取指定工具的描述（缺工具=Err 人话，不 panic）。
fn description_of<'a>(
    schemas: &'a [yunyou_llm::ToolSchema],
    name: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    schemas
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.description.as_str())
        .ok_or_else(|| format!("工具 {name} 应在注册表").into())
}

/// lead 侧断言：submit/approve/claim/create/list 的描述含自动通知语义关键词。
#[tokio::test(flavor = "multi_thread")]
async fn 流转工具描述含自动通知语义() -> TestResult {
    let dir = std::env::temp_dir().join(format!("ya-tool-desc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let root = assemble(&dir).await?;
    let registry = root.get::<dyn RegistryService>(yunyou::ServiceId::of::<dyn RegistryService>())?;
    let schemas = registry.schemas();

    let submit = description_of(&schemas, "team_task_submit")?;
    assert!(
        submit.contains("自动通知") && submit.contains("lead"),
        "submit 描述缺自动送审语义: {submit}"
    );
    let review = description_of(&schemas, "team_task_review")?;
    assert!(
        review.contains("自动通知") && review.contains("前置已通过"),
        "review 描述缺自动解锁语义: {review}"
    );
    let create = description_of(&schemas, "team_task_create")?;
    assert!(create.contains("自动通知"), "create 描述缺自动通知预告: {create}");
    let list = description_of(&schemas, "team_task_list")?;
    assert!(list.contains("自动"), "task_list 描述缺流转自动通知说明: {list}");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// 成员侧覆写面（claim/submit 成员分支）同样携带自动通知语义——成员是
/// 「不会自动通知」误答的第一现场（池卡面实证）。
#[tokio::test(flavor = "multi_thread")]
async fn 成员侧覆写描述同步携带语义() -> TestResult {
    let dir = std::env::temp_dir().join(format!("ya-tool-desc-m-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let root = assemble(&dir).await?;
    let global = root.get::<dyn RegistryService>(yunyou::ServiceId::of::<dyn RegistryService>())?;
    let member = member_registry(&root, "scout", global)?;
    let schemas = member.schemas();

    let claim = description_of(&schemas, "team_task_claim")?;
    assert!(claim.contains("自动收到"), "成员 claim 描述缺解锁自动通知: {claim}");
    let submit = description_of(&schemas, "team_task_submit")?;
    assert!(
        submit.contains("自动通知") && submit.contains("lead"),
        "成员 submit 描述缺自动送审语义: {submit}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
