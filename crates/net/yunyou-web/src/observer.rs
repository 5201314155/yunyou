//! 广播观察者：TurnObserver → WS 广播通道。
//!
//! web 是 CLI 渲染的**同源第二消费端**：同一 agent-loop 事件流、同一七事件
//! 词汇——映射只做词汇翻译（TurnEvent → ServerMessage），不造新事件、不改
//! 时序。CLI 侧跳过的 UserMessage 这里不跳（CLI 靠 stdin 回显天然承担，
//! web 提交方不本地回显，统一走事件渲染，所见即所存）。
//!
//! 契约对齐（yunyou-agent-loop observer.rs）：on_event 被同步 await——广播
//! send 非阻塞（容量积压时订阅端收 Lagged，由连接层人话提示），快速返回
//! 不拖慢事件流；落款耗时由观察者自记（Instant 起点=回合开始，CLI 同口径）。

use crate::protocol::ServerMessage;
use std::time::Instant;
use tokio::sync::broadcast;
use yunyou_agent_loop::{TurnEvent, TurnObserver};
use yunyou_util::BoxFut;

/// 事件 → WS 广播的观察者（每回合构造一个：计时起点即回合起点）。
pub struct WebObserver {
    sender: broadcast::Sender<ServerMessage>,
    model: String,
    started: Instant,
}

impl WebObserver {
    /// 构造：广播发送端 + 模型名（落款用）。
    pub fn new(sender: broadcast::Sender<ServerMessage>, model: String) -> Self {
        Self { sender, model, started: Instant::now() }
    }

    /// 广播一条（无订阅者=无人看，安全丢弃——对齐 ChannelObserver 纪律）。
    fn emit(&self, msg: ServerMessage) {
        let _ = self.sender.send(msg);
    }
}

impl TurnObserver for WebObserver {
    fn on_event(&self, event: TurnEvent) -> BoxFut<'_, ()> {
        match event {
            TurnEvent::UserMessage { text } => self.emit(ServerMessage::UserMessage { text }),
            TurnEvent::AssistantDelta { text } => self.emit(ServerMessage::AssistantDelta { text }),
            TurnEvent::ReasoningDelta { text } => self.emit(ServerMessage::ReasoningDelta { text }),
            TurnEvent::ToolCall { call_id, name, arguments_summary, parent_tool_call_id, batch_id } => {
                self.emit(ServerMessage::ToolCall {
                    call_id,
                    name,
                    arguments_summary,
                    parent_tool_call_id,
                    batch_id,
                });
            }
            TurnEvent::ToolResult {
                call_id,
                name,
                summary,
                content,
                exit_code,
                is_error,
                parent_tool_call_id,
                batch_id,
            } => {
                self.emit(ServerMessage::ToolResult {
                    call_id,
                    name,
                    summary,
                    content,
                    exit_code,
                    is_error,
                    parent_tool_call_id,
                    batch_id,
                });
            }
            TurnEvent::AssistantMessage { text, reasoning } => {
                self.emit(ServerMessage::AssistantMessage { text, reasoning });
            }
            TurnEvent::TurnComplete { outcome, prompt_tokens, completion_tokens, .. } => {
                self.emit(ServerMessage::TurnComplete {
                    elapsed_ms: self.started.elapsed().as_millis() as u64,
                    prompt_tokens,
                    completion_tokens,
                    model: self.model.clone(),
                    tool_calls: outcome.tool_calls,
                    truncated: outcome.truncated,
                });
            }
            // task-27 压缩完成（加法演进）：web 展示面候适配层批（同 TurnPaused
            // 挂账口径——文案在渲染域，此处显式忽略保编译）。
            TurnEvent::Compacted { .. } => {}
            // task-23 软着陆暂停（结构化事件加法演进）：web 展示面候适配层
            // 批（云屏 WIP 域，文案展示与「已截断」旧文案联动同批）——此处
            // 显式忽略保编译，文案已在落盘 SystemMessage（会话事实源）。
            TurnEvent::TurnPaused { .. } => {}
        }
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_agent_loop::TurnOutcome;

    /// 七事件映射同源性：TurnEvent → ServerMessage 词汇一一对应、时序保序、
    /// 落款字段（模型名/耗时/步数/截断）不丢——web 与 CLI 同源契约的锁定测试。
    #[tokio::test]
    async fn 七事件映射保序且落款字段不丢() {
        let (sender, mut rx) = broadcast::channel(16);
        let observer = WebObserver::new(sender, "z-ai/glm-5.3-flash".into());
        let events = vec![
            TurnEvent::UserMessage { text: "看看 fiber".into() },
            TurnEvent::ReasoningDelta { text: "想…".into() },
            TurnEvent::AssistantDelta { text: "我来".into() },
            TurnEvent::ToolCall {
                call_id: "c1".into(),
                name: "read".into(),
                arguments_summary: "fiber.rs".into(),
                parent_tool_call_id: None,
                batch_id: None,
            },
            TurnEvent::ToolResult {
                call_id: "c1".into(),
                name: "read".into(),
                summary: "26 行".into(),
                content: serde_json::json!({ "lines": 26 }),
                exit_code: None,
                is_error: false,
                parent_tool_call_id: None,
                batch_id: None,
            },
            TurnEvent::AssistantMessage { text: "我看完了".into(), reasoning: Some("想…".into()) },
            TurnEvent::TurnComplete {
                outcome: TurnOutcome { assistant_text: "我看完了".into(), tool_calls: 1, truncated: false, auto_resume: false, wind_down: false },
                prompt_tokens: Some(1204),
                completion_tokens: Some(8931),
                context_tokens: Some(1204), // task-26：水位=最后一步 prompt（此处单步=同值）
                context_window: None, // task-27：窗口未配→落款无百分比
            },
        ];
        for event in events {
            observer.on_event(event).await;
        }
        let mut kinds = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            kinds.push(serde_json::to_value(&msg).unwrap()["type"].clone());
        }
        assert_eq!(
            kinds,
            vec![
                "user-message",       // web 不跳过（CLI 靠 stdin 回显）
                "reasoning-delta",
                "assistant-delta",
                "tool-call",
                "tool-result",
                "assistant-message",
                "turn-complete",
            ],
            "七事件按序映射"
        );
        // 落款字段抽查（TurnComplete）：模型名/步数/截断/None→null/耗时必带
        let (sender2, mut rx2) = broadcast::channel(4);
        let obs2 = WebObserver::new(sender2, "z-ai/glm-5.3-flash".into());
        obs2.on_event(TurnEvent::TurnComplete {
            outcome: TurnOutcome { assistant_text: String::new(), tool_calls: 3, truncated: true, auto_resume: false, wind_down: false },
            prompt_tokens: Some(1204),
            completion_tokens: None,
            context_tokens: None, // task-26：usage 未报→水位 None（诚实不估算）
            context_window: None,
        })
        .await;
        let value = serde_json::to_value(rx2.recv().await.unwrap()).unwrap();
        assert_eq!(value["model"], "z-ai/glm-5.3-flash");
        assert_eq!(value["tool_calls"], 3);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["completion_tokens"], serde_json::Value::Null);
        assert!(value["elapsed_ms"].as_u64().is_some(), "耗时必带");
    }
}
