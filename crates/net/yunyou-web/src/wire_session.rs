//! 每会话 wire 状态机（多会话步 1，自 wire.rs 拆出）：单条会话的 13 词汇
//! [`ServerMessage`] → 对端 SessionWireEvent / assistantStream 帧转换。
//!
//! # 职责分家
//! - 本文件：**一条会话**的全部状态（seq/turn/revision 计数、attempt、重放环、
//!   title 投影、requestId echo）与帧合成——多会话后每条会话各自致密；
//! - 容器与编排在 [`crate::wire::WireState`]（active 路由/工作区/基线聚合）；
//! - 共享外壳（单例/订阅/泵）在 [`crate::bridge`]。
//!
//! # 协议铁律（不变，逐会话成立）
//! 持久事件 seq 致密 +1；surface 事件必带 `surfaceOp:'append'`；
//! assistantStream `revision` 逐帧 +1；chunk `index` 致密；end 帧 committed
//! seq 指向已推的 assistant/message；user/message `source.rpcId` 用后即清。

use crate::protocol::ServerMessage;
use crate::wire_frames::{derive_title, now_ms, Attempt};
use serde_json::{json, Value};
use std::collections::VecDeque;

pub(crate) struct SessionWire {
    /// 会话 id（None=未铸——投影/列表不猜 id，不伪造块）。
    pub(crate) session_id: Option<String>,
    /// assistant message source 落款（帧原语跨模块消费，字段 crate 内可见）。
    pub(crate) model: String,
    pub(crate) next_seq: u64,
    pub(crate) turn: u64,
    pub(crate) turn_open: bool,
    pub(crate) revision: u64,
    /// prompt requestId（echo 对账；下一条 user/message 消费后即清）。
    pub(crate) request_id: Option<String>,
    /// 会话标题（首条用户消息派生）。
    pub(crate) title: Option<String>,
    /// 最近一条用户消息落款时刻（sessionListMetadata.lastPromptAt）。
    pub(crate) last_prompt_at: Option<u64>,
    pub(crate) attempt: Option<Attempt>,
    pub(crate) history: VecDeque<Value>,
    /// 历史灌入时跳过的未桥接词汇事件数（快照 skippedEvents 可见——诚实）。
    pub(crate) skipped_events: u64,
}

/// apply 产出：会话帧（follow 流）+ 控制投影帧（control 流）。
pub(crate) struct WireOut {
    pub(crate) session: Vec<Value>,
    pub(crate) control: Vec<Value>,
}

impl SessionWire {
    /// 构造（model=assistant message source 落款；session_id 可后铸）。
    pub(crate) fn new(model: String, session_id: Option<String>) -> Self {
        Self {
            session_id,
            model,
            next_seq: 0,
            turn: 0,
            turn_open: false,
            revision: 0,
            request_id: None,
            title: None,
            last_prompt_at: None,
            attempt: None,
            history: VecDeque::new(),
            skipped_events: 0,
        }
    }

    /// 单条 ServerMessage → 0..n 会话帧 + 0..n 控制投影帧（顺序=持久事件与
    /// 流式帧按对端状态机交错）。
    pub(crate) fn apply(&mut self, msg: &ServerMessage) -> WireOut {
        let mut out = WireOut { session: Vec::new(), control: Vec::new() };
        match msg {
            ServerMessage::UserMessage { text } => self.on_user_message(text, &mut out),
            ServerMessage::AssistantDelta { text } => out.session = self.on_chunk(text, "text-delta"),
            ServerMessage::ReasoningDelta { text } => {
                out.session = self.on_chunk(text, "reasoning-delta")
            }
            ServerMessage::ToolCall { call_id, name, arguments_summary, .. } => {
                let (turn, step) = self.current_place();
                out.session = vec![self.emit("tool/call", json!({
                    "turn": turn, "step": step,
                    "callId": call_id, "name": name,
                    "arguments": arguments_summary,
                }), false)];
            }
            ServerMessage::ToolResult { call_id, content, is_error, .. } => {
                let (turn, step) = self.current_place();
                let content_text = match content {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                out.session = vec![self.emit("tool/result", json!({
                    "turn": turn, "step": step,
                    "message": {
                        "id": uuid::Uuid::new_v4().to_string(),
                        "role": "tool",
                        "content": [{"type": "text", "text": content_text}],
                        "source": {"kind": "tool", "callId": call_id},
                        "toolCallId": call_id,
                        "isError": is_error,
                    },
                }), true)];
            }
            ServerMessage::AssistantMessage { text, reasoning } => {
                out.session = self.on_assistant_message(text, reasoning.as_deref())
            }
            ServerMessage::TurnComplete { .. } => out.session = self.on_turn_complete(),
            ServerMessage::Error { text } => out.session = self.on_error(text),
            // 控制面词汇不进 follow 流（hello→连接状态；session-list→unary；
            // history→v1 页面词汇；bye/command-output→P3 命令缝）
            _ => {}
        }
        out
    }

    /// 用户消息：持久 user/message（append，source 带 rpcId echo）+ 回合/
    /// 步骤开闸事件 + attempt start 帧 + 首名标题投影控制帧。
    fn on_user_message(&mut self, text: &str, out: &mut WireOut) {
        let (turn, step) = self.open_turn();
        let mut source = json!({"kind": "user"});
        if let Some(request_id) = self.request_id.take() {
            source["rpcId"] = Value::String(request_id);
        }
        // 标题派生：会话首名=首条用户消息（首行截 40 字）；随后推控制投影帧。
        let titled = self.title.is_none();
        if titled {
            self.title = Some(derive_title(text));
        }
        self.last_prompt_at = Some(now_ms());
        let user_event = self.emit("user/message", json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "role": "user",
            "content": [{"type": "text", "text": text}],
            "source": source,
        }), true);
        let event_seq = user_event["event"]["seq"].as_i64().unwrap_or(0);
        out.session.push(user_event);
        out.session.push(self.emit("turn/start", json!({"turn": turn}), false));
        out.session.push(self.emit("step/start", json!({"turn": turn, "step": step}), false));
        // attempt 字段先取值再合成帧（开闸借用与 revision 递增借用不相交）
        let (attempt_id, started_after_seq) = self.open_attempt(turn, step);
        let revision = self.bump_revision();
        out.session.push(json!({"type": "assistant-stream", "frame": {
            "type": "start",
            "attemptId": attempt_id,
            "revision": revision,
            "startedAfterSeq": started_after_seq,
            "turn": turn,
            "step": step,
        }}));
        // 控制投影帧（侧栏真名 live 更新）：seq 与 user/message 事件对齐。
        let mut changes = vec![json!({"key": "sessionListMetadata", "val": {
            "blank": false, "lastPromptAt": self.last_prompt_at,
        }})];
        if titled {
            changes.insert(0, json!({"key": "title", "val": self.title}));
        }
        if let Some(id) = self.session_id.as_deref().filter(|id| !id.is_empty()) {
            out.control.push(json!({
                "type": "projection", "sessionId": id, "seq": event_seq, "changes": changes,
            }));
        }
    }

    /// 流式增量：chunk 通知帧（index 致密，revision 逐帧 +1，重放记录同步积累）。
    fn on_chunk(&mut self, text: &str, delta_type: &str) -> Vec<Value> {
        // 状态变更在 attempt 借用内完成，帧合成在借用结束后（字段先行取出）
        let Some((attempt_id, index, chunk)) = (|s: &mut Self| {
            let attempt = s.attempt.as_mut()?;
            let chunk = json!({"type": delta_type, "index": 0, "text": text});
            attempt.stream.push(json!({"type": "chunk", "time": now_ms(), "chunk": chunk}));
            let frame = (attempt.attempt_id.clone(), attempt.next_index, chunk);
            attempt.next_index += 1;
            Some(frame)
        })(self)
        else {
            return Vec::new();
        };
        let revision = self.bump_revision();
        vec![json!({"type": "assistant-stream", "frame": {
            "type": "chunk",
            "attemptId": attempt_id,
            "revision": revision,
            "index": index,
            "time": now_ms(),
            "chunk": chunk,
        }})]
    }

    /// 助手消息落定：持久 assistant/message + attempt end 帧（committed 指向
    /// 该事件 seq）。
    fn on_assistant_message(&mut self, text: &str, reasoning: Option<&str>) -> Vec<Value> {
        let (turn, step) = self.current_place();
        let mut blocks = vec![json!({"type": "text", "text": text})];
        if let Some(reasoning) = reasoning.filter(|r| !r.is_empty()) {
            blocks.push(json!({"type": "reasoning", "text": reasoning}));
        }
        let event_seq = self.next_seq;
        let message_event = self.emit("assistant/message", json!({
            "turn": turn, "step": step,
            "message": {
                "id": uuid::Uuid::new_v4().to_string(),
                "role": "assistant",
                "content": blocks,
                "source": {"kind": "model", "provider": "yunyou", "model": self.model},
            },
            "stream": [],
        }), true);
        let mut frames = vec![message_event];
        if let Some(attempt) = self.attempt.take() {
            frames.push(json!({"type": "assistant-stream", "frame": {
                "type": "end",
                "attemptId": attempt.attempt_id,
                "revision": self.bump_revision(),
                "index": attempt.next_index,
                "outcome": {"kind": "committed", "eventType": "assistant/message", "seq": event_seq},
            }}));
        }
        frames
    }

    /// 回合完结：step/end + turn/end(completed)；attempt 残留（异常序）先弃。
    fn on_turn_complete(&mut self) -> Vec<Value> {
        let mut frames = Vec::new();
        if let Some(abandoned) = self.abandon_attempt() {
            frames.push(abandoned);
        }
        let (turn, step) = self.current_place();
        frames.push(self.emit("step/end", json!({"turn": turn, "step": step}), false));
        frames.push(self.emit("turn/end", json!({
            "turn": turn, "reason": {"kind": "completed"},
        }), false));
        self.turn_open = false;
        frames
    }

    /// 错误：attempt 开→end(abandoned)；回合开→step/end+system/message+
    /// turn/end(error)；均未开→仅 system/message（fallback 渲染人话卡）。
    fn on_error(&mut self, text: &str) -> Vec<Value> {
        let mut frames = Vec::new();
        if let Some(abandoned) = self.abandon_attempt() {
            frames.push(abandoned);
        }
        if self.turn_open {
            let (turn, step) = self.current_place();
            frames.push(self.emit("step/end", json!({"turn": turn, "step": step}), false));
            frames.push(self.emit("system/message", json!({
                "turn": turn, "step": step,
                "message": {
                    "id": uuid::Uuid::new_v4().to_string(),
                    "role": "system",
                    "content": [{"type": "text", "text": text}],
                    "source": {"kind": "system"},
                },
            }), true));
            frames.push(self.emit("turn/end", json!({
                "turn": turn,
                "reason": {"kind": "error", "error": {"message": text, "code": "UNKNOWN"}},
            }), false));
            self.turn_open = false;
        } else {
            frames.push(self.emit("system/message", json!({
                "turn": self.turn + 1, "step": 0,
                "message": {
                    "id": uuid::Uuid::new_v4().to_string(),
                    "role": "system",
                    "content": [{"type": "text", "text": text}],
                    "source": {"kind": "system"},
                },
            }), true));
        }
        frames
    }
}
