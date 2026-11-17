//! patch_tool 工具级验收测试（task-22 卡①-⑦：双文件预检零副作用/部分失败
//! 记账/路径越界拒——ToolExec 全链直调，temp fakehome）。
use super::*;
use crate::builtin::cwd::CwdService;
use crate::contract::{ToolDefinition, ToolExec};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn exec() -> ToolExec {
    ToolExec { call_id: "t22".into(), cancel: CancellationToken::new(), workspace: None }
}

fn tool(home: &std::path::Path) -> ApplyPatchTool {
    ApplyPatchTool { cwd: Arc::new(CwdService::new(home)) }
}

/// 卡①②：双 hunk 预检——hunk2 目标缺失→整体拒绝+零副作用（hunk1 不落盘）。
#[tokio::test]
async fn 双文件预检整体拒绝零副作用() {
    let home = std::env::temp_dir().join(format!("t22a-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("a.txt"), "旧内容\n").unwrap();
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-旧内容\n+新内容\n*** Update File: b.txt\n@@\n-不存在\n+无\n*** End Patch";
    let out = tool(&home).execute(serde_json::json!({"patch": patch}), exec()).await;
    match &out {
        ToolOutcome::Err(e) => assert!(e.message.contains("不存在"), "{e:?}"),
        ToolOutcome::Ok(v) => panic!("预检应整体拒绝: {v:?}"),
    }
    assert_eq!(std::fs::read_to_string(home.join("a.txt")).unwrap(), "旧内容\n", "零副作用：hunk1 不落盘");
    let _ = std::fs::remove_dir_all(&home);
}

/// 卡③：部分失败记账——hunk1 成功+hunk2 写只读目录失败→Err 含已生效清单不回滚。
#[tokio::test]
async fn 部分失败记账不回滚() {
    let home = std::env::temp_dir().join(format!("t22b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("ro")).unwrap();
    std::fs::write(home.join("a.txt"), "旧\n").unwrap();
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-旧\n+新\n*** Add File: ro/新文件.txt\n+内容\n*** End Patch";
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home.join("ro"), std::fs::Permissions::from_mode(0o555)).unwrap();
    }
    let out = tool(&home).execute(serde_json::json!({"patch": patch}), exec()).await;
    match &out {
        ToolOutcome::Err(e) => {
            let msg = &e.message;
            assert!(msg.contains("未回滚") && msg.contains("a.txt"), "记账清单应含已生效项: {msg}");
        }
        ToolOutcome::Ok(v) => panic!("只读目录应失败: {v:?}"),
    }
    assert_eq!(std::fs::read_to_string(home.join("a.txt")).unwrap(), "新\n", "已生效项保留（不回滚）");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(home.join("ro"), std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// 卡⑦：路径越界（../ 逃逸）→tool/path-outside-root 拒。
#[tokio::test]
async fn 路径越界拒绝() {
    let home = std::env::temp_dir().join(format!("t22c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let patch = "*** Begin Patch\n*** Add File: ../逃逸.txt\n+越界\n*** End Patch";
    let out = tool(&home).execute(serde_json::json!({"patch": patch}), exec()).await;
    match &out {
        ToolOutcome::Err(e) => assert!(e.code.contains("path-outside-root"), "{e:?}"),
        ToolOutcome::Ok(v) => panic!("越界应拒: {v:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}
