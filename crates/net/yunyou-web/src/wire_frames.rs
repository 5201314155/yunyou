//! wire 帧原语（多会话步 1 自 wire_session.rs 拆出，行数纪律）：单条会话的
//! 计数/重放环原语（emit/attempt 生命周期/revision）与只读投影面（快照/
//! 基线块/列表行）。类型与 apply 分派在 [`crate::wire_session`]；同 crate
//! 固有 impl 跨模块合法（职责分家不拆类型）。
//!
//! 协议铁律注记见 wire_session.rs 模块头（本文件全部方法逐会话成立）。

use serde_json::{json, Value};

use crate::wire_session::SessionWire;

/// 重放环容量（快照 records 上限；超出丢最旧——完整流在会话 JSONL）。
pub(crate) const HISTORY_CAP: usize = 512;

/// 进行中的流式 attempt（对端 start-chunk-end 模型；生命周期原语在侧）。
pub(crate) struct Attempt {
    pub(crate) attempt_id: String,
    pub(crate) started_after_seq: i64,
    pub(crate) turn: u64,
    pub(crate) step: u64,
    pub(crate) next_index: u64,
    /// 已发 chunk 的 raw 记录（快照 activeAttempt.stream 重放用，保序）。
    pub(crate) stream: Vec<Value>,
}

impl SessionWire {
    /// 控制投影值表（title + sessionListMetadata；无 turn 时 title 键缺席=
    /// 对端 `session.untitled` 兜底）。
    pub(crate) fn projection_values(&self) -> Value {
        let mut values = json!({"sessionListMetadata": {
            "blank": self.turn == 0,
            "lastPromptAt": self.last_prompt_at,
        }});
        if let Some(title) = &self.title {
            values["title"] = Value::String(title.clone());
        }
        values
    }
    /// 本会话投影基线块（未铸会话=None——诚实缺省，不伪造块）。返回 (id, block)。
    pub(crate) fn baseline_block(&self) -> Option<(String, Value)> {
        let id = self.session_id.as_deref().filter(|id| !id.is_empty())?;
        Some((
            id.to_string(),
            json!({
                "asOfSeq": self.next_seq as i64 - 1,
                "values": self.projection_values(),
            }),
        ))
    }

    /// session/list 单行（SessionSummary 形状；agentAvailable 由容器按
    /// 「本会话是否为当前活跃会话」判定传入——非活跃历史行不可直接 prompt）。
    pub(crate) fn summary_row(&self, agent_available: bool) -> Option<Value> {
        let id = self.session_id.as_deref().filter(|id| !id.is_empty())?;
        Some(json!({
            "sessionId": id,
            "agentAvailable": agent_available,
            "updatedAt": now_ms(),
            "running": self.turn_open,
            "blank": self.turn == 0,
            "projections": {
                "kind": "sequenced",
                "asOfSeq": self.next_seq as i64 - 1,
                "values": self.projection_values(),
            },
        }))
    }

    /// follow 快照：header+投影基线+重放环+assistantStream 基线
    /// （transport.ts 强制要求快照必带 assistantStream，缺失即抛错）。
    pub(crate) fn snapshot(&self, address_session_id: Option<&str>) -> Value {
        let cursor = self.next_seq as i64 - 1;
        let id = address_session_id
            .filter(|id| !id.is_empty())
            .or(self.session_id.as_deref())
            .unwrap_or("default");
        let mut opening = json!({
            "type": "snapshot",
            "header": {
                "version": 1, "id": id,
                "createdAt": now_ms(), "isSeeded": false,
            },
            "projections": {"asOfSeq": cursor, "values": {}},
            "cursor": cursor,
            "records": self.history.iter().collect::<Vec<_>>(),
            "hasMore": false,
        });
        // 灌入跳过计数（加法演进可选字段：未桥接词汇事件数，诚实可见——
        // 契约 §6.1 允许新增可选字段；0=缺席不噪音）
        if self.skipped_events > 0 {
            opening["skippedEvents"] = json!(self.skipped_events);
        }
        let mut baseline = json!({"revision": self.revision});
        if let Some(attempt) = &self.attempt {
            baseline["activeAttempt"] = json!({
                "attemptId": attempt.attempt_id,
                "startedAfterSeq": attempt.started_after_seq,
                "turn": attempt.turn,
                "step": attempt.step,
                "nextIndex": attempt.next_index,
                "stream": attempt.stream,
            });
        }
        opening["assistantStream"] = baseline;
        opening
    }
    /// 开新回合：turn 计数 +1，step 恒 1（单 AssistantMessage 终态现态）。
    pub(crate) fn open_turn(&mut self) -> (u64, u64) {
        self.turn += 1;
        self.turn_open = true;
        (self.turn, 1)
    }

    /// 开新 attempt（startedAfterSeq=已分配最大 seq；落定事件 seq 严格大于它）。
    pub(crate) fn open_attempt(&mut self, turn: u64, step: u64) -> (String, i64) {
        let attempt = Attempt {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            started_after_seq: self.next_seq as i64 - 1,
            turn,
            step,
            next_index: 0,
            stream: Vec::new(),
        };
        let id = attempt.attempt_id.clone();
        let started = attempt.started_after_seq;
        self.attempt = Some(attempt);
        (id, started)
    }

    /// 弃置残留 attempt（end 帧 abandoned——客户端即弃暂态行）。
    pub(crate) fn abandon_attempt(&mut self) -> Option<Value> {
        let attempt = self.attempt.take()?;
        Some(json!({"type": "assistant-stream", "frame": {
            "type": "end",
            "attemptId": attempt.attempt_id,
            "revision": self.bump_revision(),
            "index": attempt.next_index,
            "outcome": {"kind": "abandoned"},
        }}))
    }

    /// 当前回合位置（tool 事件落款用；step 恒 1 同 open_turn）。
    pub(crate) fn current_place(&self) -> (u64, u64) {
        (self.turn, 1)
    }

    /// 分配 seq、入重放环、包 event 帧一次完成（surface=带 surfaceOp:'append'）。
    pub(crate) fn emit(&mut self, event_type: &str, data: Value, surface: bool) -> Value {
        let seq = self.next_seq;
        self.next_seq += 1;
        let mut event = json!({
            "type": event_type,
            "seq": seq,
            "time": now_ms(),
            "data": data,
        });
        if surface {
            event["surfaceOp"] = Value::String("append".into());
        }
        let entry = json!({"type": "event", "event": event});
        self.history.push_back(entry.clone());
        if self.history.len() > HISTORY_CAP {
            self.history.pop_front();
        }
        entry
    }

    /// 通知帧 revision 递增（transport.ts 逐帧 +1 铁律）。
    pub(crate) fn bump_revision(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }
}

impl SessionWire {
    /// 历史回放灌入（session/open，多会话步 2）：会话 JSONL 事件 → 对端持久
    /// 事件帧，直入重放环（seq 延续、HISTORY_CAP 封顶）。返回 (灌入数, 跳过数)。
    ///
    /// # 词汇映射（域-会话命令与技能.md §1.1-4：完整历史在 JSONL，Web 重放环
    /// 512 尾部封顶）
    /// UserMessage→user/message｜AssistantMessage→assistant/message｜
    /// ToolCall→tool/call｜ToolResult→tool/result｜SystemMessage→system/message；
    /// 其余词汇（Compaction/CommandRun/CommandDone/ResumeMark 等）**诚实跳过
    /// 并计数**（跳过数经快照可见——错误码/日志面呈现，不伪造完整重放）。
    pub(crate) fn ingest_history(
        &mut self,
        events: &[yunyou_session::SessionEvent],
    ) -> (usize, usize) {
        let mut ingested = 0;
        let mut skipped = 0;
        for event in events {
            let _frame = match event {
                yunyou_session::SessionEvent::UserMessage { text } => {
                    // 回合边界：每条用户消息开新回合（turn 编号与 live 侧同规）
                    self.turn += 1;
                    self.turn_open = false;
                    if self.title.is_none() {
                        self.title = Some(derive_title(text));
                    }
                    self.last_prompt_at = Some(now_ms());
                    self.emit("user/message", json!({
                        "id": uuid::Uuid::new_v4().to_string(),
                        "role": "user",
                        "content": [{"type": "text", "text": text}],
                        "source": {"kind": "user"},
                    }), true)
                }
                yunyou_session::SessionEvent::AssistantMessage { text, reasoning } => {
                    let (turn, step) = self.current_place();
                    let mut blocks = vec![json!({"type": "text", "text": text})];
                    if let Some(reasoning) = reasoning.as_deref().filter(|r| !r.is_empty()) {
                        blocks.push(json!({"type": "reasoning", "text": reasoning}));
                    }
                    self.emit("assistant/message", json!({
                        "turn": turn, "step": step,
                        "message": {
                            "id": uuid::Uuid::new_v4().to_string(),
                            "role": "assistant",
                            "content": blocks,
                            "source": {"kind": "model", "provider": "yunyou", "model": self.model},
                        },
                        "stream": [],
                    }), true)
                }
                yunyou_session::SessionEvent::ToolCall { call_id, name, arguments, .. } => {
                    let (turn, step) = self.current_place();
                    self.emit("tool/call", json!({
                        "turn": turn, "step": step,
                        "callId": call_id, "name": name,
                        "arguments": arguments,
                    }), false)
                }
                yunyou_session::SessionEvent::ToolResult { call_id, content, is_error, .. } => {
                    let (turn, step) = self.current_place();
                    self.emit("tool/result", json!({
                        "turn": turn, "step": step,
                        "message": {
                            "id": uuid::Uuid::new_v4().to_string(),
                            "role": "tool",
                            "content": [{"type": "text", "text": content}],
                            "source": {"kind": "tool", "callId": call_id},
                            "toolCallId": call_id,
                            "isError": is_error,
                        },
                    }), true)
                }
                yunyou_session::SessionEvent::SystemMessage { text } => {
                    let (turn, step) = self.current_place();
                    self.emit("system/message", json!({
                        "turn": turn, "step": step,
                        "message": {
                            "id": uuid::Uuid::new_v4().to_string(),
                            "role": "system",
                            "content": [{"type": "text", "text": text}],
                            "source": {"kind": "system"},
                        },
                    }), true)
                }
                // 未桥接词汇：诚实跳过计数（快照面可见），不伪造对应帧
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            ingested += 1;
        }
        // 跳过数累计到会话状态（快照 skippedEvents 持续可见）
        self.skipped_events += skipped as u64;
        (ingested, skipped)
    }
}

/// 会话标题派生：首行、去空白、截 40 字（chars 语义天然不劈 surrogate 对）。
pub(crate) fn derive_title(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    let mut title: String = first_line.chars().take(40).collect();
    if first_line.chars().count() > 40 {
        title.push('…');
    }
    title
}

/// 事件时间戳（毫秒 epoch；对端 SessionWireEvent.time=安全整数毫秒）。
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ServerMessage;

    /// 快照锚点（自主测试面拆出的会话级补充）：cursor/records/头部 id。
    #[test]
    fn 快照锚点_未铸与铸后() {
        let mut wire = SessionWire::new("m".into(), None);
        let snapshot = wire.snapshot(Some("s-1"));
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["cursor"], -1, "未铸=空环 cursor 基线");
        assert_eq!(snapshot["records"].as_array().map(Vec::len), Some(0));
        assert_eq!(snapshot["header"]["id"], "s-1");
        assert!(snapshot["assistantStream"]["revision"].as_u64().is_some());
        // 一回合后 cursor/records 增长；address 缺席回退会话 id
        wire.session_id = Some("created-9".into());
        let _ = wire.apply(&ServerMessage::UserMessage { text: "你好".into() });
        let snapshot = wire.snapshot(None);
        assert_eq!(snapshot["header"]["id"], "created-9");
        assert_eq!(snapshot["cursor"], 2, "user/message+turn/start+step/start 已分配 seq");
        assert_eq!(snapshot["records"].as_array().map(Vec::len), Some(3));
    }

    /// 会话标题派生：首行截 40 字+省略号、多行取首行、空白去边。
    #[test]
    fn 标题派生规则() {
        let long = derive_title(&format!("{}\n第二行", "一".repeat(50)));
        assert_eq!(long.chars().count(), 41, "40 字+省略号：{long}");
        assert!(!long.contains('\n'));
        assert_eq!(derive_title("  短标题  "), "短标题");
        assert_eq!(derive_title(""), "");
    }

    /// 未铸会话：投影基线块与列表行双 None（不猜 id 不伪造）；铸后带出。
    #[test]
    fn 未铸不伪造_铸后带出() {
        let mut wire = SessionWire::new("m".into(), None);
        assert!(wire.baseline_block().is_none());
        assert!(wire.summary_row(true).is_none());
        // 应用事件不出控制帧（无 id）
        let out = wire.apply(&ServerMessage::UserMessage { text: "你好".into() });
        assert!(!out.session.is_empty() && out.control.is_empty());
        wire.session_id = Some("s-1".into());
        let (id, _block) = wire.baseline_block().expect("铸后块在");
        assert_eq!(id, "s-1");
        let row = wire.summary_row(false).expect("铸后行在");
        assert_eq!(row["sessionId"], "s-1");
        assert_eq!(row["agentAvailable"], false, "agentAvailable 由容器判定透传");
    }

    /// 历史灌入（session/open 步 2）：五桥接词汇逐帧入环、未桥接跳过计数、
    /// 标题派生、skippedEvents 进快照（诚实可见）。
    #[test]
    fn 历史灌入_桥接与跳过计数() {
        use yunyou_session::SessionEvent;
        let mut wire = SessionWire::new("m".into(), Some("s-h".into()));
        let events = vec![
            SessionEvent::UserMessage { text: "帮我看看".into() },
            SessionEvent::CommandRun {
                command_id: "c".into(), name: "help".into(), raw_input: "/help".into(),
            },
            SessionEvent::AssistantMessage { text: "看完了".into(), reasoning: None },
            SessionEvent::ToolCall {
                call_id: "t1".into(), name: "read".into(),
                arguments: "{\"path\":\"a\"}".into(),
                parent_tool_call_id: None, batch_id: None,
            },
            SessionEvent::ToolResult {
                call_id: "t1".into(), name: "read".into(),
                content: "26 行".into(), is_error: false,
                parent_tool_call_id: None, batch_id: None,
            },
            SessionEvent::SystemMessage { text: "提示".into() },
        ];
        let (ingested, skipped) = wire.ingest_history(&events);
        assert_eq!((ingested, skipped), (5, 1), "五桥接+一跳过");
        // 重放环=5 帧，词汇序保持；跳过数进快照
        let snapshot = wire.snapshot(None);
        let records = snapshot["records"].as_array().expect("records 在");
        let types: Vec<&str> = records
            .iter()
            .filter_map(|r| r["event"]["type"].as_str())
            .collect();
        assert_eq!(
            types,
            vec!["user/message", "assistant/message", "tool/call", "tool/result", "system/message"],
            "词汇序：{types:?}"
        );
        assert_eq!(snapshot["skippedEvents"], 1, "跳过计数诚实可见：{snapshot}");
        // 标题派生自首条用户消息；seq 延续致密
        assert_eq!(snapshot["header"]["id"], "s-h");
        let seqs: Vec<u64> = records
            .iter()
            .filter_map(|r| r["event"]["seq"].as_u64())
            .collect();
        assert_eq!(seqs, (0..5).collect::<Vec<_>>(), "seq 致密：{seqs:?}");
    }
}
