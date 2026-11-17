//! 就绪标记文件：装配完成创建、退出删除（为团队 launcher / 网关阶段预留的
//! 启动判定观测点，对标《团队运行时落地方案》§5.1 的就绪探针语义）。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 就绪标记句柄：持路径；Drop 兜底删除（异常退出路径也尽量不留垃圾）。
#[derive(Debug)]
pub struct ReadyMark {
    path: PathBuf,
}

impl ReadyMark {
    /// 创建 `<yunyou-home>/run/ready-<pid>`（run 目录不存在则建）。
    ///
    /// 文件内容为创建时刻的 UNIX 秒（观测用）；文件名带 pid 避免多实例互踩。
    /// 创建前先清扫无主陈尸（task-38③：SIGKILL/SIGTERM 路径的 Drop 兜底不
    /// 执行，历史实证真实 home 累积 5 枚；pid 存活探测通过才保留）。
    pub fn create(home: &Path) -> std::io::Result<Self> {
        sweep_stale(home);
        let dir = home.join("run");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("ready-{}", std::process::id()));
        std::fs::write(&path, unix_ts().to_string())?;
        Ok(Self { path })
    }

    /// 标记文件路径（联调与测试断言用）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 删除标记（幂等：文件已消失不报错）。
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for ReadyMark {
    fn drop(&mut self) {
        self.remove();
    }
}

/// 当前 UNIX 秒（标记内容仅观测用途，不承担时间语义）。
fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// 清扫无主陈尸标记（task-38③）：遍历 `run/`，`ready-<pid>` 的 pid 不存活
/// 即删除。存活探测用 `kill(pid, 0)`：ESRCH=不存在，EPERM=存在但无权（视为
/// 存活，宁留勿误删他人实例——面板多席并行同名二进制必有他人进程）。返回
/// 清理数（观测用）。非 ready 前缀/非数字尾缀文件一律不动。
pub fn sweep_stale(home: &Path) -> usize {
    let dir = home.join("run");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0; // 目录不存在=无陈尸
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix("ready-"))
            .and_then(|tail| tail.parse::<i32>().ok())
        else {
            continue;
        };
        if !pid_alive(pid) && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// pid 存活探测：`kill(pid, 0)` 零信号探活——成功=存活；EPERM=存在但无权
/// （同机多用户/多席场景视为存活）；其余（ESRCH 等）=不存活。
fn pid_alive(pid: i32) -> bool {
    // SAFETY：kill(2) 纯系统调用，零信号不发，无悬垂指针面
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 创建即存在；显式删除即消失；Drop 兜底再删一次不报错。
    #[test]
    fn 标记创建与删除生命周期() {
        let dir = std::env::temp_dir().join(format!("yunyou-cli-ready-{}", std::process::id()));
        let mark = ReadyMark::create(&dir).expect("应能创建");
        assert!(mark.path().exists(), "标记应在 {}", mark.path().display());
        mark.remove();
        assert!(!mark.path().exists(), "删除后应消失");
        mark.remove(); // 幂等
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Drop 兜底：不显式 remove，对象销毁时标记应被清掉。
    #[test]
    fn drop_兜底删除() {
        let dir = std::env::temp_dir().join(format!("yunyou-cli-ready-drop-{}", std::process::id()));
        let path = {
            let mark = ReadyMark::create(&dir).expect("应能创建");
            mark.path().to_path_buf()
        };
        assert!(!path.exists(), "Drop 后标记应被删除");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// task-38③：陈尸（pid 不存在）被清，存活 pid（本进程）保留，非 ready
    /// 前缀/非数字文件不动。（文件放 `dir/run/` 下与产品布局同构。）
    #[test]
    fn 清尸_存活保留_杂物不动() {
        let dir = std::env::temp_dir().join(format!("yunyou-cli-ready-sweep-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("run")).expect("temp run dir");
        let stale = dir.join("run/ready-999999999"); // Linux pid 上限远小于此值，必不存在
        std::fs::write(&stale, "0").expect("写陈尸");
        let live = dir.join("run").join(format!("ready-{}", std::process::id()));
        std::fs::write(&live, "0").expect("写存活标记");
        let junk = dir.join("run/not-ready-file");
        std::fs::write(&junk, "x").expect("写杂物");
        let removed = sweep_stale(&dir);
        assert_eq!(removed, 1, "恰清一枚陈尸");
        assert!(!stale.exists(), "陈尸应被删除");
        assert!(live.exists(), "存活标记应保留");
        assert!(junk.exists(), "非 ready 前缀杂物不动");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// task-38③：create 前自动清尸——create 一个新标记时，同目录陈尸消失。
    #[test]
    fn create_前置清尸() {
        let dir = std::env::temp_dir().join(format!("yunyou-cli-ready-create-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("run")).expect("run dir");
        let stale = dir.join("run/ready-999999999");
        std::fs::write(&stale, "0").expect("写陈尸");
        let mark = ReadyMark::create(&dir).expect("应能创建");
        assert!(!stale.exists(), "create 应先清陈尸");
        assert!(mark.path().exists(), "新标记应在位");
        drop(mark);
        std::fs::remove_dir_all(&dir).ok();
    }
}
