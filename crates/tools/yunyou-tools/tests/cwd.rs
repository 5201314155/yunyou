//! 统一目录视图集成测试（P0 缺陷一）：bash cd 之后，文件类工具立即基于新
//! cwd 解析相对路径；cwd 恒在 root 内（越界捕获 clamp 回根）；bash 执行是
//! cwd 的唯一推动源，六工具共享同一服务实例。

use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use yunyou_tools::builtin::bash::BashTool;
use yunyou_tools::builtin::cwd::CwdService;
use yunyou_tools::builtin::fs_tools::{ReadTool, WriteTool};
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};

fn exec() -> ToolExec {
    ToolExec { call_id: "cwd".into(), cancel: CancellationToken::new(), workspace: None }
}

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yunyou-it-cwd-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 端到端：bash cd sub → 文件工具相对路径落在 sub 下（缺陷一的核心场景——
/// 旧行为是文件工具仍按老根解析＝路径迷航）。
#[tokio::test]
async fn bash_cd后文件工具仍锚定工作区根() {
    // task-50 rev3 语义改版（业主定调「相对路径绑 WorkspaceRoot 不绑 cwd」）：
    // bash cd 只影响 bash 子进程自身，fs 三件（read/write/edit）相对路径恒锚
    // 工作区根——相对语义可预测+结果回显绝对路径，模型拼错当场可见。旧语义
    // （跟随 cd）由此测试原身锁定，本测试为其语义改版锁。
    let dir = temp_root("follow");
    let cwd = Arc::new(CwdService::new(&dir));
    let bash = BashTool { cwd: cwd.clone(), enforcer: None, tasks: None, background_after_secs: 60 };

    let outcome = bash.execute(serde_json::json!({"cmd": "mkdir -p sub && cd sub"}), exec()).await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "{outcome:?}");
    assert_eq!(cwd.cwd(), dir.join("sub"), "bash cd 后共享 cwd 仍跟随（bash spawn 用）");

    let write = WriteTool { cwd: cwd.clone() };
    let outcome = write.execute(serde_json::json!({"path": "note.txt", "content": "hi"}), exec()).await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "{outcome:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("note.txt")).unwrap(),
        "hi",
        "rev3：文件工具相对路径锚定工作区根落盘"
    );
    assert!(!dir.join("sub/note.txt").exists(), "不得落位 bash 当前目录下");

    let read = ReadTool { cwd: cwd.clone() };
    let outcome = read.execute(serde_json::json!({"path": "note.txt"}), exec()).await;
    match outcome {
        ToolOutcome::Ok(value) => {
            assert!(value["content"].as_str().unwrap().contains("hi"), "read 同锚点可读回");
        }
        other => panic!("read 应成功: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 安全硬约束：bash 里借变量间接绕过门禁 cd 出根（已知盲区），共享 cwd 必须
/// 被 clamp 回 root 并留痕——文件工具随后照常守 root。
#[tokio::test]
async fn bash_pwd越界被clamp回根() {
    let dir = temp_root("clamp");
    let cwd = Arc::new(CwdService::new(&dir));
    let bash = BashTool { cwd: cwd.clone(), enforcer: None, tasks: None, background_after_secs: 60 };

    let outcome = bash.execute(serde_json::json!({"cmd": "cd $(echo /etc)"}), exec()).await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "变量间接属门禁盲区应放行: {outcome:?}");
    assert_eq!(cwd.cwd(), dir, "越界 pwd 必须 clamp 回 root");
    assert!(cwd.trail().iter().any(|t| t.contains("回退根目录")), "clamp 必须留痕");

    // clamp 后文件工具照常守 root：相对路径回 root 基准，越界仍拒
    let write = WriteTool { cwd: cwd.clone() };
    let outcome = write.execute(serde_json::json!({"path": "/etc/passwd", "content": "x"}), exec()).await;
    assert!(matches!(&outcome, ToolOutcome::Err(err) if err.code == "tool/path-outside-root"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// cwd 恒在 root 内的双保险：即使 cwd 已在子目录，文件工具的相对 `..` 逃逸
/// 与绝对路径越界仍然拒绝（path_guard 恒守 root）。
#[tokio::test]
async fn cwd在子目录时root约束不松动() {
    let dir = temp_root("guard");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let cwd = Arc::new(CwdService::new(&dir));
    let bash = BashTool { cwd: cwd.clone(), enforcer: None, tasks: None, background_after_secs: 60 };
    let outcome = bash.execute(serde_json::json!({"cmd": "cd sub"}), exec()).await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "{outcome:?}");

    let write = WriteTool { cwd: cwd.clone() };
    for path in ["../../../etc/passwd", "/etc/passwd"] {
        let outcome = write.execute(serde_json::json!({"path": path, "content": "x"}), exec()).await;
        assert!(
            matches!(&outcome, ToolOutcome::Err(err) if err.code == "tool/path-outside-root"),
            "cwd 在 sub 时 {path} 仍必须拒绝"
        );
    }
    // cwd 捕获未产出（命令提前退出）：cwd 维持不变
    let outcome = bash.execute(serde_json::json!({"cmd": "exit 1"}), exec()).await;
    match outcome {
        ToolOutcome::Ok(value) => assert_eq!(value["exit_code"], 1),
        other => panic!("{other:?}"),
    }
    assert_eq!(cwd.cwd(), dir.join("sub"), "无捕获时 cwd 维持不变");
    let _ = std::fs::remove_dir_all(&dir);
}

/// root 不存在的构造场景（目录尚未创建）：服务以绝对化形态落位，不 panic。
#[test]
fn root未存在时构造不炸() {
    let dir = temp_root("ghost").join("not-yet");
    let service = CwdService::new(&dir);
    assert!(service.cwd().is_absolute());
    assert!(service.trail().is_empty());
}
