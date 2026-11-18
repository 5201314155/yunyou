//! JSONL 追加存储：一行一个事件；追加用 `tokio::fs::OpenOptions::append`，
//! 读取端逐行解析并跳过未知类型（crate 文档的 ignorable 机制）。
//!
//! 跨进程互斥（task-36①）实现在 [`crate::lock`] 模块：会话插件装配期经
//! [`JsonlStore::try_lock_exclusive`] 抢 `<会话文件>.lock` 的 flock 排他锁。
//!
//! 内部状态经 `Arc` 共享：`SessionStore` trait 的方法返回 `'static` future
//!（方案 §2.1.3 的 object-safe 契约），实现必须拥有自己的数据。

use super::SessionStore;
use crate::events::SessionEvent;
use crate::lock::ExclusiveLock;
use yunyou::{CordisError, ErrorKind, Result};
use yunyou_util::BoxFut;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// 基于 JSONL 文件的会话存储。
///
/// # 契约
/// - 目录不存在时首次追加自动创建父目录；
/// - 写串行化：内部一把写锁保证多任务并发追加不交错；
/// - `load_all` 逐行解析：空行跳过；未知 `type` 的行跳过（不失败，
///   见 crate 文档 ignorable 机制）。
pub struct JsonlStore {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    /// 追加写句柄（懒打开，一把锁串行化）。
    writer: Mutex<Option<tokio::fs::File>>,
    /// 跨进程排他锁句柄（flock，实现在 [`crate::lock`]；持有期=锁存活期）。
    /// std 锁即可：临界区为同步 flock 抢锁（微秒级），且在装配同步段调用。
    exclusive: std::sync::Mutex<Option<ExclusiveLock>>,
}

impl JsonlStore {
    /// 用会话文件路径构造（父目录可不存在）。返回共享句柄。
    pub fn new(path: impl Into<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                path: path.into(),
                writer: Mutex::new(None),
                exclusive: std::sync::Mutex::new(None),
            }),
        })
    }

    /// 存储文件路径。
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// 取跨进程排他锁（task-36①，方案 A；锁实现在 [`crate::lock`]）。
    ///
    /// 装配期调用：失败即装配失败、启动退出——双开并发写同一会话被拦在起步。
    /// 句柄存于 [`Inner::exclusive`]，进程存活期持锁；重复调用幂等（已持锁直接成功）。
    pub fn try_lock_exclusive(&self) -> Result<()> {
        let mut guard = self.inner.exclusive.lock().map_err(|_| {
            CordisError::new(ErrorKind::Internal, "session/会话锁句柄互斥中毒")
        })?;
        if guard.is_some() {
            return Ok(());
        }
        *guard = Some(ExclusiveLock::acquire(&self.inner.path)?);
        Ok(())
    }
}

/// 会话域错误统一包装（lock 模块同用）。
pub(crate) fn file_error(path: &Path, what: &str, err: std::io::Error) -> CordisError {
    CordisError::new(ErrorKind::Internal, format!("session/{what} {}: {err}", path.display()))
}

impl SessionStore for JsonlStore {
    fn path(&self) -> Option<&std::path::Path> {
        Some(self.inner.path.as_path())
    }

    fn append(&self, event: &SessionEvent) -> BoxFut<'static, Result<()>> {
        let inner = self.inner.clone();
        // 序列化前移（task-36①a，挑刺 P3-1a）：序列化在同步段完成（&event 引用
        // 仍存活），future 只搬 String——消除「深克隆事件进 future 再序列化」的
        // 同内容双份 O(size) 拷贝（'static 契约不破）。
        let line = match serde_json::to_string(event) {
            Ok(line) => line,
            Err(err) => {
                // 序列化失败同步段提前返回（task-36①a：future 只搬 String）
                return Box::pin(async move {
                    Err(CordisError::new(
                        ErrorKind::Internal,
                        format!("session/序列化失败: {err}"),
                    ))
                }) as BoxFut<'static, Result<()>>;
            }
        };
        Box::pin(async move {
            {
                let mut guard = inner.writer.lock().await;
                if guard.is_none() {
                    if let Some(parent) = inner.path.parent() {
                        tokio::fs::create_dir_all(parent)
                            .await
                            .map_err(|err| file_error(&inner.path, "创建会话目录失败", err))?;
                    }
                    let file = tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&inner.path)
                        .await
                        .map_err(|err| file_error(&inner.path, "打开会话文件失败", err))?;
                    *guard = Some(file);
                }
                let file = guard.as_mut().expect("上方已初始化");
                // 单次 write（task-36①b，挑刺 P3-1b）：行体与换行合并一次
                // write_all——每事件少一次 syscall（H4 修 BufWriter 时同批聚合）。
                let mut buf = line.into_bytes();
                buf.push(b'\n');
                file.write_all(&buf)
                    .await
                    .map_err(|err| file_error(&inner.path, "写入会话失败", err))?;
                file.flush()
                    .await
                    .map_err(|err| file_error(&inner.path, "写入会话失败", err))?;
            }
            Ok(())
        })
    }

    fn ensure_writable(&self) -> BoxFut<'static, Result<()>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            // task-74 v5 续聊升级预检：写路径可开（append 语义不截断）；真句柄
            // 留给 append 懒初始化（避免双 fd 竞写同一 O_APPEND 流的偏移混淆）。
            // flock 在装配期已持（重开文件不重锁，锁在 store 生命周期）。
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&inner.path)
                .await
                .map_err(|err| file_error(&inner.path, "续聊写模式升级失败", err))?;
            drop(file);
            Ok(())
        })
    }

    fn load_all(&self) -> BoxFut<'static, Result<Vec<SessionEvent>>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let path = &inner.path;
            if !path.exists() {
                return Ok(Vec::new());
            }
            let file = tokio::fs::File::open(path)
                .await
                .map_err(|err| file_error(path, "读取会话失败", err))?;
            let mut lines = BufReader::new(file).lines();
            let mut events = Vec::new();
            let mut skipped_unknown = 0usize;
            let mut skipped_corrupt = 0usize;
            loop {
                // UTF-8 半字节损坏（断电/强杀把多字节字符截断）在 tokio lines 层
                // 直接报 InvalidData，走不到下方 parse_line 的 Corrupt 自救
                //（task-36③：此类残行曾致整会话永久硬拒绝）。tokio 语义：read_line
                // 的字节已从底层消费、仅 UTF-8 校验失败——跳过残行继续读安全；
                // 其余 IO 错误（权限/硬件）仍诚实硬失败。
                let line = match lines.next_line().await {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                        skipped_corrupt += 1;
                        continue;
                    }
                    Err(err) => return Err(file_error(path, "读取会话行失败", err)),
                };
                if line.trim().is_empty() {
                    continue;
                }
                match parse_line(&line) {
                    Parsed::Event(event) => events.push(event),
                    Parsed::UnknownType => skipped_unknown += 1,
                    Parsed::Corrupt => skipped_corrupt += 1,
                }
            }
            if skipped_unknown + skipped_corrupt > 0 {
                // 不失败，但必须留痕（ignorable 机制的观测位）
                eprintln!(
                    "session/警告: {} 存在不可识别事件行（未知 {skipped_unknown}，损坏 {skipped_corrupt}），已跳过",
                    path.display()
                );
            }
            Ok(events)
        })
    }
}

enum Parsed {
    Event(SessionEvent),
    UnknownType,
    Corrupt,
}

/// 解析一行：先取 `type` 标签判词汇，已知类型再完整解析。
///
/// 词汇表=五类对话事件+两类命令日志事件（command-run/command-done，任务29 D2）。
/// 本集合之外的一切 tag（含未来版本新增）走 UnknownType 跳过——tag 探测即版本门：
/// 旧读取器（词汇表只含五类）遇本两枚变体同样落 UnknownType，不炸（既有实证行为）。
fn parse_line(line: &str) -> Parsed {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Parsed::Corrupt;
    };
    let known = matches!(
        value.get("type").and_then(|tag| tag.as_str()),
        Some(
            "user-message"
                | "assistant-message"
                | "tool-call"
                | "tool-result"
                | "system-message"
                | "command-run"
                | "command-done"
                | "resume-mark"
                | "compaction"
        )
    );
    if !known {
        return Parsed::UnknownType;
    }
    match serde_json::from_value::<SessionEvent>(value) {
        Ok(event) => Parsed::Event(event),
        Err(_) => Parsed::Corrupt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::CommandOutcome;
    use crate::SessionEvent;

    /// 追加 → 恢复 → 再追加 → 再恢复（重启恢复验收，验收标准 3）。
    #[tokio::test]
    async fn 追加与重启恢复() {
        let dir = std::env::temp_dir().join(format!("yunyou-session-test-{}", std::process::id()));
        let path = dir.join("s1/session.jsonl");
        let _ = tokio::fs::remove_dir_all(dir.join("s1")).await;
        let store = JsonlStore::new(&path);
        store
            .append(&SessionEvent::UserMessage { text: "你好".into() })
            .await
            .expect("追加应成功");
        store
            .append(&SessionEvent::ToolCall {
                call_id: "c1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
                parent_tool_call_id: None,
                batch_id: None,
            })
            .await
            .expect("追加应成功");
        // 模拟重启：全新实例指向同一文件
        let reopened = JsonlStore::new(&path);
        let events = reopened.load_all().await.expect("恢复应成功");
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], SessionEvent::UserMessage { text } if text == "你好"));
        assert!(matches!(&events[1], SessionEvent::ToolCall { name, .. } if name == "bash"));
        // 重启后继续追加，两代实例的写入都在
        reopened
            .append(&SessionEvent::SystemMessage { text: "续跑".into() })
            .await
            .expect("重启后追加应成功");
        let events = reopened.load_all().await.expect("二次恢复应成功");
        assert_eq!(events.len(), 3);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    /// 未知类型与坏行跳过不失败（ignorable 机制）。
    #[tokio::test]
    async fn 未知事件与坏行容忍() {
        let dir = std::env::temp_dir().join(format!("yunyou-session-tolerant-{}", std::process::id()));
        let path = dir.join("session.jsonl");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            &path,
            concat!(
                "{\"type\":\"user-message\",\"text\":\"ok\"}\n",
                "{\"type\":\"future-event\",\"x\":1}\n",
                "{broken json\n",
                "{\"type\":\"tool-result\",\"call_id\":\"c\",\"name\":\"t\",\"content\":\"v\",\"is_error\":false}\n",
            ),
        )
        .await
        .unwrap();
        let store = JsonlStore::new(&path);
        let events = store.load_all().await.expect("容忍读取应成功");
        assert_eq!(events.len(), 2, "未知行与坏行应被跳过");
        assert!(matches!(&events[0], SessionEvent::UserMessage { text } if text == "ok"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// parse_line 的分类判定。
    #[test]
    fn 行解析分类() {
        assert!(matches!(parse_line(""), Parsed::Corrupt));
        assert!(matches!(parse_line("{broken"), Parsed::Corrupt));
        assert!(matches!(parse_line("{\"type\":\"future\"}"), Parsed::UnknownType));
        assert!(matches!(
            parse_line("{\"type\":\"system-message\",\"text\":\"t\"}"),
            Parsed::Event(_)
        ));
        // 命令日志两变体（任务29 D2）落 Event；未知名照旧 UnknownType
        assert!(matches!(
            parse_line("{\"type\":\"command-run\",\"command_id\":\"u1\",\"name\":\"help\",\"raw_input\":\"/help\"}"),
            Parsed::Event(_)
        ));
        assert!(matches!(
            parse_line("{\"type\":\"command-done\",\"command_id\":\"u1\",\"kind\":\"success\",\"text\":\"ok\"}"),
            Parsed::Event(_)
        ));
        assert!(matches!(parse_line("{\"type\":\"command-ghost\"}"), Parsed::UnknownType));
    }

    /// 半字节截断行跳过（task-36③）：断电/强杀把多字节字符截断时，tokio lines 层
    /// 直接报 InvalidData（走不到 parse_line 的 Corrupt 自救）——load_all 须在该层
    /// 跳过残行并继续读后续合法行，整会话不再因此永久硬拒绝。
    #[tokio::test]
    async fn 半字节截断行跳过() {
        let dir = std::env::temp_dir().join(format!("yunyou-session-utf8-{}", std::process::id()));
        let path = dir.join("session.jsonl");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        // 合法行 + 「汉」的前 2 字节残迹（第 3 字节被截）+ 换行 + 合法行
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(b"{\"type\":\"user-message\",\"text\":\"ahead\"}\n");
        body.extend_from_slice(&"汉".as_bytes()[..2]);
        body.extend_from_slice(b"\n");
        body.extend_from_slice(b"{\"type\":\"assistant-message\",\"text\":\"tail\"}\n");
        tokio::fs::write(&path, &body).await.unwrap();
        let store = JsonlStore::new(&path);
        let events = store.load_all().await.expect("残行跳过应成功不失败");
        assert_eq!(events.len(), 2, "两条合法行应读回，截断残行跳过");
        assert!(matches!(&events[0], SessionEvent::UserMessage { text } if text == "ahead"));
        assert!(matches!(&events[1], SessionEvent::AssistantMessage { text, .. } if text == "tail"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }


    /// ignorable 新旧互读回归（任务29 C2 验收）：混合文件（五类旧词汇+两枚命令
    /// 新词汇+未来未知+坏行）一次读取——新读取器全量读回新旧已知事件、跳过未知
    /// 与坏行；命令事件对只认五类词汇的旧宿主即「未知 tag」，与 future-event
    /// 同走 UnknownType 跳过路径（机制已在「未知事件与坏行容忍」实证，不另造旧版）。
    #[tokio::test]
    async fn 命令事件与旧词汇互读回归() {
        let dir = std::env::temp_dir().join(format!("yunyou-session-cmdmix-{}", std::process::id()));
        let path = dir.join("session.jsonl");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            &path,
            concat!(
                "{\"type\":\"user-message\",\"text\":\"旧词汇在先\"}\n",
                "{\"type\":\"command-run\",\"command_id\":\"u1\",\"name\":\"help\",\"raw_input\":\"/help\"}\n",
                "{\"type\":\"command-done\",\"command_id\":\"u1\",\"kind\":\"success\",\"text\":\"命令列表\"}\n",
                "{\"type\":\"future-event\",\"x\":1}\n",
                "{broken json\n",
                "{\"type\":\"assistant-message\",\"text\":\"旧词汇在后\"}\n",
            ),
        )
        .await
        .unwrap();
        let store = JsonlStore::new(&path);
        let events = store.load_all().await.expect("互读应成功不失败");
        assert_eq!(events.len(), 4, "四行已知事件应全读回（未知/坏行跳过）");
        assert!(matches!(&events[0], SessionEvent::UserMessage { text } if text == "旧词汇在先"));
        assert!(matches!(
            &events[1],
            SessionEvent::CommandRun { command_id, name, raw_input }
                if command_id == "u1" && name == "help" && raw_input == "/help"
        ));
        assert!(matches!(
            &events[2],
            SessionEvent::CommandDone { command_id, kind: CommandOutcome::Success, text }
                if command_id == "u1" && text == "命令列表"
        ));
        assert!(matches!(&events[3], SessionEvent::AssistantMessage { text, .. } if text == "旧词汇在后"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

/// V4（task-27 云枢裁量）：ignorable 双面锁——新宿主认得 compaction（往返）+
/// 「旧五类词汇白名单」不含 compaction（旧宿主跳过路径的文档性断言——防
/// 未来词汇表改动破兼容；行为退化不炸=ignorable 设计意图）。
#[test]
fn compaction_新宿主识别与旧宿主跳过双面() {
    let line = r#"{"type":"compaction","summary":"摘要正文","pre_tokens":100}"#;
    assert!(
        matches!(parse_line(line), Parsed::Event(SessionEvent::Compaction { summary, pre_tokens }) if summary == "摘要正文" && pre_tokens == 100),
        "新宿主应识别 compaction 事件"
    );
    // 旧宿主模拟：只认五类对话词汇的 matches! 白名单不含 compaction——
    // 落 UnknownType 跳过路径（既有「未知事件与坏行容忍」测试的行为面）。
    let old_host_known = matches!(
        Some("compaction"),
        Some("user-message" | "assistant-message" | "tool-call" | "tool-result" | "system-message")
    );
    assert!(!old_host_known, "旧五类白名单不得含 compaction（跳过路径前提）");
}
