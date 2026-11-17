//! 工具执行器：超时 / 取消传播 / 结果归一（对标 TS executor 的服务面裁剪）。

use crate::contract::{ToolDefinition, ToolError, ToolExec, ToolOutcome};
use yunyou::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// 执行一个工具调用（统一入口：超时 + 取消 + 错误归一）。
///
/// # 契约
/// - `cancel` 触发或 `timeout` 到期 → 返回 `ToolOutcome::Err`（code 分别为
///   `tool/cancelled`、`tool/timeout`）；不抛弃工具的 future，等其静止
///   （对齐 contracts.ts「does not abandon this promise」——超时后仍 join 完成态）；
/// - 工具内部 panic 不会击穿执行器（tokio 任务边界捕获为 `tool/panic`）；
/// - **信号必达（task-14 R2 接线修复）**：工具拿到的 cancel 是回合令牌的
///   **子令牌**（`child_token` 派生）——回合取消经父子联动自动传导；
///   **超时分支显式 `child.cancel()`** 再返回 `tool/timeout`——超时不是
///   「只上屏错误」，工具任务收到信号后自行收尾（bash 组收割等挂点得以
///   触发），进程树不双活。abort（强杀 future）仍不使用：任务收尾靠信号
///   不靠抛弃，「等其静止」语义保持。
///
/// # 参数
/// - `timeout`：`None` = 不限时（bash 等自带取消观察的工具可不限时）。
pub async fn execute_tool(
    tool: Arc<dyn ToolDefinition>,
    args: serde_json::Value,
    call_id: String,
    cancel: CancellationToken,
    timeout: Option<Duration>,
) -> ToolOutcome {
    // 契约⑨槽位预留（方案 §3-⑨）：一期不实现校验逻辑，恒 None；
    // P1-11a 策略对象落地时由本签名补传 workspace 上下文。
    let tool_cancel = cancel.child_token();
    let exec = ToolExec { call_id: call_id.clone(), cancel: tool_cancel.clone(), workspace: None };
    let tool_for_task = tool.clone();
    let task = tokio::spawn(async move { tool_for_task.execute(args, exec).await });
    let deadline = async {
        match timeout {
            Some(duration) => tokio::time::sleep(duration).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        joined = task => match joined {
            Ok(outcome) => outcome,
            Err(join_err) => ToolOutcome::Err(ToolError::new(
                "tool/panic",
                format!("工具执行任务异常结束: {join_err}"),
            )),
        },
        _ = deadline => {
            // 超时先发信号再返回：分离任务收到 child cancel 自行收尾
            // （bash：取消分支 killpg 全组+reap 后返回，任务秒级静止）；
            // 本函数不 join 等它（超时结果即时上屏，任务完成态由其自行承担）。
            tool_cancel.cancel();
            ToolOutcome::Err(ToolError::new(
                "tool/timeout",
                format!("工具 {} 执行超时", tool.name()),
            ))
        }
        _ = cancel.cancelled() => ToolOutcome::Err(ToolError::new(
            "tool/cancelled",
            "工具执行被取消",
        )),
    }
}

/// 便捷入口：不设超时的执行（多数调用方传 fiber 的取消令牌）。
pub async fn run(tool: Arc<dyn ToolDefinition>, args: serde_json::Value, exec: ToolExec) -> ToolOutcome {
    execute_tool(tool, args, exec.call_id, exec.cancel, None).await
}

/// 执行器错误 → 会话可见文本的统一渲染。
pub fn outcome_text(outcome: &ToolOutcome) -> String {
    crate::contract::render_outcome(outcome)
}

/// 供调用方把执行结果归一为 (content, is_error)。
pub fn outcome_parts(outcome: &ToolOutcome) -> (String, bool) {
    match outcome {
        ToolOutcome::Ok(value) => (crate::contract::render_outcome(&ToolOutcome::Ok(value.clone())), false),
        // 错误渲染单源化走 render_error（含 hint 指路面，防两份格式漂移）
        ToolOutcome::Err(err) => (crate::contract::render_error(err), true),
    }
}

/// 确保调用方在无超时语义下也能等待取消（占位辅助，避免未使用告警）。
pub fn noop_result() -> Result<ToolOutcome> {
    Ok(ToolOutcome::Ok(serde_json::Value::Null))
}
