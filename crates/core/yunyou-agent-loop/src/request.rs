//! 请求组装（task-15 拆分件：turn.rs 行数回 400 内的家规四拆分）。
//!
//! 会话事件 → 模型请求消息序列（system prompt + 历史）；纯映射零 IO。
//! 思考内容不回传端点（容忍清单：reasoning_content 仅落盘）。

use yunyou_llm::{GenerateRequest, Message, Role};
use yunyou_session::SessionEvent;
use yunyou_system_prompt::SystemPrompt;
use yunyou_tools::RegistryService;

/// 组装一轮请求：system prompt（人设+工具声明）+ 会话历史映射。
///
/// 上下文压缩分割（task-27 设计稿 §5）：history 含 Compaction 事件 → **最后
/// 一个**之前的对话事件不进上下文（其内容由摘要承载），摘要以 User role+
/// 「【此前对话摘要（自动压缩）】」前缀注入（云枢裁审定稿：对话内容域非指令
/// 域；仅请求期注入，摘要住在 Compaction.summary 里绝不双落盘）；Compaction
/// 事件自身不进 messages。多轮链：只认最后一个，前段全丢含更早 compaction
///（摘要天然链式包含）。
pub(crate) fn compose_request(
    tools: &dyn RegistryService,
    prompt: &dyn SystemPrompt,
    persona: &str,
    history: &[SessionEvent],
) -> GenerateRequest {
    let (compaction_summary, effective) = match history
        .iter()
        .rposition(|event| matches!(event, SessionEvent::Compaction { .. }))
    {
        Some(pos) => {
            let Some(SessionEvent::Compaction { summary, .. }) = history.get(pos) else {
                unreachable!("rposition 命中即 Compaction");
            };
            (Some(summary.as_str()), &history[pos + 1..])
        }
        None => (None, history),
    };
    let tool_schemas = tools.schemas();
    let system = prompt.compose(persona, &tool_schemas);
    let mut messages = vec![Message::text(Role::System, system)];
    if let Some(summary) = compaction_summary {
        messages.push(Message::text(
            Role::User,
            format!("【此前对话摘要（自动压缩）】\n{summary}"),
        ));
    }
    for event in effective {
        match event {
            SessionEvent::UserMessage { text } => {
                messages.push(Message::text(Role::User, text.clone()))
            }
            SessionEvent::AssistantMessage { text, .. } => {
                // 思考内容不回传端点（容忍清单：reasoning_content 仅落盘）
                messages.push(Message::text(Role::Assistant, text.clone()));
            }
            SessionEvent::ToolCall { call_id, name, arguments, .. } => {
                // L-10（五席评审 20261117）：畸形 arguments 归一为 "{}" 再回发——
                // 原样回发非法 JSON 会被严格端点 400（ClientError 不重试），
                // 会话从此无法推进。归一只动回发面，落盘记录保持原样（事实源
                // 不改写）；该调用的执行结果已是错误 ToolResult（turn_steps
                // 执行侧人话指路）或 L-3 合成兜底，模型看得到并会修正重发。
                let arguments = if serde_json::from_str::<serde_json::Value>(arguments).is_ok() {
                    arguments.clone()
                } else {
                    "{}".to_string()
                };
                let mut msg = Message::text(Role::Assistant, String::new());
                msg.tool_calls = vec![yunyou_llm::ToolCallRequest {
                    id: call_id.clone(),
                    name: name.clone(),
                    arguments,
                }];
                messages.push(msg);
            }
            SessionEvent::ToolResult { call_id, content, .. } => {
                messages.push(Message::tool_result(call_id.clone(), content.clone()))
            }
            SessionEvent::SystemMessage { text } => {
                messages.push(Message::text(Role::System, text.clone()))
            }
            // ignorable 兜底：命令日志事件（command-run/command-done，纯审计配对）
            // 及未来新增词汇不进模型上下文——组 prompt 只消费对话与工具词汇
            _ => {}
        }
    }
    GenerateRequest { model: String::new(), messages, tools: tool_schemas, temperature: None }
}
