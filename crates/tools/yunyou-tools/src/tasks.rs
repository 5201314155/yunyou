//! 后台任务注册表（池 task-20）：bash 长命令后台化基建——任务表、输出落盘汇、
//! 全组收割、尾部窗口读取。设计=docs/设计/task-20-后台任务机制设计-20261118.md
//! （三点全批 2026-11-18：双入口转后台/落盘弃环形/随会话 Drop 收割无跨会话恢复）。
//!
//! # 生命周期
//! 注册表随 ToolsPlugin 装配实例存活（每会话一个），`Drop` 遍历 Running 任务
//! 逐组 killpg——孤儿活不过会话边界（收割复用 bash 域 killpg 基建，孙进程
//! 一并死透）。无跨会话恢复：进程被 kill -9 的遗留组不认领（claude 同款），
//! 日志文件留盘可人工排查。
//!
//! # 输出形态（裁决点②）
//! 落盘文件（`<workspace>/.yunyou/tasks/<task_id>.log`）弃内存环形缓冲：
//! 长编译数十 MB 零内存取舍、进程死后日志仍在（task_stop 后可查死前输出）、
//! claude diskOutput 同形态先例。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// task_output 尾部窗口默认值与上限（写进工具参数描述防模型傻等）。
pub(crate) const TAIL_DEFAULT_BYTES: usize = 4 * 1024;
pub(crate) const TAIL_MAX_BYTES: usize = 64 * 1024;
/// block_secs 上限（轮询等待语义，防模型傻等长阻塞）。
pub(crate) const BLOCK_SECS_MAX: u64 = 30;

/// 任务状态三态：running / done / stopped（stop 或退场收割）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Done,
    Stopped,
}

impl TaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Running => "running",
            TaskState::Done => "done",
            TaskState::Stopped => "stopped",
        }
    }
}

/// 单条后台任务登记。
#[derive(Debug, Clone)]
pub struct TaskEntry {
    /// 进程组 id（unix）；移交瞬间抓取（wait 后 Child::id 变 None）。
    /// None=非 unix 降级（无进程组语义，收割走 kill_on_drop 兜底）。
    pub pgid: Option<u32>,
    pub log_path: PathBuf,
    pub state: TaskState,
    pub started_at: Instant,
    /// 命令摘要（首 80 字符，task_list 人话面）。
    pub command: String,
    pub exit_code: Option<i32>,
}

/// 后台任务注册表：会话内全部后台任务（构造即 Arc 共享给 bash 与三管理工具）。
pub struct TaskRegistry {
    tasks: Mutex<BTreeMap<String, TaskEntry>>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self { tasks: Mutex::new(BTreeMap::new()) }
    }

    /// 生成 task_id：`bg-` + 六位 hex（纳秒时钟^pid 混合；会话内唯一性足够
    /// ——表内撞 id 重试由调用方 register 的返回值兜底不引入）。
    pub fn new_task_id() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
            .unwrap_or(0);
        let pid = std::process::id() as u64;
        format!("bg-{:06x}", (nanos ^ (pid << 17)) & 0xffffff)
    }

    /// 登记（移交瞬间调用；pgid 在此固定）。
    pub fn register(&self, id: &str, pgid: Option<u32>, log_path: PathBuf, command: &str) {
        let mut summary: String = command.chars().take(80).collect();
        if command.chars().count() > 80 {
            summary.push('…');
        }
        self.tasks.lock().expect("任务表锁").insert(
            id.to_string(),
            TaskEntry {
                pgid,
                log_path,
                state: TaskState::Running,
                started_at: Instant::now(),
                command: summary,
                exit_code: None,
            },
        );
    }

    /// 终态登记（pump 收尾/stop/退场收割）。
    pub fn transition(&self, id: &str, state: TaskState, exit_code: Option<i32>) {
        if let Some(entry) = self.tasks.lock().expect("任务表锁").get_mut(id) {
            entry.state = state;
            entry.exit_code = exit_code;
        }
    }

    pub fn get(&self, id: &str) -> Option<TaskEntry> {
        self.tasks.lock().expect("任务表锁").get(id).cloned()
    }

    pub fn list(&self) -> Vec<(String, TaskEntry)> {
        self.tasks.lock().expect("任务表锁").iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// 退场收割：全部 Running 任务逐组 killpg+标记 stopped（Drop 与显式清场
    /// 共用；组空=ESRCH 幂等）。
    pub fn harvest_all(&self) {
        let mut tasks = self.tasks.lock().expect("任务表锁");
        for entry in tasks.values_mut() {
            if entry.state != TaskState::Running {
                continue;
            }
            if let Some(pgid) = entry.pgid {
                kill_process_group(pgid);
            }
            entry.state = TaskState::Stopped;
        }
    }
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TaskRegistry {
    fn drop(&mut self) {
        self.harvest_all();
    }
}

/// 全组收割（unix，自 bash.rs 迁入复用）：`process_group(0)` 后子进程自为
/// 组长（PGID=子 PID），`killpg` 对整组发 SIGKILL——组内孙进程一并死透；
/// 组空=ESRCH 静默。幂等：重复调用无害。
#[cfg(unix)]
pub(crate) fn kill_process_group(pgid: u32) {
    // SIGKILL 直达：TERM 需进程配合退出，收割场景（取消/超时/退场）不假设
    // 组内进程体面配合——先 KILL 后 reap，无常宽限（任务书信号序裁决）。
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGKILL);
    }
}

/// 输出汇：前台内存缓冲→后台落盘切换（裁决点②）。读任务持续 push；
/// 转后台瞬间 go_background 把已缓冲段落盘，此后 chunk 直写文件。
pub struct OutputSink {
    background: AtomicBool,
    buffered: Mutex<Vec<u8>>,
    log: Mutex<Option<(PathBuf, std::fs::File)>>,
}

impl OutputSink {
    pub fn new() -> Self {
        Self {
            background: AtomicBool::new(false),
            buffered: Mutex::new(Vec::new()),
            log: Mutex::new(None),
        }
    }

    /// 切换后台：打开日志文件并把已缓冲段先落盘（移交点调用一次；重复调用
    /// 幂等——已切换则只补写缓冲段，文件句柄不重开）。
    pub fn go_background(&self, log_path: &std::path::Path) -> std::io::Result<()> {
        let mut buffered = self.buffered.lock().expect("缓冲锁");
        let mut log = self.log.lock().expect("日志锁");
        let file = match log.as_mut() {
            Some((_, file)) => file,
            None => {
                let file = std::fs::OpenOptions::new().create(true).append(true).open(log_path)?;
                let (_, file_ref) = log.insert((log_path.to_path_buf(), file));
                file_ref
            }
        };
        file.write_all(&buffered)?;
        file.flush()?;
        buffered.clear();
        drop(log);
        drop(buffered);
        self.background.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// 读任务推流：前台缓冲，后台直写文件（文件缺席=首次 push 重开，防
    /// go_background 与 push 竞态丢段）。
    pub fn push(&self, chunk: &[u8]) {
        if self.background.load(Ordering::SeqCst) {
            let mut log = self.log.lock().expect("日志锁");
            let slot = log.get_or_insert_with(|| {
                // 防御臂：go_background 未及调用而 push 先见 background 位——
                // 按约定路径重开（原子切换序：先置位后建文件则不会走到）
                let path = std::env::temp_dir().join("yunyou-bg-fallback.log");
                let file = std::fs::OpenOptions::new().create(true).append(true).open(&path);
                (path, file.expect("后台日志兜底文件应可创建"))
            });
            let _ = slot.1.write_all(chunk);
            let _ = slot.1.flush();
            return;
        }
        self.buffered.lock().expect("缓冲锁").extend_from_slice(chunk);
    }

    /// 前台完成路径取全量输出（既有 64KB 截断在 bash.rs 收口处）。
    pub fn snapshot(&self) -> Vec<u8> {
        self.buffered.lock().expect("缓冲锁").clone()
    }
}

impl Default for OutputSink {
    fn default() -> Self {
        Self::new()
    }
}

/// 尾部窗口读取：返回 (是否截断, 文本)。文件缺席=Ok((false, String::new()))
/// （任务刚移交未产出/被外部清理均属正常态，不报错）。
pub(crate) fn tail_read(path: &std::path::Path, max_bytes: usize) -> std::io::Result<(bool, String)> {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((false, String::new())),
        Err(err) => return Err(err),
    };
    if raw.len() <= max_bytes {
        return Ok((false, String::from_utf8_lossy(&raw).into_owned()));
    }
    let mut cut = raw.len() - max_bytes;
    // 字符边界安全（字节视图）：跳过 UTF-8 续字节（10xxxxxx）
    while cut < raw.len() && (raw[cut] & 0xC0) == 0x80 {
        cut += 1;
    }
    Ok((true, String::from_utf8_lossy(&raw[cut..]).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 输出汇切换：前台缓冲段在 go_background 时先落盘，后续 push 直写文件。
    #[test]
    fn 输出汇前台缓冲后台落盘() {
        let dir = std::env::temp_dir().join(format!("ytasks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("t1.log");
        let sink = OutputSink::new();
        sink.push(b"front-part\n");
        sink.go_background(&log).expect("切换应成功");
        sink.push(b"bg-part\n");
        let raw = std::fs::read(&log).expect("日志应存在");
        assert_eq!(raw, b"front-part\nbg-part\n", "缓冲段+后台段连续落盘");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 尾部窗口：小文件全量返回不标截断；大文件取尾+字符边界安全+截断标注。
    #[test]
    fn 尾部窗口与字符边界() {
        let dir = std::env::temp_dir().join(format!("ytasks2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("t2.log");
        std::fs::write(&log, "中".repeat(3000)).unwrap();
        let (trunc, text) = tail_read(&log, 1024).unwrap();
        assert!(trunc && text.chars().count() <= 1026, "尾部窗口取尾");
        let (trunc, text) = tail_read(&log, 64 * 1024).unwrap();
        assert!(!trunc && text.chars().count() == 3000, "小文件全量");
        assert_eq!(tail_read(&dir.join("absent.log"), 100).unwrap(), (false, String::new()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 注册表登记/终态/收割标记（收割 killpg 在白名单外路径组空=ESRCH 无害）。
    #[test]
    fn 注册表登记终态与收割() {
        let reg = TaskRegistry::new();
        let id = TaskRegistry::new_task_id();
        assert!(id.starts_with("bg-"));
        reg.register(&id, Some(u32::MAX), PathBuf::from("/tmp/x.log"), "cargo build");
        assert_eq!(reg.get(&id).unwrap().state, TaskState::Running);
        assert!(reg.get(&id).unwrap().command.starts_with("cargo build"));
        reg.transition(&id, TaskState::Done, Some(0));
        assert_eq!(reg.get(&id).unwrap().state, TaskState::Done);
        let id2 = TaskRegistry::new_task_id();
        reg.register(&id2, Some(u32::MAX), PathBuf::from("/tmp/y.log"), "sleep 1000");
        reg.harvest_all();
        assert_eq!(reg.get(&id2).unwrap().state, TaskState::Stopped, "Running 被收割标记");
        assert_eq!(reg.get(&id).unwrap().state, TaskState::Done, "终态不被收割改写");
        assert_eq!(reg.list().len(), 2);
    }
}
