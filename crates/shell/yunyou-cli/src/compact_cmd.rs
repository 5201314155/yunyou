//! /compact 手动压缩命令（task-27 一期，命令缝 D1 Action）：立即摘要压缩
//! 当前会话（不等水位阈值）——用户主动管理上下文。定义与 handler 本件
//!（shell_commands.rs 行数贴硬顶，增量外置先例=sessions_cmd.rs）。
//! 熔断联动：压缩成功清熔断计数（run_compaction 内）；context_window 未配
//! 也可用（摘要压缩不依赖窗口配置，阈值才依赖）。

use crate::shell_commands::ShellCore;
use yunyou_agent_loop::AgentLoop;
use yunyou_commands::{CommandDefinition, CommandHandler, CommandInvocation, CommandKind, CommandOutput};
use yunyou_util::BoxFut;

/// /compact 命令定义（无参 Action）。
pub fn definition(shell: &std::sync::Arc<ShellCore>) -> CommandDefinition {
    CommandDefinition {
        name: "compact".into(),
        aliases: vec!["压缩".into()],
        description: "立即压缩当前会话上下文（摘要前文，完整历史保留可回查）".into(),
        kind: CommandKind::Action,
        accepts_args: false,
        handler: compact_handler(shell.clone()),
    }
}

fn compact_handler(shell: std::sync::Arc<ShellCore>) -> CommandHandler {
    std::sync::Arc::new(move |_inv: CommandInvocation| {
        let shell = shell.clone();
        Box::pin(async move {
            // root 从当前会话槽取（与 REPL 对话回路同源；/new 换槽自动跟随）
            let session = shell.slot.load();
            let agent_loop = match session
                .root
                .get::<dyn AgentLoop>(yunyou::ServiceId::of::<dyn AgentLoop>())
            {
                Ok(loop_service) => loop_service,
                Err(err) => {
                    return Ok(CommandOutput::error(format!(
                        "agentLoop 服务未就绪，无法压缩：{err}"
                    )))
                }
            };
            let cancel = tokio_util::sync::CancellationToken::new();
            match agent_loop.compact_manual(cancel).await {
                Ok(receipt) => Ok(CommandOutput::success(receipt)),
                Err(err) => Ok(CommandOutput::error(format!("压缩失败：{err}（完整历史仍在会话文件，可直接继续对话）"))),
            }
        }) as BoxFut<'static, _>
    })
}
