//! 工具配置化开关集成测试（池 task-7，业主令「工具读配置开关」）：
//! 默认全开 / `[tools.builtin]` 关闭后 schema 缺席且调用落未知工具 /
//! 拼错工具名键装载期 fail-loud。走完整内核装配（CommandsPlugin → ToolsPlugin）。

use yunyou::{Context, ErrorKind, FiberState, ServiceId};
use yunyou_tools::{RegistryService, ToolsPlugin};

async fn setup_tools_with(config: serde_json::Value) -> (Context, yunyou::FiberHandle) {
    let root = Context::new();
    let commands = root.plugin(yunyou_commands::CommandsPlugin, serde_json::json!({}));
    commands.await_ready().await.expect("commands 插件应装载");
    let handle = root.plugin(ToolsPlugin, config);
    for _ in 0..400 {
        if matches!(handle.state(), FiberState::Active | FiberState::Failed) {
            return (root, handle);
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("ToolsPlugin 未能在限时内落到稳定态，当前 {:?}", handle.state());
}

/// 缺省配置=全开：七内置工具全部进 schemas() 模型面。
#[tokio::test]
async fn 缺省全开_schema齐() {
    let (_root, handle) = setup_tools_with(serde_json::json!({})).await;
    handle.await_ready().await.expect("tools 插件应装载");
    let registry = _root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("注册表应可见");
    let names: Vec<String> = registry.schemas().into_iter().map(|s| s.name).collect();
    for expected in ["bash", "apply_patch", "read", "write", "edit", "glob", "grep"] {
        assert!(names.iter().any(|n| n == expected), "缺省应含工具 {expected}，实际 {names:?}");
    }
}

/// 关闭 bash：不进 schemas()（模型面缺席=LLM 看不见），get 也取不到（真关）。
#[tokio::test]
async fn 关闭工具_schema缺席且不可取() {
    let (root, handle) =
        setup_tools_with(serde_json::json!({ "builtin": { "bash": false } })).await;
    handle.await_ready().await.expect("关闭单工具不应影响装载");
    let registry = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("注册表应可见");
    let names: Vec<String> = registry.schemas().into_iter().map(|s| s.name).collect();
    assert!(!names.iter().any(|n| n == "bash"), "关闭后 bash 不得进模型面，实际 {names:?}");
    assert!(
        names.iter().any(|n| n == "read"),
        "未关闭的工具不受影响（对照面），实际 {names:?}"
    );
    assert!(registry.get("bash").is_none(), "关闭后 get(bash) 应为 None（真关）");
}

/// 拼错工具名键：装载期 fail-loud（Failed 态），错误信息点名未知键与已知清单。
#[tokio::test]
async fn 拼错工具名装载失败() {
    let (_root, handle) =
        setup_tools_with(serde_json::json!({ "builtin": { "basgh": false } })).await;
    let err = handle.await_ready().await.expect_err("拼错键应装载失败");
    assert_eq!(err.kind(), &ErrorKind::Internal, "错误类别应为 Internal，实际 {err:?}");
    let text = err.to_string();
    assert!(text.contains("basgh"), "错误应点名未知键，实际 {text}");
    assert!(text.contains("bash"), "错误应附已知工具清单供指路，实际 {text}");
}
