//! WS 协议词汇（页面 ⇆ 服务端；serde tag=type kebab-case，与 SessionEvent 同款
//! 词汇纪律）。
//!
//! 服务端词汇与 TurnObserver 七事件一一对应——web 与 CLI 渲染同源的唯一事实；
//! command-output/error/bye 是 web 壳层自有词汇（命令结算文案复用命令缝输出，
//! P3 接线；/quit 断开语义见任务书裁决）。

use serde::{Deserialize, Serialize};

/// 服务端 → 页面。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerMessage {
    /// 连接建立（模型名进顶栏；P4 在其后回放会话历史尾部）。
    Hello { model: String },
    /// 用户输入已受理并落盘（agent-loop UserMessage 事件——提交方不本地回显，
    /// 全部连接统一走事件流渲染，所见即所存）。
    UserMessage { text: String },
    /// 助手可见文本增量（流式过程态；完整文本由 assistant-message 终态冻结）。
    AssistantDelta { text: String },
    /// 思考增量（页面收敛提示：折叠条 + 字数计数，与 CLI Collapse 模式同口径）。
    ReasoningDelta { text: String },
    /// 工具调用发起（spinner 卡；树归属字段随事件透传，一期恒 None 平铺）。
    ToolCall {
        call_id: String,
        name: String,
        /// 参数摘要（与 CLI 同源：agent-loop 已截断至 120 字符）。
        arguments_summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        batch_id: Option<String>,
    },
    /// 工具结果（完成卡落定；content=工具 canonical value 原样，详情折叠展示，
    /// 完整无损版在会话 JSONL——与 CLI 同纪律）。
    ToolResult {
        call_id: String,
        name: String,
        summary: String,
        content: serde_json::Value,
        exit_code: Option<i64>,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        batch_id: Option<String>,
    },
    /// 助手消息落盘终态（页面用它冻结流式块，防增量漏字）。
    AssistantMessage {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },
    /// 回合完结（落款行数据；口径与 CLI 规范 §4.12 一致：耗时/↑↓/模型名）。
    TurnComplete {
        elapsed_ms: u64,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        model: String,
        tool_calls: usize,
        truncated: bool,
    },
    /// 会话列表（P4：sessions/*.jsonl，按修改时间倒序）。
    SessionList { sessions: Vec<HistoryEntry> },
    /// 历史回放（P4：连接自动回放当前会话尾部；按名回放指定会话）。
    /// 事件=SessionEvent 原样（与 JSONL 同词汇——前端渲染器共用一套视觉块）。
    History {
        session_id: String,
        events: Vec<yunyou_session::SessionEvent>,
        /// true=超出尾部上限有截断（完整无损在 JSONL）。
        truncated: bool,
    },
    /// 斜杠命令结算文本（ok=成功 / 失败红；P3 接命令缝——词汇表一次定稿，
    /// 页面分派已就位，允许暂未构造）。
    #[allow(dead_code)]
    CommandOutput { text: String, ok: bool },
    /// 壳层错误（服务未就绪/回合失败/并发拒绝/输入格式）。
    Error { text: String },
    /// /quit：断开本连接（web 语义裁决——不杀服务，其他连接不受影响）。
    Bye { text: String },
}

/// 页面 → 服务端。
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientMessage {
    /// 提交一行（消息或斜杠命令，服务端按首字符分派——与 REPL 同规则）。
    Submit { text: String },
    /// 请求会话列表（P4）。
    ListSessions,
    /// 请求历史回放：None=当前会话（连接时服务端自动推），Some(id)=指定会话。
    ReplaySession { id: Option<String> },
}

/// 会话条目（列表用；id=文件词干，modified-ms=最后写入时刻）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryEntry {
    pub id: String,
    pub modified_ms: u64,
    pub size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tag 形态冻结：词汇名 kebab-case；None 可选字段省略（载荷最小化）。
    #[test]
    fn 服务端词汇tag形态与可选省略() {
        let msg = ServerMessage::AssistantDelta { text: "你".into() };
        let text = serde_json::to_string(&msg).unwrap();
        assert_eq!(text, r#"{"type":"assistant-delta","text":"你"}"#);

        let msg = ServerMessage::ToolResult {
            call_id: "c1".into(),
            name: "bash".into(),
            summary: "6 项".into(),
            content: serde_json::json!({ "exit_code": 0 }),
            exit_code: Some(0),
            is_error: false,
            parent_tool_call_id: None,
            batch_id: None,
        };
        let text = serde_json::to_string(&msg).unwrap();
        assert!(text.contains(r#""type":"tool-result""#), "{text}");
        assert!(!text.contains("parent_tool_call_id"), "None 字段应省略：{text}");
    }

    /// 入站词汇解析 + 未知 type 拒绝（fail-visible 不静默）。
    #[test]
    fn 入站词汇与未知type() {
        let m: ClientMessage = serde_json::from_str(r#"{"type":"submit","text":"你好"}"#).unwrap();
        assert!(matches!(m, ClientMessage::Submit { text } if text == "你好"));
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"nope"}"#).is_err());
    }
}
