//! 会话面（P4）：连接回放当前会话历史尾部 + sessions/*.jsonl 列表与按名回放。
//!
//! # 事实源纪律
//! 当前会话历史走 **live SessionStore.load_all**（/new 换槽后自动跟新装配，
//! 无需路径知识——SessionStore trait 未导出路径查询口，报备 lead 留痕：v1
//! 以「服务对象即当前会话」语义绕开，不给 SessionStore 加口）；其他会话用
//! JsonlStore 只读重建（同一解析器，未知类型行跳过——ignorable 机制同源）。
//!
//! # 范围声明（v1）
//! 「会话选择」=查看回放任一历史会话（只读）；把活跃会话**切换**到旧文件
//! 需要壳侧重建装配（ShellCore 是 cli 域），留二期设计注记。

use crate::protocol::{HistoryEntry, ServerMessage};
use crate::server::WebState;
use std::sync::Arc;
use yunyou::ServiceId;
use yunyou_session::{JsonlStore, SessionEvent, SessionStore};

/// 回放尾部上限（任务书口径「历史尾部」；完整无损永远在会话 JSONL）。
const TAIL: usize = 100;

/// 连接回放：当前会话（live store）历史尾部。store 缺失=装配异常 → Error 帧。
pub(crate) async fn history_current(state: &Arc<WebState>) -> ServerMessage {
    let root = state.turns.root();
    match root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()) {
        Ok(store) => match store.load_all().await {
            Ok(events) => history_frame("当前会话", events),
            Err(err) => ServerMessage::Error { text: format!("当前会话读取失败：{err}") },
        },
        Err(err) => ServerMessage::Error { text: format!("会话服务未就绪（装配异常）：{err}") },
    }
}

/// 指定会话回放（id=文件词干；只读）。路径穿越防护：id 只认字母数字与 -_。
pub(crate) async fn history_of(state: &Arc<WebState>, id: &str) -> ServerMessage {
    if !valid_id(id) {
        return ServerMessage::Error { text: format!("会话名不合法：{id}") };
    }
    let path = state.session_dir.join(format!("{id}.jsonl"));
    if !path.is_file() {
        return ServerMessage::Error { text: format!("会话文件不存在：{id}.jsonl") };
    }
    match JsonlStore::new(path).load_all().await {
        Ok(events) => history_frame(id, events),
        Err(err) => ServerMessage::Error { text: format!("会话 {id} 读取失败：{err}") },
    }
}

/// 会话列表（sessions/*.jsonl，按修改时间倒序；id=文件词干）。
pub(crate) fn list(state: &Arc<WebState>) -> ServerMessage {
    let mut sessions: Vec<HistoryEntry> = std::fs::read_dir(&state.session_dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let path = entry.path();
                    // 只认 *.jsonl（与 /new 的落点同款；task-*.jsonl 同列，业主可查）
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    let stem = name.strip_suffix(".jsonl")?.to_string();
                    if !valid_id(&stem) {
                        return None;
                    }
                    let meta = entry.metadata().ok()?;
                    let modified_ms = meta
                        .modified()
                        .ok()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()?
                        .as_millis() as u64;
                    Some(HistoryEntry {
                        id: stem,
                        modified_ms,
                        size_bytes: meta.len(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    sessions.sort_by_key(|entry| std::cmp::Reverse(entry.modified_ms));
    ServerMessage::SessionList { sessions }
}

/// 历史帧组装：超尾部上限时截尾并标注（截断数人话可见）。
fn history_frame(session_id: &str, mut events: Vec<SessionEvent>) -> ServerMessage {
    let total = events.len();
    let truncated = total > TAIL;
    if truncated {
        events = events.split_off(total - TAIL);
    }
    ServerMessage::History { session_id: session_id.to_string(), events, truncated }
}

/// 扫描行条目（步 4 session/list 多行的 JSONL 来源侧）。
pub(crate) struct ScanRow {
    /// 会话 id（文件词干）。
    pub(crate) id: String,
    /// 最后修改时刻（毫秒）。
    pub(crate) modified_ms: u64,
    /// 标题（首个 user-message 首行 derive_title；无用户消息=None 不虚构）。
    pub(crate) title: Option<String>,
    /// 无用户消息=blank（诚实：扫描行以「是否存在 user-message」判定）。
    pub(crate) blank: bool,
}

/// 扫描 sessions/*.jsonl（valid_id 过滤；跳过已在内存表的 id 由调用方去重）。
/// 标题/blank 判定只读文件头部（256KB 上限——长文件截断不炸，完整在文件本身）。
pub(crate) fn scan_rows(session_dir: &std::path::Path) -> Vec<ScanRow> {
    let mut rows = Vec::new();
    let Ok(entries) = std::fs::read_dir(session_dir) else { return rows };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        })
        .collect();
    files.sort();
    for path in files {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !valid_id(stem) {
            continue;
        }
        let Ok(meta) = path.metadata() else { continue };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut title = None;
        let mut blank = true;
        if let Ok(text) = std::fs::read_to_string(&path) {
            let head: String = text.chars().take(256 * 1024).collect();
            for line in head.lines() {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                if value["type"] == "user-message" {
                    let text = value["text"].as_str().unwrap_or("");
                    title = Some(crate::wire_frames::derive_title(text));
                    blank = false;
                    break;
                }
            }
        }
        rows.push(ScanRow { id: stem.to_string(), modified_ms, title, blank });
    }
    rows
}

/// 全部已知会话 ids（workspace 基线同步用，步 4）：内存表（wire 活跃/已开）∪
/// 磁盘扫描（valid_id jsonl 词干），去重稳定序。
pub(crate) fn all_ids(wire_ids: Vec<String>, session_dir: &std::path::Path) -> Vec<String> {
    let mut ids: Vec<String> = wire_ids;
    for row in scan_rows(session_dir) {
        if !ids.contains(&row.id) {
            ids.push(row.id);
        }
    }
    ids
}

/// id 合法性（列表与回放共用）：字母数字与 -_，防空防穿越。
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ServerMessage;
    use crate::server::WebState;
    use crate::turns::TurnRunner;
    use tokio::sync::broadcast;
    use yunyou::Context;
    use yunyou_session::SessionPlugin;

    /// 测试态：真 SessionPlugin（temp jsonl 预置事件）+ 空 sessions 目录。
    /// 目录按原子序号隔离：同一测试内两次 setup 各持 flock，共享路径会互锁
    ///（command_exec 同款隔离手段）。
    async fn setup(with_events: usize) -> (Arc<WebState>, std::path::PathBuf) {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("yunyou-web-sess-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions_dir = dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("sessions 目录");
        let session_file = sessions_dir.join("default.jsonl");
        let root = Context::new();
        let handle = root.plugin(
            SessionPlugin,
            serde_json::json!({ "path": session_file.display().to_string() }),
        );
        handle.await_ready().await.expect("session 插件应装载");
        let store = root
            .get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>())
            .expect("store 应就绪");
        for i in 0..with_events {
            store
                .append(&SessionEvent::UserMessage { text: format!("消息{i}") })
                .await
                .expect("落盘应成功");
        }
        let root_for_getter = root.clone();
        let (sender, _) = broadcast::channel(4);
        let state = Arc::new(WebState {
            token: "t".into(),
            model: "test-model".into(),
            turns: Arc::new(TurnRunner::new(
                Arc::new(move || root_for_getter.clone()),
                "test-model".into(),
            )),
            broadcast: sender,
            session_dir: sessions_dir.clone(),
            wire: Arc::new(crate::bridge::WireBridge::new("test-model".into())),
            scheduler: Arc::new(yunyou_plugin_sdk::scheduler::Scheduler::new(|| 1_000_000)),
            switcher: Arc::new(|_| Box::pin(async { Ok::<(), yunyou::CordisError>(()) })),
            config_store: Arc::new(crate::config_store::ConfigStore::new(std::path::PathBuf::from("/tmp/yunyou-test-yunyou.toml"))),
            loopback: true,
        });
        (state, sessions_dir)
    }

    /// 连接回放：live store 事件按序带出；TAIL 截断标注。
    #[tokio::test(flavor = "multi_thread")]
    async fn 连接回放与尾部截断() {
        let (state, _) = setup(3).await;
        let msg = history_current(&state).await;
        let ServerMessage::History { session_id, events, truncated } = msg else {
            panic!("应回历史帧");
        };
        assert_eq!(session_id, "当前会话");
        assert_eq!(events.len(), 3);
        assert!(!truncated);
        // 超限截尾：造 102 个事件（直接写文件绕开逐个 append 的耗时）
        let (state2, _) = setup(0).await;
        let root = state2.turns.root();
        let store = root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()).unwrap();
        for i in 0..102 {
            store.append(&SessionEvent::UserMessage { text: format!("m{i}") }).await.unwrap();
        }
        let msg = history_current(&state2).await;
        let ServerMessage::History { events, truncated, .. } = msg else { panic!("应回历史帧") };
        assert_eq!(events.len(), TAIL, "应截到尾部 100 条");
        assert!(truncated, "截断必须标注");
        assert!(
            matches!(events.last(), Some(SessionEvent::UserMessage { text }) if text == "m101"),
            "应保留最新事件"
        );
    }

    /// 列表与按名回放：jsonl 只入列、id 防穿越、不存在报人话错。
    #[tokio::test(flavor = "multi_thread")]
    async fn 列表与按名回放与防穿越() {
        let (state, sessions_dir) = setup(1).await;
        // 另造一个历史会话 + 一个非法名文件
        std::fs::write(sessions_dir.join("abc123.jsonl"), "").unwrap();
        std::fs::write(sessions_dir.join("..evil.jsonl"), "").unwrap();
        let msg = list(&state);
        let ServerMessage::SessionList { sessions } = msg else { panic!("应回列表帧") };
        assert_eq!(sessions.len(), 2, "只认合法 id 的 jsonl：{sessions:?}");
        // 按名回放
        let msg = history_of(&state, "abc123").await;
        let ServerMessage::History { session_id, events, .. } = msg else { panic!("应回历史帧") };
        assert_eq!((session_id.as_str(), events.len()), ("abc123", 0));
        // 防穿越与不存在
        let msg = history_of(&state, "../etc").await;
        assert!(matches!(msg, ServerMessage::Error { .. }), "穿越 id 应拒");
        let msg = history_of(&state, "nope").await;
        let ServerMessage::Error { text } = msg else { panic!("应回错误帧") };
        assert!(text.contains("不存在"), "人话指路：{text}");
    }
}
