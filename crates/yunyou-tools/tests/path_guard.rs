//! 工具路径约束集成测试（重审补充项 1）：走完整内核装配（ToolsPlugin → 注册表
//! → 工具执行），验证 read/write/edit/glob/grep 对越界路径的拒绝。
//!
//! 约束语义：相对路径锚定 root；`..` 逃逸与绝对路径越出 root 一律拒绝
//!（错误码 tool/path-outside-root）；bash 工具不受文件级路径约束（命令天然
//! 可触达任意路径，由宿主运行环境约束——敏感场景禁用，见 builtin/bash.rs 契约）。

use yunyou::{Context, FiberState, ServiceId};
use yunyou_tools::contract::ToolExec;
use yunyou_tools::{RegistryService, ToolsPlugin};
use tokio_util::sync::CancellationToken;

/// 装配 ToolsPlugin 并等 Active（await_ready 对 Pending 稳定态会立即 Ok，必须轮询）。
///
/// 任务29 C4 起 ToolsPlugin 声明 `inject: dyn CommandsService`（/tools-info 注册
/// 依赖，setup 内缺服务 fail-loud）——测试装配先装 CommandsPlugin 满足依赖。
async fn setup_tools(root: &Context, dir: &std::path::Path) -> Arc<dyn RegistryService> {
    let commands_handle = root.plugin(
        yunyou_commands::CommandsPlugin,
        serde_json::json!({}),
    );
    commands_handle
        .await_ready()
        .await
        .expect("commands 插件应装载");
    let handle = root.plugin(
        ToolsPlugin,
        serde_json::json!({ "root_dir": dir.to_string_lossy() }),
    );
    for _ in 0..400 {
        if handle.state() == FiberState::Active {
            return root
                .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
                .expect("注册表应可见");
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("ToolsPlugin 未能在限时内装载到 Active");
}

fn exec() -> ToolExec {
    ToolExec { call_id: "it".into(), cancel: CancellationToken::new(), workspace: None }
}

/// 六工具中五个文件/检索工具的越界拒绝矩阵（bash 不受文件级约束，见模块注释）。
#[tokio::test(flavor = "multi_thread")]
async fn 越界路径拒绝矩阵() {
    let dir = std::env::temp_dir().join(format!("dsh-tools-guard-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    std::fs::write(dir.join("inner/seed.txt"), "hello\n").unwrap();
    let root = Context::new();
    let registry = setup_tools(&root, &dir).await;

    let outside = "../../etc/passwd";
    let cases: Vec<(&str, serde_json::Value)> = vec![
        ("read", serde_json::json!({"path": outside})),
        ("write", serde_json::json!({"path": outside, "content": "x"})),
        (
            "edit",
            serde_json::json!({"path": outside, "old": "a", "new": "b"}),
        ),
        ("glob", serde_json::json!({"pattern": "../../etc/*"})),
        ("grep", serde_json::json!({"pattern": "root", "subdir": "../../etc"})),
    ];
    for (name, args) in cases {
        let tool = registry.get(name).unwrap_or_else(|| panic!("工具 {name} 应已注册"));
        let outcome = tool.execute(args, exec()).await;
        assert!(
            matches!(&outcome, yunyou_tools::ToolOutcome::Err(err) if err.code == "tool/path-outside-root"),
            "工具 {name} 的越界调用应被拒绝，实得 {outcome:?}"
        );
    }

    // 绝对路径越出 root 同样拒绝（write 直接指向 /etc）
    let tool = registry.get("write").unwrap();
    let outcome = tool
        .execute(serde_json::json!({"path": "/etc/dsh-should-not-exist", "content": "x"}), exec())
        .await;
    assert!(
        matches!(&outcome, yunyou_tools::ToolOutcome::Err(err) if err.code == "tool/path-outside-root"),
        "绝对路径越界应被拒绝，实得 {outcome:?}"
    );

    // 对照组：root 内的合法路径正常工作（证明拒绝是约束而非工具损坏）
    let read = registry.get("read").unwrap();
    let outcome = read.execute(serde_json::json!({"path": "inner/seed.txt"}), exec()).await;
    assert!(
        matches!(&outcome, yunyou_tools::ToolOutcome::Ok(value) if value["content"].as_str().unwrap().contains("hello")),
        "root 内读取应正常: {outcome:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

use std::sync::Arc;
