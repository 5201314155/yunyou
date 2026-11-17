//! 流式增量聚合：把 StreamDelta 序列聚合为一条助手消息
//!（文本 + 思考 + 按 index 拼接的工具调用）。

use yunyou_llm::{Message, StreamDelta, ToolCallRequest, Usage};

/// 一轮模型调用的聚合结果。
#[derive(Debug, Default)]
pub struct Aggregated {
    /// 可见文本。
    pub text: String,
    /// 思考内容（reasoning_content 聚合；不回传端点，仅落盘）。
    pub reasoning: String,
    /// 聚合完成的工具调用（按 index 排序）。
    pub tool_calls: Vec<ToolCallRequest>,
    /// 用量（可选）。
    pub usage: Option<Usage>,
    /// 结束原因。
    pub finish_reason: Option<String>,
}

impl Aggregated {
    /// 从增量序列聚合。
    pub fn fold(mut self, delta: StreamDelta) -> Self {
        match delta {
            StreamDelta::Text(piece) => self.text.push_str(&piece),
            StreamDelta::Reasoning(piece) => self.reasoning.push_str(&piece),
            StreamDelta::ToolCall(call) => self.fold_tool_call(call),
            StreamDelta::Done { finish_reason, usage } => {
                self.finish_reason = finish_reason.or(self.finish_reason);
                self.usage = usage.or(self.usage);
            }
        }
        self
    }

    /// 按 index 聚合工具调用分片（id/name 首片携带，arguments 逐片拼接）。
    fn fold_tool_call(&mut self, call: yunyou_llm::ToolCallDelta) {
        while self.tool_calls.len() <= call.index {
            self.tool_calls.push(ToolCallRequest {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
        }
        let slot = &mut self.tool_calls[call.index];
        if slot.id.is_empty() {
            if let Some(id) = call.id {
                slot.id = id;
            }
        }
        if slot.name.is_empty() {
            if let Some(name) = call.name {
                slot.name = name;
            }
        }
        slot.arguments.push_str(&call.arguments_delta);
    }

    /// 是否发起了工具调用。
    pub fn has_tool_calls(&self) -> bool {
        self.tool_calls.iter().any(|call| !call.name.is_empty())
    }

    /// 空回复判定（task-49③）：正文与思考皆空且无工具调用=模型本轮零产出——
    /// 静默 Ok 会落空 assistant 装死（业主实证「任务没完成像被截断」收尾形态），
    /// 调用方应转可重试错误诚实上报。
    pub(crate) fn is_empty_response(&self) -> bool {
        self.text.trim().is_empty() && self.reasoning.is_empty() && self.tool_calls.is_empty()
    }

    /// 转为 assistant 消息（工具调用非空时携带；content 为空且无调用时保留空串）。
    pub fn into_message(self) -> Message {
        Message {
            role: yunyou_llm::Role::Assistant,
            content: self.text,
            tool_calls: self.tool_calls,
            tool_call_id: None,
            reasoning: if self.reasoning.is_empty() { None } else { Some(self.reasoning) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_llm::ToolCallDelta;

    /// 分片乱序到达也能按 index 聚合（端点可能交错）。
    #[test]
    fn 工具调用按index聚合() {
        let aggregated = Aggregated::default()
            .fold(StreamDelta::Text("我来查一下。".into()))
            .fold(StreamDelta::ToolCall(ToolCallDelta {
                index: 1,
                id: Some("c2".into()),
                name: Some("read".into()),
                arguments_delta: "{\"path\":\"b\"".into(),
            }))
            .fold(StreamDelta::ToolCall(ToolCallDelta {
                index: 0,
                id: Some("c1".into()),
                name: Some("bash".into()),
                arguments_delta: "{\"cmd\":\"ls\"".into(),
            }))
            .fold(StreamDelta::ToolCall(ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments_delta: "}".into(),
            }))
            .fold(StreamDelta::Done { finish_reason: Some("tool_calls".into()), usage: None });
        assert_eq!(aggregated.text, "我来查一下。");
        assert_eq!(aggregated.tool_calls.len(), 2);
        assert_eq!(aggregated.tool_calls[0].name, "bash");
        assert_eq!(aggregated.tool_calls[0].arguments, "{\"cmd\":\"ls\"}");
        assert_eq!(aggregated.tool_calls[1].id, "c2");
        assert!(aggregated.has_tool_calls());
    }
}
