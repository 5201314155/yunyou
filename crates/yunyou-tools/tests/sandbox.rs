//! task-19 沙箱集成测试（阶段 4 收尾）：BashTool 降级路径 meta 标注验证。
//! helper 链端到端（Landlock/seccomp）在 worktree 隔离场单测覆盖——
//! 主树测试文件只验降级路径（enforcer=None 的 meta 标注），与 worktree
//! 隔离场互补（家规第四条：单元锁行为，集成真机兜底）。

use std::sync::Arc;
use yunyou_tools::builtin::bash::BashTool;
use yunyou_tools::builtin::cwd::CwdService;
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};

fn exec() -> ToolExec {
    ToolExec { call_id: "sbx".into(), cancel: tokio_util::sync::CancellationToken::new(), workspace: None }
}

#[tokio::test(flavor = "multi_thread")]
async fn 降级路径_meta标注sandboxed_false() {
    let dir = std::env::temp_dir().join(format!("sbx-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tool = BashTool { cwd: Arc::new(CwdService::new(&dir)), enforcer: None };
    let outcome = tool
        .execute(serde_json::json!({"cmd": "echo plain"}), exec())
        .await;
    match &outcome {
        ToolOutcome::Ok(v) => {
            assert_eq!(v["sandboxed"], serde_json::json!(false), "降级 meta 标注");
        }
        other => panic!("正常命令应 Ok：{other:?}"),
    }
    std::fs::remove_dir_all(&dir).ok();
}
