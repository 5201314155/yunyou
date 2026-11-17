//! 工具面 panic 边界补测（C11，《插件崩溃隔离设计》§6.3）。
//!
//! executor.rs 的 spawn+JoinErr 边界已实现但无专项测试（设计实测确认现缺）——
//! 本文件补上：毒工具 panic → `tool/panic` 错误归一、执行器后续调用不受影响。

use serde_json::{json, Value};
use std::sync::Arc;
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_tools::executor::execute_tool;
use yunyou_util::BoxFut;

/// 毒工具：execute 内直接 panic（任务边界应捕获为 tool/panic）。
struct PoisonTool;

impl ToolDefinition for PoisonTool {
    fn name(&self) -> &str {
        "poison-tool"
    }
    fn description(&self) -> &str {
        "故意爆炸的测试工具"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn execute(&self, _args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        Box::pin(async {
            panic!("毒工具故意爆炸（C11 注入）");
        })
    }
}

/// 健康工具：正常返回（执行器存活性探针）。
struct HealthyTool;

impl ToolDefinition for HealthyTool {
    fn name(&self) -> &str {
        "healthy-tool"
    }
    fn description(&self) -> &str {
        "健康的测试工具"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn execute(&self, _args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        Box::pin(async { ToolOutcome::ok(json!({"ok": true})) })
    }
}

fn exec() -> ToolExec {
    ToolExec { call_id: "c11".into(), cancel: tokio_util::sync::CancellationToken::new(), workspace: None }
}

/// C11：毒工具 panic → tool/panic 归一（不炸测试进程/调用方）；执行器后续调用正常。
#[tokio::test(flavor = "multi_thread")]
async fn 毒工具panic归一且执行器存活() {
    let outcome = execute_tool(Arc::new(PoisonTool), json!({}), "call-1".into(), exec().cancel, None).await;
    match &outcome {
        ToolOutcome::Err(err) => {
            assert_eq!(err.code, "tool/panic", "panic 应归一 tool/panic：{err:?}");
            assert!(err.message.contains("毒工具故意爆炸"), "{err:?}");
        }
        other => panic!("应归一为 Err：{other:?}"),
    }

    // 执行器存活：同执行器继续调度健康工具，正常往返
    let outcome = execute_tool(Arc::new(HealthyTool), json!({}), "call-2".into(), exec().cancel, None).await;
    match &outcome {
        ToolOutcome::Ok(value) => assert_eq!(value["ok"], json!(true), "{value}"),
        other => panic!("健康工具应正常：{other:?}"),
    }
}

/// C11 补充：毒工具带超时——超时与 panic 双路都收敛 Err（任务泄漏面回归观察）。
#[tokio::test(flavor = "multi_thread")]
async fn 毒工具panic在超时语义下同样收敛() {
    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome =
        execute_tool(Arc::new(PoisonTool), json!({}), "call-3".into(), cancel, Some(std::time::Duration::from_secs(5))).await;
    match &outcome {
        ToolOutcome::Err(err) => {
            // panic 先于超时发生：仍应归一 tool/panic（5s 预算未到即已落定）
            assert_eq!(err.code, "tool/panic", "{err:?}");
        }
        other => panic!("应归一为 Err：{other:?}"),
    }
}
