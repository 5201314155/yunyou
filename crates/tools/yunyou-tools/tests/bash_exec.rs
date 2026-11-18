//! bash 工具执行类集成测试（task-14 自 bash.rs tests mod 外迁——收割接线
//! 增量后源文件贴 400 行硬顶，测试迁至集成场零语义变化：全走公开 API，
//! 断言逐条保持）。

use std::path::PathBuf;
use std::sync::Arc;
use yunyou_tools::builtin::bash::BashTool;
use yunyou_tools::builtin::cwd::CwdService;
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};

fn exec() -> ToolExec {
    ToolExec { call_id: "t".into(), cancel: tokio_util::sync::CancellationToken::new(), workspace: None }
}

/// 独立 temp root + 注入共享服务的工具（测试互不串目录）。
fn tool(tag: &str) -> (BashTool, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yunyou-tools-bash-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let service = Arc::new(CwdService::new(&dir));
    (BashTool { cwd: service, enforcer: None, tasks: None, background_after_secs: 60 }, dir)
}

/// bash 真实执行（本机有 bash）：退出码与输出。
#[tokio::test]
async fn 执行与退出码() {
    let (tool, dir) = tool("run");
    let outcome = tool.execute(serde_json::json!({"cmd": "echo hello"}), exec()).await;
    match outcome {
        ToolOutcome::Ok(value) => {
            assert_eq!(value["exit_code"], 0);
            assert!(value["output"].as_str().unwrap().contains("hello"));
        }
        other => panic!("应成功: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 统一目录视图：显式 cwd 切起点并捕获；命令内 cd 后共享 cwd 跟随；退出码
/// 穿透 wrapper；exit 后无捕获＝cwd 维持。（不用 `cd ..`——门禁 fail-closed
/// 恒拒 `..`，属既有纪律。）
#[tokio::test]
async fn 执行后捕获pwd更新共享视图() {
    let (tool, dir) = tool("capture");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    // 显式 cwd 参数：起点=解析后的 sub，执行后捕获 sub
    let outcome = tool.execute(serde_json::json!({"cmd": "pwd", "cwd": "sub"}), exec()).await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "{outcome:?}");
    assert_eq!(tool.cwd.cwd(), dir.join("sub"));
    // 命令内 cd（向下建目录后进入）：捕获跟随
    let outcome = tool.execute(serde_json::json!({"cmd": "mkdir -p deep && cd deep"}), exec()).await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "{outcome:?}");
    assert_eq!(tool.cwd.cwd(), dir.join("sub/deep"));
    // 退出码穿透 wrapper；exit 后无捕获＝cwd 维持
    let outcome = tool.execute(serde_json::json!({"cmd": "exit 3"}), exec()).await;
    match outcome {
        ToolOutcome::Ok(value) => assert_eq!(value["exit_code"], 3),
        other => panic!("{other:?}"),
    }
    assert_eq!(tool.cwd.cwd(), dir.join("sub/deep"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// cwd 捕获越界被 clamp 回 root（变量间接绕过门禁的真执行路径实证）。
#[tokio::test]
async fn 越界pwd被clamp回根() {
    let (tool, dir) = tool("clamp");
    let outcome = tool.execute(serde_json::json!({"cmd": "cd $(echo /etc)"}), exec()).await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "变量间接属门禁盲区应放行: {outcome:?}");
    assert_eq!(tool.cwd.cwd(), dir, "越界 pwd 必须 clamp 回 root");
    assert!(tool.cwd.trail().iter().any(|t| t.contains("回退根目录")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// cwd 指向 root 内不存在的目录：rev3 前置拒绝带指路（词法通过但不达
/// spawn——目录缺失在人话层暴露，模型按 mkdir hint 自纠）。
#[tokio::test]
async fn cwd不存在带指路不达spawn() {
    let (tool, dir) = tool("spawn");
    let outcome = tool
        .execute(serde_json::json!({"cmd": "echo x", "cwd": "no-such-dir"}), exec())
        .await;
    match &outcome {
        ToolOutcome::Err(err) => {
            assert_eq!(err.code, "tool/not-found");
            assert!(err.message.contains("目录不存在"), "{err:?}");
            assert!(err.hint.as_deref().unwrap_or_default().contains("mkdir"));
        }
        other => panic!("应前置拒绝: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 取消：睡眠命令被收割。
#[tokio::test]
async fn 取消收割() {
    let (tool, dir) = tool("cancel");
    let cancel = tokio_util::sync::CancellationToken::new();
    let exec = ToolExec { call_id: "t2".into(), cancel: cancel.clone(), workspace: None };
    let task = tokio::spawn(tool.execute(serde_json::json!({"cmd": "sleep 30"}), exec));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    cancel.cancel();
    let outcome = task.await.unwrap();
    match outcome {
        ToolOutcome::Err(err) => assert_eq!(err.code, "tool/cancelled"),
        other => panic!("应取消: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
