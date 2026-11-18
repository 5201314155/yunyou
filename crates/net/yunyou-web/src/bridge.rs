//! wire 桥单例与泵（任务40 适配层第二片）：[`crate::wire::WireState`] 状态机
//! 的共享外壳——ServerMessage 广播 → 对端帧的进程级单消费泵 + mux 连接的
//! 订阅分发端。状态机本体（词汇转换/帧合成/快照）在 [`crate::wire`]；
//! 职责分家：状态机管「怎么转」，单例管「共享与分发」。

use crate::protocol::ServerMessage;
use crate::server::WebState;
use crate::wire::WireState;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// 桥帧流别（mux 连接按流端点路由：session 流吃事件/流式帧，workspace 流吃
/// 工作区帧——帧值本体带对端 type 字段，流别只做路由）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamKind {
    Session,
    Workspace,
    /// 全局会话控制投影流（session/control：基线+live 投影帧——侧栏真名）。
    Control,
}

/// 状态锁毒化恢复（ratchet：生产面零 expect/unwrap——桥状态是纯计数/缓冲，
/// panic 后原值仍可用，客户端按 seq/revision 铁律自会重连对齐）。
pub(crate) fn lock_state(state: &Mutex<WireState>) -> std::sync::MutexGuard<'_, WireState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// wire 桥单例：广播端（mux 连接订阅）+ 状态机（临界区无 await，std 锁）。
pub(crate) struct WireBridge {
    state: Mutex<WireState>,
    /// 下行帧流：流别 + 会话标签（Session 帧带活跃会话 id 供定向；其余 None）。
    frames: broadcast::Sender<(StreamKind, Option<String>, Arc<Value>)>,
}

impl WireBridge {
    /// 构造（进程级单例；model=assistant message source 落款）。
    pub(crate) fn new(model: String) -> Self {
        let (frames, _) = broadcast::channel(256);
        Self { state: Mutex::new(WireState::new(model)), frames }
    }

    /// mux 订阅端（每条 mux 连接一个；无订阅者时事件安全丢弃）。
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<(StreamKind, Option<String>, Arc<Value>)> {
        self.frames.subscribe()
    }

    /// 消费一条 ServerMessage，产出 0..n 个 mux item 值帧并广播。
    /// 返回产出数（测试/诊断用；无订阅者不报错——WebObserver 同纪律）。
    pub(crate) fn push(&self, msg: &ServerMessage) -> usize {
        let (frames, control) = {
            let mut state = lock_state(&self.state);
            state.apply(msg)
        };
        let count = frames.len();
        // 会话标签：活跃会话 id（apply 恒打活跃会话——单会话现态唯一产出源）
        let tag = {
            let state = lock_state(&self.state);
            state.active_session_id()
        };
        for frame in frames {
            // 无订阅者=无人看 follow 流，安全丢弃
            let _ = self.frames.send((StreamKind::Session, tag.clone(), Arc::new(frame)));
        }
        for frame in control {
            let _ = self.frames.send((StreamKind::Control, None, Arc::new(frame)));
        }
        count
    }

    /// 登记工作区元数据（首个 workspace/follow 打开时；upsert 合成用）。
    pub(crate) fn register_workspace(&self, path: &str, title: &str) {
        let mut state = lock_state(&self.state);
        let id = state
            .workspace_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        state.workspace_meta = Some((id, path.to_string(), title.to_string()));
    }

    /// 会话创建（v2 RPC session/create）：单会话语义——本服务恒一条对话，
    /// 已有会话一律复用（单会话复用是刻意的；否则客户端
    /// 每次开页各建新会话而重放环是全局的，会串历史）。首次铸造时广播
    /// workspace upsert（先开的 workspace 流基线 sessionIds 为空，客户端
    /// composer chip 名解析依赖 session 落进工作区）。
    pub(crate) fn create_session(&self, requested: Option<&str>) -> String {
        let upsert = {
            let mut state = lock_state(&self.state);
            // 已铸=复用（现有会话 id 原样返回，不重铸）
            let existing = state.session_ids().into_iter().next();
            if let Some(existing) = existing {
                return existing;
            }
            let id = state.create_session(requested);
            // 首铸 upsert（元数据在位才发；无元数据=无 workspace 流开过，缺省无害）
            state.workspace_meta.as_ref().map(|(workspace_id, path, title)| {
                let now = crate::wire_frames::now_ms();
                json!({
                    "type": "upsert",
                    "workspace": {
                        "workspaceId": workspace_id,
                        "path": path, "title": title,
                        "sessionIds": [id], "createdAt": now, "updatedAt": now,
                    }
                })
            })
        };
        if let Some(frame) = upsert {
            let _ = self.frames.send((StreamKind::Workspace, None, Arc::new(frame)));
        }
        lock_state(&self.state).session_ids().into_iter().next().unwrap_or_default()
    }

    /// 会话打开（session/open，多会话步 2）：切活跃+灌历史（load 在锁外
    /// 完成，锁内只做纯状态操作）。返回 (灌入, 跳过)。
    pub(crate) fn open_session(
        &self,
        id: &str,
        events: &[yunyou_session::SessionEvent],
    ) -> (usize, usize) {
        lock_state(&self.state).open_session(id, events)
    }

    /// 工作区稳定 id（同进程恒一；sessionIds 变更靠 upsert/重开流重取基线）。
    pub(crate) fn workspace_id(&self) -> String {
        lock_state(&self.state)
            .workspace_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone()
    }

    /// 当前会话 ids（workspace 基线 sessionIds 用；单会话=0 或 1 个）。
    pub(crate) fn session_ids(&self) -> Vec<String> {
        lock_state(&self.state).session_ids()
    }

    /// 登记 prompt requestId（echo 对账；下一条 user/message 消费后即清）。
    pub(crate) fn set_request_id(&self, request_id: Option<String>) {
        lock_state(&self.state).set_request_id(request_id);
    }

    /// 最近登记的 requestId（测试观测用；生产面经 user/message echo 消费）。
    #[cfg(test)]
    pub(crate) fn request_id(&self) -> Option<String> {
        lock_state(&self.state).active_ref().and_then(|w| w.request_id.clone())
    }

    /// follow 快照（mux open session/follow 的 opening item 值）：
    /// 委托状态机（锁内成形，无 await）。
    pub(crate) fn snapshot(&self, address_session_id: Option<&str>) -> Value {
        lock_state(&self.state).snapshot(address_session_id)
    }

    /// session/control 基线（投影块；批⑬投影面）。
    pub(crate) fn control_baseline(&self) -> Value {
        lock_state(&self.state).control_baseline()
    }

    /// session/list items（侧栏行真名投影；批⑬投影面）。
    pub(crate) fn session_summaries(&self) -> Vec<Value> {
        lock_state(&self.state).session_summaries()
    }
}

/// wire 桥泵：ServerMessage 广播 → 对端帧状态机 → mux follow 流分发。
/// 单消费泵保 seq/revision 致密（多路订阅由桥内二次广播承担）；积压丢弃
/// 不破坏计数连续性（文本缺字可由 assistant-message 终态冻结补齐）。
pub(crate) fn spawn_pump(state: Arc<WebState>) {
    let mut server_rx = state.broadcast.subscribe();
    tokio::spawn(async move {
        loop {
            match server_rx.recv().await {
                Ok(msg) => {
                    state.wire.push(&msg);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    println!("☁ 云悠 web wire 桥：事件积压丢弃 {n} 条（流式增量可能缺字，终态帧会冻结补齐）");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
