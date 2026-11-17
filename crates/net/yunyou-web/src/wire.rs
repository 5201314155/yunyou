//! 对端 wire 帧状态机：我们 13 词汇 [`ServerMessage`] → 对端 SessionWireEvent /
//! assistantStream 通知帧（任务40 适配层第二片「对话事件桥」，施工图=
//! docs/设计/yunyou-web-v2-适配层桥接设计-云屏.md §三，形状全部按上游
//! vendor 树实读核对：session-controller/types.ts、llm/message.ts、
//! core/session/surface.ts、client/sessions/assistant-stream.ts）。
//!
//! # 词汇映射（§3.2 契约记录的落码）
//! user-message→user/message（data=Message 本体）｜tool-call→tool/call｜
//! tool-result→tool/result｜assistant-message→assistant/message｜
//! turn-complete→step/end+turn/end｜error→system/message+turn/end(error)｜
//! assistant-delta/reasoning-delta→assistantStream chunk 通知帧（非持久事件）。
//!
//! # 协议铁律（上游 client 消费侧实测，违者触发重连风暴）
//! - 持久事件 seq 全局致密 +1（journal `follows: right===left+1` 检查）；
//! - surface 事件（user/assistant/system message、tool/result）必带
//!   `surfaceOp:'append'`（isSurfaceEvent/isAppendSurfaceEvent 判定）；
//! - assistantStream 通知帧 `revision` 逐帧 +1（transport.ts 跳号=载体重连）；
//! - chunk 帧 `index` 致密（0,1,2…）；`end` 帧 committed seq 必须指向已推
//!   的 assistant/message 落定事件，且其 data.turn/step 与 attempt 对齐
//!   （attemptForSettlement 检查）；
//! - user/message data.source 必带 `rpcId`（prompt requestId echo 对账——
//!   缺失则客户端乐观 echo 永不退场，消息双显）。
//!
//! # 演进清单（批⑩ A 路加法，随批落地情况）
//! ①index/attemptId/revision——由本状态机合成（ServerMessage 词汇不动，比
//!   设计稿预期更收敛）；②tool arguments 全量——agent-loop 只透传 120 字
//!   摘要，全量需观察面改造（跨域，未做，置摘要）；③usage——用量在
//!   TurnComplete 才到、错过 assistant/message 时机（对端字段可选，缺失=
//!   不显示用量，未做，需 agent-loop 演进，跨域）。
//!
//! 共享外壳（单例/订阅/泵）在 [`crate::bridge`]；测试外挂 `wire_tests.rs`
//! （#[path] 先例，源文件行数纪律）。
//!
//! # 投影面（批⑬「品牌化+投影面补桥」新增）
//! session/control 基线与 live 投影帧（对端消费面实读：
//! session-controller/src/control.ts + client/sessions/projection-store.ts +
//! manager.ts:755『list rows read the generic title projection key』）：
//! - 键 `title`：首条用户消息派生（首行截 40 字）；键 `sessionListMetadata`：
//!   `{blank, lastPromptAt}`（SessionListMetadata 形状，types.ts:38）。
//! - 侧栏列表行另需 session/list items 真名（[`Self::session_summary`]）。
//! - live 增量走 `{type:'projection',sessionId,seq,changes:[{key,val}]}` 帧
//!   （StreamKind::Control 路由），首条用户消息时推 title+metadata 两键。

use crate::protocol::ServerMessage;
use serde_json::{json, Value};
use std::collections::VecDeque;

/// 重放环容量（快照 records 上限；超出丢最旧——完整流在会话 JSONL，与 CLI 同纪律）。
const HISTORY_CAP: usize = 512;

/// 状态机：seq/turn/revision 计数 + 当前 attempt + 快照重放环。
pub(crate) struct WireState {
    model: String,
    next_seq: u64,
    turn: u64,
    turn_open: bool,
    revision: u64,
    pub(crate) session_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    /// 工作区元数据 (id, path, title)——首铸会话时的 upsert 素材。
    pub(crate) workspace_meta: Option<(String, String, String)>,
    pub(crate) request_id: Option<String>,
    /// 会话标题（首条用户消息派生；投影面 `title` 键与 session/list 行真名）。
    title: Option<String>,
    /// 最近一条用户消息落款时刻（投影面 sessionListMetadata.lastPromptAt）。
    last_prompt_at: Option<u64>,
    attempt: Option<Attempt>,
    history: VecDeque<Value>,
    /// 待发控制投影帧（bridge.push 泵出，StreamKind::Control 路由）。
    pub(crate) control_out: Vec<Value>,
}

/// 进行中的流式 attempt（对端 SessionAssistantStreamFrame start-chunk-end 模型）。
struct Attempt {
    attempt_id: String,
    started_after_seq: i64,
    turn: u64,
    step: u64,
    next_index: u64,
    /// 已发 chunk 的 raw 记录（快照 activeAttempt.stream 重放用，保序）。
    stream: Vec<Value>,
}

impl WireState {
    /// 构造（bridge 单例持锁；model=assistant message source 落款）。
    pub(crate) fn new(model: String) -> Self {
        Self {
            model,
            next_seq: 0,
            turn: 0,
            turn_open: false,
            revision: 0,
            session_id: None,
            workspace_id: None,
            workspace_meta: None,
            request_id: None,
            title: None,
            last_prompt_at: None,
            attempt: None,
            history: VecDeque::new(),
            control_out: Vec::new(),
        }
    }

    /// 控制投影值表（title + sessionListMetadata；无 turn 时 title 键缺席=
    /// 对端 `session.untitled` 兜底，blank=true 行由列表隐藏逻辑收）。
    fn projection_values(&self) -> Value {
        let mut values = json!({"sessionListMetadata": {
            "blank": self.turn == 0,
            "lastPromptAt": self.last_prompt_at,
        }});
        if let Some(title) = &self.title {
            values["title"] = Value::String(title.clone());
        }
        values
    }

    /// 当前会话 id（未铸会话=None——投影帧与基线留空，不猜 id）。
    fn current_session_id(&self) -> Option<&str> {
        self.session_id.as_deref().filter(|id| !id.is_empty())
    }

    /// session/control 基线（对端 SessionControlBaseline 形状，control.ts:46）：
    /// 单会话投影块；未铸会话=空投影表（诚实缺省，不伪造块）。
    pub(crate) fn control_baseline(&self) -> Value {
        let mut projections = serde_json::Map::new();
        if let Some(id) = self.current_session_id() {
            projections.insert(
                id.to_string(),
                json!({"asOfSeq": self.next_seq as i64 - 1, "values": self.projection_values()}),
            );
        }
        json!({"type": "baseline", "value": {"projections": projections}})
    }

    /// session/list 单行（SessionSummary 形状，types.ts:154；侧栏行真名靠
    /// projections 块 title 键——manager.ts:755 消费路径）。未铸会话=空表。
    pub(crate) fn session_summaries(&self) -> Vec<Value> {
        let Some(id) = self.current_session_id() else { return Vec::new() };
        vec![json!({
            "sessionId": id,
            "agentAvailable": true,
            "updatedAt": now_ms(),
            "running": self.turn_open,
            "blank": self.turn == 0,
            "projections": {
                "kind": "sequenced",
                "asOfSeq": self.next_seq as i64 - 1,
                "values": self.projection_values(),
            },
        })]
    }

    /// follow 快照：header+投影基线+重放环+assistantStream 基线
    /// （transport.ts 强制要求 opted-in 快照必须带 assistantStream，缺失即抛错）。
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

    /// 单条 ServerMessage → 0..n 帧（顺序=持久事件与流式帧按对端状态机交错）。
    pub(crate) fn apply(&mut self, msg: &ServerMessage) -> Vec<Value> {
        match msg {
            ServerMessage::UserMessage { text } => self.on_user_message(text),
            ServerMessage::AssistantDelta { text } => self.on_chunk(text, "text-delta"),
            ServerMessage::ReasoningDelta { text } => self.on_chunk(text, "reasoning-delta"),
            ServerMessage::ToolCall { call_id, name, arguments_summary, .. } => {
                let (turn, step) = self.current_place();
                vec![self.emit("tool/call", json!({
                    "turn": turn, "step": step,
                    "callId": call_id, "name": name,
                    "arguments": arguments_summary,
                }), false)]
            }
            ServerMessage::ToolResult {
                call_id, content, is_error, ..
            } => {
                let (turn, step) = self.current_place();
                let content_text = match content {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                vec![self.emit("tool/result", json!({
                    "turn": turn, "step": step,
                    "message": {
                        "id": uuid::Uuid::new_v4().to_string(),
                        "role": "tool",
                        "content": [{"type": "text", "text": content_text}],
                        "source": {"kind": "tool", "callId": call_id},
                        "toolCallId": call_id,
                        "isError": is_error,
                    },
                }), true)]
            }
            ServerMessage::AssistantMessage { text, reasoning } => {
                self.on_assistant_message(text, reasoning.as_deref())
            }
            ServerMessage::TurnComplete { .. } => self.on_turn_complete(),
            ServerMessage::Error { text } => self.on_error(text),
            // 控制面词汇不进 follow 流（hello→连接状态；session-list→unary；
            // history→v1 页面词汇；bye/command-output→P3 命令缝）
            _ => Vec::new(),
        }
    }

    /// 用户消息：持久 user/message（append，source 带 rpcId echo）+ 回合/
    /// 步骤开闸事件 + attempt start 帧。
    fn on_user_message(&mut self, text: &str) -> Vec<Value> {
        let (turn, step) = self.open_turn();
        let mut source = json!({"kind": "user"});
        if let Some(request_id) = self.request_id.take() {
            source["rpcId"] = Value::String(request_id);
        }
        // 标题派生：会话首名=首条用户消息（首行截 40 字，上游 host 自动标题
        // 同语义）；随后推控制投影帧（title+sessionListMetadata 两键）。
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
        let mut frames = vec![user_event];
        frames.push(self.emit("turn/start", json!({"turn": turn}), false));
        frames.push(self.emit("step/start", json!({"turn": turn, "step": step}), false));
        // attempt 字段先取值再合成帧（开闸借用与 revision 递增借用不相交）
        let (attempt_id, started_after_seq) = self.open_attempt(turn, step);
        let revision = self.bump_revision();
        frames.push(json!({"type": "assistant-stream", "frame": {
            "type": "start",
            "attemptId": attempt_id,
            "revision": revision,
            "startedAfterSeq": started_after_seq,
            "turn": turn,
            "step": step,
        }}));
        // 控制投影帧（侧栏真名 live 更新）：seq=本条 user/message 事件 seq
        //（投影 cut 与事件对齐；projection-store higher-seq-wins）。
        let mut changes = vec![json!({"key": "sessionListMetadata", "val": {
            "blank": false, "lastPromptAt": self.last_prompt_at,
        }})];
        if titled {
            changes.insert(0, json!({"key": "title", "val": self.title}));
        }
        if let Some(id) = self.current_session_id().map(str::to_string) {
            self.control_out.push(json!({
                "type": "projection", "sessionId": id, "seq": event_seq, "changes": changes,
            }));
        }
        frames
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

    /// 助手消息落定：持久 assistant/message（append，model source 落款）+
    /// attempt end 帧（committed 指向该事件 seq——pending 落定对齐）。
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

    /// 开新回合：turn 计数 +1，step 恒 1（一回合一次模型尝试——我们 agent-loop
    /// 单 AssistantMessage 终态；多步演进随 agent-loop 观察面，见演进清单）。
    fn open_turn(&mut self) -> (u64, u64) {
        self.turn += 1;
        self.turn_open = true;
        (self.turn, 1)
    }

    /// 开新 attempt（回合一 attempt；startedAfterSeq=已分配最大 seq，
    /// attemptForSettlement 要求落定事件 seq 严格大于它）。返回 (attemptId,
    /// startedAfterSeq) 供 start 帧合成（借用不外溢）。
    fn open_attempt(&mut self, turn: u64, step: u64) -> (String, i64) {
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
    fn abandon_attempt(&mut self) -> Option<Value> {
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
    fn current_place(&self) -> (u64, u64) {
        (self.turn, 1)
    }

    /// 分配 seq、入重放环、包 event 帧一次完成（surface=是否带 surfaceOp:'append'）。
    fn emit(&mut self, event_type: &str, data: Value, surface: bool) -> Value {
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
    fn bump_revision(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }
}

/// 会话标题派生：首行、去空白、截 40 字（字符面截断，不劈 surrogate 对——
/// chars 语义天然安全）。
fn derive_title(text: &str) -> String {
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
#[path = "wire_tests.rs"]
mod tests;
