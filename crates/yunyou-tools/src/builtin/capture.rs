//! bash pwd 捕获临时文件（task-33② 建、task-52 拆分归位）：RAII 生命周期
//! 与 0600 创建即锁权——职责独立于命令执行主体（家规第四条拆分；
//! bash.rs 403 行贴顶由此件外迁回落）。

use std::path::PathBuf;

/// cwd 捕获临时文件（RAII：Drop 即删，任何退出路径不留垃圾）。
///
/// `None`＝预创建失败（撞名/磁盘异常）→ 本轮无捕获（bash 照常执行，退出码
/// 穿透，cwd 维持不变走 note_uncaptured 留痕）——临时文件建不出来不让 bash
/// 整体不可用，降级安全。
pub(crate) struct CaptureFile(Option<PathBuf>);

impl CaptureFile {
    /// 预创建 0600（对齐 store.rs `write_secret_file` 惯用法「创建即锁权限」；
    /// 云镜 P2-A2）：bash `>` 对已存在文件是 O_TRUNC 截断写、**不重设权限**——
    /// 宿主先以 0600 建档，子进程重定向只截断不提权；不预创建则权限由子进程
    /// umask 决定（典型 0644＝同机其他用户可读捕获值，信息面无谓扩大）。
    /// `create_new` 防撞名误写：pid+纳秒撞名（上次进程崩溃残留）时换名重试，
    /// 三次仍撞或创建失败＝放弃捕获（None 降级）。
    /// 内部路径槽（bash.rs 三处 match 用； RAII 删除仍在 Drop）。
    pub(crate) fn as_inner(&self) -> &Option<std::path::PathBuf> {
        &self.0
    }

    pub(crate) fn new() -> Self {
        let pid = std::process::id();
        for attempt in 0u32..3 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
                + u128::from(attempt);
            let path = std::env::temp_dir().join(format!("yunyou-cwd-{pid}-{nanos}"));
            match create_private_file(&path) {
                Ok(()) => return Self(Some(path)),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => break,
            }
        }
        Self(None)
    }
}

/// 0600 独占建档（unix `mode(0o600)` 创建即锁权；非 unix 无 POSIX 权限位，
/// 退化为 create_new 防撞名创建）。
pub(crate) fn create_private_file(path: &PathBuf) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map(|_| ())
    }
    #[cfg(not(unix))]
    std::fs::OpenOptions::new().write(true).create_new(true).open(path).map(|_| ())
}

impl Drop for CaptureFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}
