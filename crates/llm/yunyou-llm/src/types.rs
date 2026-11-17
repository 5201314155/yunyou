//! 消息、内容块与流增量类型（对标 TS 版 message.ts/content.ts 的最小裁剪，方案 §2.5）。
//!
//! 一期裁剪边界：纯文本消息 + 工具调用；图片/结构化内容块、多路由、重试策略
//! 等推迟到出现需求时再扩（保持契约面最小 = API 稳定性好维护）。

use serde::{Deserialize, Serialize};

/// 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// 工具结果回传（OpenAI 兼容的 role:"tool"）。
    Tool,
}

/// 一条对话消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// 文本内容（assistant 中间态可为空——纯工具调用消息）。
    #[serde(default)]
    pub content: String,
    /// assistant 消息携带的模型发起的工具调用。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRequest>,
    /// role=Tool 时对应的调用 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// 推理模型的思考内容（reasoning_content，实测端点字段；不回传给端点）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

impl Message {
    /// 构造纯文本消息。
    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self { role, content: content.into(), tool_calls: Vec::new(), tool_call_id: None, reasoning: None }
    }

    /// 构造工具结果消息。
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: content.into(), tool_calls: Vec::new(), tool_call_id: Some(call_id.into()), reasoning: None }
    }
}

/// 模型发起的一次工具调用（聚合完成后形态）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    /// 端点分配的调用 id（回传 tool 结果时必须一致）。
    pub id: String,
    /// 工具名。
    pub name: String,
    /// 参数 JSON 文本（增量拼接后的完整串；解析失败由调用方归为工具错误）。
    pub arguments: String,
}

/// 工具声明的模型可见形态（name/description/parameters 三元组，
/// 对标 TS 的 ToolSchema 白名单——超时等元数据绝不外发，contracts.ts:96-100）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// 参数 JSON Schema（draft 2020-12）。
    pub parameters: serde_json::Value,
}

/// 生成请求（一期字段面；temperature 为 task-33 加法演进）。
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    /// 模型名（如 `deepseek-chat`；Provider 也可用配置默认值覆盖空值）。
    pub model: String,
    /// 完整消息序列（system + 历史 + 当前输入）。
    pub messages: Vec<Message>,
    /// 可声明的工具集（空 = 不启用工具调用）。
    pub tools: Vec<ToolSchema>,
    /// 采样温度（task-33 加法演进；None = Provider 不传该字段=端点默认值，
    /// 请求体与现状逐字节一致——None 语义由各 Provider 映射层承担，本结构
    /// 非 serde wire 形态）。用途：摘要类请求显式传 0.0（忠实不创造——
    /// compaction 路径；设计稿 §12 伴注）。
    pub temperature: Option<f64>,
}

/// 流式增量。
#[derive(Debug, Clone)]
pub enum StreamDelta {
    /// 助手可见文本增量。
    Text(String),
    /// 推理模型思考增量（reasoning_content；不进入发给端点的消息体）。
    Reasoning(String),
    /// 工具调用增量（按 index 聚合）。
    ToolCall(ToolCallDelta),
    /// 终止块：携带结束原因与用量（可选）。
    Done {
        /// 如 `stop` / `tool_calls` / `length`。
        finish_reason: Option<String>,
        usage: Option<Usage>,
    },
}

/// 工具调用的流式增量（按 `index` 分片聚合为 [`ToolCallRequest`]）。
#[derive(Debug, Clone)]
pub struct ToolCallDelta {
    pub index: usize,
    /// 首个分片携带。
    pub id: Option<String>,
    /// 首个分片携带。
    pub name: Option<String>,
    /// 参数 JSON 文本分片。
    pub arguments_delta: String,
}

/// Token 用量。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 消息序列化必须稳定（会话 JSONL 兼容的基础）。
    #[test]
    fn 消息序列化往返() {
        let msg = Message::tool_result("call-1", "42");
        let text = serde_json::to_string(&msg).unwrap();
        let round: Message = serde_json::from_str(&text).unwrap();
        let text2 = serde_json::to_string(&round).unwrap();
        assert_eq!(text, text2, "序列化应稳定");
        assert!(text.contains("call-1"));
    }

    /// 空字段不落 JSON（JSONL 行保持紧凑）。
    #[test]
    fn 空字段省略() {
        let msg = Message::text(Role::User, "hi");
        let text = serde_json::to_string(&msg).unwrap();
        assert!(!text.contains("tool_calls") && !text.contains("reasoning"));
    }
}
