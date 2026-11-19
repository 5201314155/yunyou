//! # yunyou-team-procmgr —— K-11 团队运行时·进程管理层（三层之二）
//!
//! **职责**：进程表（worker 注册:pid+capability+session_id/心跳超时感知/死亡
//! 事件发布）+收割（状态收敛事件产出——board 回 pending/hub 通报由消费方执
//! 行，本 crate 只产事件=纯进程内核心可单测）。
//!
//! **明确不做**：spawn（launcher 职责）/board 直写（team service 消费
//! DeathEvent 执行收敛）/hub 直发（同）。
//!
//! 对表锚点：K-11 设计案 85547bfae §一§二§三（心跳 30s×3/PDEATHSIG 互补/
//! 收割收敛序四步）。

/// 进程表条目（worker 注册态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcEntry {
    pub pid: u32,
    /// 连接会话标识（身份锚点，spawn argv --session 透传）。
    pub session_id: String,
    /// capability 标签（任务分派匹配面，K-17 词汇复用）。
    pub capability: String,
    /// 最后心跳毫秒时戳（调用方 now_ms 注入，可测）。
    pub last_heartbeat_ms: u64,
}

/// 死亡事件（收割产出；消费方=team service 收敛 board+hub 通报）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeathEvent {
    pub pid: u32,
    pub session_id: String,
    /// 死亡原因（心跳超时/进程退出收割）。
    pub reason: DeathReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathReason {
    /// 心跳超时（连续 3 次未达，设计 §二）。
    HeartbeatTimeout,
    /// 进程退出被收割（waitpid 面，退出码透传）。
    Exited(i32),
}

/// 进程管理核心（纯进程内状态机；进程级 waitpid=集成面，本层零系统调用）。
#[derive(Debug)]
pub struct ProcMgr {
    procs: std::collections::BTreeMap<u32, ProcEntry>,
    /// 心跳超时毫秒（默认 90_000=30s×3，设计 §二）。
    heartbeat_timeout_ms: u64,
}

impl ProcMgr {
    pub fn new(heartbeat_timeout_ms: u64) -> Self {
        Self { procs: std::collections::BTreeMap::new(), heartbeat_timeout_ms }
    }

    /// 注册 worker（spawn 成功后登记；重复 pid=幂等覆盖——重启 worker 同 pid
    /// 场景收敛为更新）。
    pub fn register(&mut self, pid: u32, session_id: &str, capability: &str, now_ms: u64) {
        self.procs.insert(
            pid,
            ProcEntry {
                pid,
                session_id: session_id.to_string(),
                capability: capability.to_string(),
                last_heartbeat_ms: now_ms,
            },
        );
    }

    /// 心跳到达（刷新时戳；未知 pid=Err——孤儿心跳=异常面，人话报）。
    pub fn heartbeat(&mut self, pid: u32, now_ms: u64) -> Result<(), String> {
        match self.procs.get_mut(&pid) {
            Some(entry) => {
                entry.last_heartbeat_ms = now_ms;
                Ok(())
            }
            None => Err(format!("未知 worker pid={pid} 的心跳（未注册或已收割）")),
        }
    }

    /// 心跳超时扫描（调用方周期驱动）：超时条目摘除并产死亡事件。
    pub fn sweep_timeouts(&mut self, now_ms: u64) -> Vec<DeathEvent> {
        let timeout = self.heartbeat_timeout_ms;
        let dead: Vec<u32> = self
            .procs
            .iter()
            .filter(|(_, e)| now_ms.saturating_sub(e.last_heartbeat_ms) > timeout)
            .map(|(pid, _)| *pid)
            .collect();
        dead.into_iter()
            .filter_map(|pid| self.procs.remove(&pid))
            .map(|e| DeathEvent {
                pid: e.pid,
                session_id: e.session_id,
                reason: DeathReason::HeartbeatTimeout,
            })
            .collect()
    }

    /// 进程退出收割（waitpid 面：集成层收割到退出→本层摘除产事件）。
    pub fn reap_exit(&mut self, pid: u32, exit_status: i32) -> Option<DeathEvent> {
        self.procs.remove(&pid).map(|e| DeathEvent {
            pid: e.pid,
            session_id: e.session_id,
            reason: DeathReason::Exited(exit_status),
        })
    }

    /// 存活清单（观测面：/team 面板/lead 修复链查询）。
    pub fn alive(&self) -> Vec<ProcEntry> {
        self.procs.values().cloned().collect()
    }

    pub fn heartbeat_timeout_ms(&self) -> u64 {
        self.heartbeat_timeout_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 注册心跳与超时扫描() {
        let mut mgr = ProcMgr::new(90_000);
        mgr.register(100, "s1", "review", 1_000);
        mgr.register(101, "s2", "coding", 1_000);
        assert_eq!(mgr.alive().len(), 2);
        mgr.heartbeat(100, 50_000).expect("已知 pid 心跳应刷新");
        let deaths = mgr.sweep_timeouts(100_000);
        assert_eq!(deaths.len(), 1, "仅 101 超时");
        assert_eq!(deaths[0].pid, 101);
        assert_eq!(deaths[0].reason, DeathReason::HeartbeatTimeout);
        assert_eq!(mgr.alive().len(), 1, "超时条目已摘除");
        assert!(mgr.alive()[0].pid == 100);
    }

    #[test]
    fn 孤儿心跳人话拒() {
        let mut mgr = ProcMgr::new(90_000);
        let err = mgr.heartbeat(999, 1_000).unwrap_err();
        assert!(err.contains("未知 worker"), "{err}");
    }

    #[test]
    fn 退出收割产出Exited事件() {
        let mut mgr = ProcMgr::new(90_000);
        mgr.register(200, "s3", "coding", 1_000);
        let death = mgr.reap_exit(200, 137).expect("在册 pid 收割应产事件");
        assert_eq!(death.reason, DeathReason::Exited(137));
        assert_eq!(death.session_id, "s3");
        assert!(mgr.alive().is_empty(), "收割后摘除");
        assert!(mgr.reap_exit(200, 0).is_none(), "重复收割=None 幂等");
    }

    #[test]
    fn 重启worker同pid幂等覆盖() {
        let mut mgr = ProcMgr::new(90_000);
        mgr.register(300, "s-old", "coding", 1_000);
        mgr.register(300, "s-new", "review", 2_000);
        let alive = mgr.alive();
        assert_eq!(alive.len(), 1, "同 pid 重启=覆盖非并存");
        assert_eq!(alive[0].session_id, "s-new", "新会话身份生效");
    }
}
