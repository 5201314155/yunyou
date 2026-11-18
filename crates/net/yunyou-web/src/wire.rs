//! wire 容器（多会话步 1）：会话路由 + 工作区元数据 + 跨会话投影聚合。
//! 单条会话的状态机与帧合成在 [`crate::wire_session::SessionWire`]（每会话
//! 独立 seq/revision 致密）；词汇映射表与协议铁律注记见该模块头。
//!
//! # 多会话语义（步 1 现态=单活跃会话，行为与拆分前等价）
//! - 容器持 sessions 表；**活跃会话**=事件流/prompt 落点（`active`）；
//! - `create_session`：活跃会话未铸 id → 就地铸（保留已有事件）；已铸 → 复用
//!   （单会话复用是刻意的——否则重放环跨会话串历史；多会话扩展随步 2-4）；
//! - 投影基线/列表行按「有 id 的会话」聚合输出（多会话天然就绪）。

use crate::protocol::ServerMessage;
use crate::wire_session::SessionWire;
use serde_json::{json, Value};
use std::collections::HashMap;

/// wire 容器：会话表 + 活跃路由 + 工作区元数据。
pub(crate) struct WireState {
    /// assistant message source 落款（新建会话状态透传）。
    model: String,
    /// 各会话状态（键=session_id；未铸活跃会话以空串键暂存）。
    sessions: HashMap<String, SessionWire>,
    /// 活跃会话键（None=尚无会话状态）。
    active: Option<String>,
    pub(crate) workspace_id: Option<String>,
    /// 工作区元数据 (id, path, title)——首铸会话时的 upsert 素材。
    pub(crate) workspace_meta: Option<(String, String, String)>,
}

impl WireState {
    /// 构造（bridge 单例持锁）。
    pub(crate) fn new(model: String) -> Self {
        Self {
            model,
            sessions: HashMap::new(),
            active: None,
            workspace_id: None,
            workspace_meta: None,
        }
    }

    /// 活跃会话状态（缺则惰性建——未铸 id，事件先入环、投影后铸）。
    fn active_mut(&mut self) -> &mut SessionWire {
        let key = self.active.clone().unwrap_or_default();
        self.active = Some(key.clone());
        self.sessions
            .entry(key)
            .or_insert_with(|| SessionWire::new(self.model.clone(), None))
    }

    /// 活跃会话只读（容器内路由与观测）。
    pub(crate) fn active_ref(&self) -> Option<&SessionWire> {
        self.active.as_ref().and_then(|k| self.sessions.get(k))
    }

    /// 单条 ServerMessage → 活跃会话的 0..n 会话帧 + 0..n 控制投影帧。
    pub(crate) fn apply(&mut self, msg: &ServerMessage) -> (Vec<Value>, Vec<Value>) {
        let out = self.active_mut().apply(msg);
        (out.session, out.control)
    }

    /// 会话创建：活跃会话未铸 id → 就地铸（保留已有事件，多会话步 2 起按 id
    /// 分立）；已铸 → 复用（返回现有 id）。返回会话 id。
    pub(crate) fn create_session(&mut self, requested: Option<&str>) -> String {
        let key = self.active.clone().unwrap_or_default();
        let wire = self.active_mut();
        if let Some(existing) = wire.session_id.as_deref().filter(|id| !id.is_empty()) {
            return existing.to_string();
        }
        let id = match requested {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => uuid::Uuid::new_v4().to_string(),
        };
        wire.session_id = Some(id.clone());
        // 未铸会话以空串键暂存——铸后重键到 id（表键=会话 id 单源）
        if key != id {
            if let Some(wire) = self.sessions.remove(&key) {
                self.sessions.insert(id.clone(), wire);
            }
            self.active = Some(id.clone());
        }
        id
    }

    /// follow 快照（address 覆盖 header.id；缺省回退活跃会话 id）。
    pub(crate) fn snapshot(&self, address_session_id: Option<&str>) -> Value {
        self.active_ref()
            .map(|wire| wire.snapshot(address_session_id))
            .unwrap_or_else(|| SessionWire::new(String::new(), None).snapshot(address_session_id))
    }

    /// session/control 基线：全部有 id 会话的投影块聚合（多会话就绪）。
    pub(crate) fn control_baseline(&self) -> Value {
        let mut projections = serde_json::Map::new();
        for wire in self.sessions.values() {
            if let Some((id, block)) = wire.baseline_block() {
                projections.insert(id, block);
            }
        }
        json!({"type": "baseline", "value": {"projections": projections}})
    }

    /// session/list 行（有 id 的会话全列；agentAvailable=是否活跃会话——
    /// 非活跃历史行不可直接 prompt，诚实标注，步 4 扩展 JSONL 扫描行）。
    pub(crate) fn session_summaries(&self) -> Vec<Value> {
        let active_key = self.active.clone().unwrap_or_default();
        let mut rows: Vec<Value> = self
            .sessions
            .iter()
            .filter_map(|(key, wire)| wire.summary_row(*key == active_key))
            .collect();
        rows.sort_by(|a, b| {
            let key = |v: &Value| {
                v["sessionId"].as_str().unwrap_or("").to_string()
            };
            key(a).cmp(&key(b))
        });
        rows
    }

    /// 当前全部会话 ids（workspace 基线 sessionIds 用）。
    pub(crate) fn session_ids(&self) -> Vec<String> {
        self.sessions
            .values()
            .filter_map(|wire| {
                wire.session_id.clone().filter(|id| !id.is_empty())
            })
            .collect()
    }

    /// 登记活跃会话的 prompt requestId（echo 对账；下一条 user/message 消费）。
    pub(crate) fn set_request_id(&mut self, request_id: Option<String>) {
        self.active_mut().request_id = request_id;
    }

    /// 活跃会话 id（live 帧定向路由标签；未铸=None）。
    pub(crate) fn active_session_id(&self) -> Option<String> {
        self.active_ref().and_then(|w| w.session_id.clone())
    }

    /// 会话打开（session/open，多会话步 2）：切活跃+灌历史（先铸 id 再灌——
    /// 表键单源纪律；目标会话已在表=直接激活，不在=新建后灌）。返回 (灌入,
    /// 跳过) 供响应如实回显（跳过=未桥接词汇计数，诚实可见）。
    pub(crate) fn open_session(
        &mut self,
        id: &str,
        events: &[yunyou_session::SessionEvent],
    ) -> (usize, usize) {
        self.active = Some(id.to_string());
        let wire = self
            .sessions
            .entry(id.to_string())
            .or_insert_with(|| SessionWire::new(self.model.clone(), Some(id.to_string())));
        if wire.session_id.is_none() {
            wire.session_id = Some(id.to_string());
        }
        wire.ingest_history(events)
    }

    /// 测试面：直设活跃会话 id（wire_tests 直构造断言用；生产面走 create_session）。
    #[cfg(test)]
    pub(crate) fn test_set_session_id(&mut self, id: Option<String>) {
        self.active_mut().session_id = id;
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
