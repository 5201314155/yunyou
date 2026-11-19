//! /mode 权限模式切换命令（task-16 快刀卡，命令缝 D1 Action）：运行期切换
//! S3 四模式引擎的会话态——无参=显示当前模式+四模式一句话说明；带参=切换
//! （写 GateHub.mode 会话内存态，run_tool 组装 GateContext 立即生效；会话
//! 结束随进程态收回=模式/授权不残留哲学）。
//!
//! # 设计决策（lead 报备点：内存 vs session-meta）
//! v1=**会话内存态**（快刀+「权限态不残留」安全哲学——模式是会话级工作姿态，
//! 跨会话残留反而危险：新会话不知道旧会话开过 bypass）；session-meta 持久化
//! （恢复会话带回模式）=演进项候单独小卡，涉及 session crate 事件面。

use crate::shell_commands::ShellCore;
use yunyou::ServiceId;
use yunyou_agent_loop::ModeStateService;
use yunyou_commands::{CommandDefinition, CommandHandler, CommandInvocation, CommandKind, CommandOutput};
use yunyou_util::BoxFut;

/// 四模式一句话说明（/mode 无参展示用；与 mode.rs 矩阵语义单源——文案此处
/// 面向用户，语义在 yunyou-permission::mode 引擎，两处不得矛盾）。
const MODE_HELP: &str = "  default      缺省：工作区内写直接放行，区外写/执行命令请求审批\n  acceptEdits  编辑放行：同 default（更宽编辑面演进位）\n  plan         计划：恒只读调研，写操作与命令一律请求审批\n  bypass       旁路：不询问直接放行（deny 规则与高危拦截仍生效）";

/// /mode 命令定义（accepts_args=true：带参=切换，无参=展示）。
pub fn definition(shell: &std::sync::Arc<ShellCore>) -> CommandDefinition {
    CommandDefinition {
        name: "mode".into(),
        aliases: vec!["模式".into()],
        description: "查看或切换权限模式（default/acceptEdits/plan/bypass）".into(),
        kind: CommandKind::Action,
        accepts_args: true,
        handler: mode_handler(shell.clone()),
    }
}

fn mode_handler(shell: std::sync::Arc<ShellCore>) -> CommandHandler {
    std::sync::Arc::new(move |inv: CommandInvocation| {
        let shell = shell.clone();
        Box::pin(async move {
            // mode 态经当前会话根取服务（与 REPL 对话回路同源；/new 换槽自动跟随）
            let session = shell.slot.load();
            let service = match session
                .root
                .get::<dyn ModeStateService>(ServiceId::of::<dyn ModeStateService>())
            {
                Ok(s) => s,
                Err(err) => {
                    return Ok(CommandOutput::error(format!(
                        "模式态服务未就绪：{err}"
                    )))
                }
            };
            // 参数提取（与 command_exec::extra_args 同源复用，sessions_cmd 先例）
            let token = inv.raw_input.split_whitespace().next().unwrap_or("");
            let arg = crate::command_exec::extra_args(&inv.raw_input, token)
                .unwrap_or("")
                .trim()
                .to_string();
            let arg = arg.as_str();
            if arg.is_empty() {
                let current = service
                    .current_mode()
                    .unwrap_or_else(|| "default（未显式设置）".into());
                return Ok(CommandOutput::success(format!(
                    "当前权限模式：{current}\n四模式说明：\n{MODE_HELP}\n切换：/mode <模式名>"
                )));
            }
            // 词汇校验（人话指路；未知词不写态——策略侧虽会保守降 Default，
            // 命令面直接拦=用户当场可见拼错）
            let normalized = match arg {
                "default" => "default",
                "acceptEdits" | "accept_edits" => "accept_edits",
                "plan" => "plan",
                "bypass" => "bypass",
                other => {
                    return Ok(CommandOutput::error(format!(
                        "未知模式「{other}」。可选：default | acceptEdits | plan | bypass"
                    )))
                }
            };
            service.set_mode(Some(normalized.into()));
            let feedback = match normalized {
                "default" => "已切换 default 模式：工作区内写直接放行，区外写/执行命令请求审批",
                "accept_edits" => "已切换 acceptEdits 模式：编辑放行（区内写免审），执行命令仍请求审批",
                "plan" => "已切换 plan 模式：只读调研，写操作与命令将请求审批",
                _ => "已切换 bypass 模式：不询问直接放行（deny 规则与高危拦截仍生效）",
            };
            Ok(CommandOutput::success(feedback))
        }) as BoxFut<'static, _>
    })
}
