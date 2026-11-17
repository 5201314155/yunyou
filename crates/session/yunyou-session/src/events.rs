//! 会话事件模型（一期五类对话事件+两类命令日志事件；对标 TS known-event-types 的裁剪词汇表）。

use serde::{Deserialize, Serialize};

/// 会话事件（持久化词汇）。
///
/// 序列化形态：`{"type":"user-message", ...}`——tag 字段 `type` 供 JSONL
/// 读取端识别未知类型并跳过（crate 文档的 ignorable 机制）。
///
/// 词汇分组：`user/assistant/tool_call/tool_result/system` 五类进模型上下文组装；
/// `command_run/command_done`（任务29 D2）是**纯日志配对**（回放/审计用），
/// 不进模型上下文——组 prompt 的消费端对未知类型按 ignorable 跳过。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SessionEvent {
    /// 用户输入。
    UserMessage { text: String },
    /// 助手回复（含思考内容时一并保留；思考内容不回传端点）。
    AssistantMessage {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },
    /// 模型发起的工具调用（聚合完成后落盘）。
    ToolCall {
        call_id: String,
        name: String,
        /// 参数 JSON 文本。
        arguments: String,
        /// 并行工具调用树状归属：本调用由哪个外层工具调用发起（顶层为 None）。
        /// 可选字段（云枢裁定 2026-11-13「加」）；旧日志缺失时读取端降级平铺。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
        /// 同批并行调用的批次标识（同批共享一个 batch_id；顺序执行时为 None）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch_id: Option<String>,
    },
    /// 工具执行结果。
    ToolResult {
        call_id: String,
        name: String,
        /// 结果内容（渲染后的文本）。
        content: String,
        /// 执行是否失败（错误与成功结果在会话里形态不同）。
        is_error: bool,
        /// 树状归属，语义同 [`SessionEvent::ToolCall::parent_tool_call_id`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
        /// 批次标识，语义同 [`SessionEvent::ToolCall::batch_id`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch_id: Option<String>,
    },
    /// 系统注入消息（错误说明、上下文提示等）。
    SystemMessage { text: String },
    /// 上下文压缩标记（task-27 一期）：摘要式压缩的持久化锚点——本事件
    /// **之前**的全部对话事件不再进模型上下文（由 summary 承载），**之后**
    /// 的事件正常进。物理上只追加（JSONL 回放语义零破坏）；旧宿主读本事件
    /// =UnknownType 跳过（ignorable 实测路径），行为退化为继续全量喂、不炸。
    /// 多轮压缩链：build_request 只认最后一个（前段全丢含更早 compaction，
    /// 摘要天然链式包含）。
    Compaction {
        /// 压缩摘要（LLM 生成；承载被压缩段的全部关键状态）。
        summary: String,
        /// 压缩前水位（该次压缩时的上下文 token 数，观测与验收对账用）。
        pre_tokens: u64,
    },
    /// 斜杠命令开始执行（任务29 D2）：run 先于 handler 落盘，
    /// 与 [`SessionEvent::CommandDone`] 以 `command_id` 配对（tool_call/tool_result 同纪律）。
    CommandRun {
        /// 配对键：每次命令调用铸造的 UUID。
        command_id: String,
        /// 命令名（不带斜杠）。
        name: String,
        /// 用户原始输入行（一期恒记录；含敏感的命令属二期，届时加开关——设计 §五）。
        raw_input: String,
    },
    /// 斜杠命令执行结算（任务29 D2）：done 后于 handler 结算落盘；
    /// thrown/aborted 结算为 `CommandOutcome::Error`。
    CommandDone {
        /// 配对键：与同次调用的 [`SessionEvent::CommandRun::command_id`] 一致。
        command_id: String,
        /// 结算结果。
        kind: CommandOutcome,
        /// 结算文本：动作类=上屏文本；展开类=回填文本；失败=错误说明。
        text: String,
    },
    /// 自动续跑标记（K-16②，设计案 §3.2）：重启检测命中→自动「继续」时落盘；
    /// round 递增计数=同一次未完回合的续跑轮次（≥上限=升级不自动续）。
    /// **不进回放**（§9 裁决②）；**ignorable 兼容**：旧读取器见未知 type 跳过。
    ResumeMark {
        /// 续跑轮次（同一未完回合第 N 次自动续跑；从 1 起）。
        round: u32,
        /// 中断原因人话（软着陆中断/工具执行中断/孤儿输入）。
        reason: String,
    },
}

/// 命令结算结果（任务29 D2；与 tool_result 的 is_error 配对纪律同构）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandOutcome {
    /// 成功。
    Success,
    /// 失败。
    Error,
}

impl SessionEvent {
    /// 展示名（日志与调试用）。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UserMessage { .. } => "user-message",
            Self::AssistantMessage { .. } => "assistant-message",
            Self::ToolCall { .. } => "tool-call",
            Self::ToolResult { .. } => "tool-result",
            Self::SystemMessage { .. } => "system-message",
            Self::CommandRun { .. } => "command-run",
            Self::CommandDone { .. } => "command-done",
            Self::ResumeMark { .. } => "resume-mark",
            Self::Compaction { .. } => "compaction",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tag 序列化形态与往返。
    #[test]
    fn 事件往返与tag形态() {
        let event = SessionEvent::ToolCall {
            call_id: "c1".into(),
            name: "bash".into(),
            arguments: "{\"cmd\":\"ls\"}".into(),
            parent_tool_call_id: None,
            batch_id: None,
        };
        let text = serde_json::to_string(&event).unwrap();
        assert!(text.starts_with("{\"type\":\"tool-call\""), "tag 形态: {text}");
        let back: SessionEvent = serde_json::from_str(&text).unwrap();
        assert_eq!(back, event);
    }

    /// 命令日志事件（任务29 D2）：tag 形态冻结+配对字段往返+CommandOutcome 序列化。
    #[test]
    fn 命令事件往返与配对字段() {
        let run = SessionEvent::CommandRun {
            command_id: "u1".into(),
            name: "help".into(),
            raw_input: "/帮助".into(),
        };
        let done = SessionEvent::CommandDone {
            command_id: "u1".into(),
            kind: CommandOutcome::Success,
            text: "命令列表".into(),
        };
        let run_text = serde_json::to_string(&run).unwrap();
        let done_text = serde_json::to_string(&done).unwrap();
        assert!(run_text.starts_with("{\"type\":\"command-run\""), "run tag 形态: {run_text}");
        assert!(done_text.starts_with("{\"type\":\"command-done\""), "done tag 形态: {done_text}");
        assert!(done_text.contains("\"kind\":\"success\""), "outcome 序列化形态: {done_text}");
        let run_back: SessionEvent = serde_json::from_str(&run_text).unwrap();
        let done_back: SessionEvent = serde_json::from_str(&done_text).unwrap();
        assert_eq!(run_back, run);
        assert_eq!(done_back, done);
        // 配对键同值：run/done 靠 command_id 关联（一次调用一次铸造）
        let (run_id, done_id) = match (&run_back, &done_back) {
            (
                SessionEvent::CommandRun { command_id, .. },
                SessionEvent::CommandDone { command_id: done_key, .. },
            ) => (command_id.clone(), done_key.clone()),
            _ => panic!("变体还原错误"),
        };
        assert_eq!(run_id, done_id, "配对键必须一致");
        assert_eq!(run.kind(), "command-run");
        assert_eq!(done.kind(), "command-done");
    }
}
