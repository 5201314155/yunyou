//! 跨进程会话排他锁（task-36①，方案 A）：锁体=`<会话文件>.lock`，
//! 原语=`flock(LOCK_EX|LOCK_NB)`——内核级锁随进程死亡自动释放（SIGKILL 亦然，
//! 零尸锁）；装配期（会话插件 setup）抢锁失败即装配失败，双实例并发写同一会话
//! 被启动期拦下。
//!
//! 锁文件体持久（进程退出后 `.lock` 空壳残留，锁态随进程不在文件体）——
//! 不 unlink 防 TOCTOU；未来会话列举（--resume 类）须排除 `*.lock`。

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use crate::jsonl::file_error;
use yunyou::{CordisError, ErrorKind, Result};

/// 会话排他锁句柄：持有即锁存活；句柄 Drop 或进程退出即自动释放。
#[derive(Debug)]
pub struct ExclusiveLock {
    /// 锁 fd：flock 绑定其 open file description，Drop 关闭即释放。
    _file: File,
}

impl ExclusiveLock {
    /// 对 `<session_path>.lock` 取排他锁（非阻塞）。
    ///
    /// 抢锁失败=另一实例持有，报人话错误（现象+锁路径+行动项）。
    pub fn acquire(session_path: &Path) -> Result<Self> {
        let lock_path = lock_path_of(session_path);
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| file_error(&lock_path, "创建会话目录失败", err))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|err| file_error(&lock_path, "打开会话锁文件失败", err))?;
        // 排他试锁（非阻塞）：抢不到=另一实例持有；flock 随 fd 存活、随进程死自动释放
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err(CordisError::new(
                ErrorKind::Internal,
                format!(
                    "会话正被另一个 yunyou 实例使用：{}（锁 {}）。\n\
                     出路任选：① yunyou --continue 续最近会话，或 --resume 选择/指定其他会话；\
                     ② yunyou --fork 复制本会话为新会话继续（不影响正在使用它的实例）；\
                     ③ 关闭占用实例后重试。",
                    session_path.display(),
                    lock_path.display()
                ),
            ));
        }
        Ok(Self { _file: file })
    }

    /// 探测会话是否正被持锁（不持久占用：试锁成功即立即释放）。
    ///
    /// 用途=启动期预检（task-9）：`--fork` 据此决定是否复制会话绕开冲突；
    /// 试锁成功（未被锁）时 fd 随即 Drop 释放，与后续 `acquire` 无竞态窗口
    /// 恶化（真锁仍在装配期抢，预检只是提前量）。
    pub fn is_locked(session_path: &Path) -> bool {
        let lock_path = lock_path_of(session_path);
        let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
        else {
            return false; // 打不开（权限等）不预设被锁，真锁判断交给 acquire
        };
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        rc != 0 // 0=试锁成功即未锁（fd Drop 自动释放）；非 0=被持
    }
}

/// 会话文件对应的锁文件路径：`<会话>.jsonl` → `<会话>.jsonl.lock`。
fn lock_path_of(session_path: &Path) -> PathBuf {
    let mut os = session_path.as_os_str().to_os_string();
    os.push(".lock");
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use crate::jsonl::JsonlStore;
    use super::ExclusiveLock;

    /// 跨进程排他锁（task-36① 方案 A）：同一路径第二把锁立即失败且报人话；
    /// 首锁释放（进程内=句柄 Drop）后可重取——flock 按 open file description 互斥，
    /// 进程内行为与跨进程一致。
    #[test]
    fn 排他锁同路径互斥与释放() {
        let dir = std::env::temp_dir().join(format!("yunyou-session-lock-{}", std::process::id()));
        let path = dir.join("session.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = JsonlStore::new(&path);
        first.try_lock_exclusive().expect("首锁应成功");
        let second = JsonlStore::new(&path);
        let err = second
            .try_lock_exclusive()
            .expect_err("同路径第二把锁应失败");
        assert!(
            err.to_string().contains("另一个 yunyou 实例"),
            "人话文案应含指向: {err}"
        );
        drop(first);
        second
            .try_lock_exclusive()
            .expect("首锁释放后应可重取");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-9：撞锁错误人话三出路——错误信息须给 --continue/--resume、
    /// --fork 复制、关闭占用实例三条行动项（现象+锁路径不变）。
    #[test]
    fn 撞锁错误给三条出路() {
        let dir = std::env::temp_dir().join(format!("yunyou-session-lockmsg-{}", std::process::id()));
        let path = dir.join("session.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = JsonlStore::new(&path);
        first.try_lock_exclusive().expect("首锁应成功");
        let second = JsonlStore::new(&path);
        let err = second.try_lock_exclusive().expect_err("应撞锁");
        let text = err.to_string();
        assert!(text.contains("--continue"), "应指路续聊: {text}");
        assert!(text.contains("--resume"), "应指路恢复: {text}");
        assert!(text.contains("--fork"), "应指路复制新会话: {text}");
        assert!(text.contains("关闭占用实例"), "应给关闭出路: {text}");
        assert!(text.contains(&dir.to_string_lossy().as_ref()) || text.contains("session.jsonl"), "应含锁路径: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-9：is_locked 探测——被持时 true；释放后 false；不持久占用锁
    /// （探测后 acquire 仍可成功）。
    #[test]
    fn 探测不占用锁() {
        let dir = std::env::temp_dir().join(format!("yunyou-session-probe-{}", std::process::id()));
        let path = dir.join("session.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!ExclusiveLock::is_locked(&path), "无锁=未锁");
        let holder = ExclusiveLock::acquire(&path).expect("应可取锁");
        assert!(ExclusiveLock::is_locked(&path), "被持=已锁");
        drop(holder);
        assert!(!ExclusiveLock::is_locked(&path), "释放后=未锁");
        // 探测（试锁即释）不持久占用：随后 acquire 仍成功
        let _ = ExclusiveLock::is_locked(&path);
        ExclusiveLock::acquire(&path).expect("探测后仍应可取锁");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
