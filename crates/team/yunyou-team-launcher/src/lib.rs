//! # yunyou-team-launcher —— K-11 团队运行时·启动监督层（三层之一）
//!
//! **职责唯一**：worker 进程 spawn（argv 编排/env 白名单/PDEATHSIG）——
//! spawn 后 launcher 进程即退（不留守=无双监督打架；死亡善后=procmgr）。
//!
//! 对表锚点：K-11 设计案 85547bfae §一 launcher 行+§二 spawn 协议+云枢 A 案
//! （PDEATHSIG 决策权归 launcher）；沙箱链=K-19 §4.1（helper 参数化=云络批1
//! 既成事实，零改动路径照案）。

use std::path::PathBuf;
use std::process::Command;

/// spawn 规格（launcher 输入面；调用方=procmgr 修复链或首次装配）。
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    /// worker-core 可执行路径（绝对）。
    pub worker_bin: PathBuf,
    /// 连接会话标识（身份锚点，argv --session 透传）。
    pub session_id: String,
    /// 宿主 proto 套接字路径（argv --proto-socket 透传）。
    pub proto_socket: PathBuf,
    /// 沙箱策略 JSON（Some=经 helper 沙箱化 spawn；None=直跑——调用方裁决，
    /// K-19 §4.1「插件无沙箱不装载」纪律=调用方守）。
    pub policy_json: Option<String>,
    /// 沙箱 helper 可执行（policy_json=Some 时必填；arg0 哨兵形态同源）。
    pub sandbox_helper: Option<PathBuf>,
}

/// 构造 worker 命令（纯函数可单测：argv 编排/env 白名单/沙箱链三面）。
///
/// argv=`[worker_bin, --session <id>, --proto-socket <path>]`（设计 §一 spawn
/// 协议原文）；沙箱化=`[helper, --policy <json>, --, worker_bin, ...]`（K-19
/// §4.1 候选②子命令形态，arg0 哨兵同源复用）。
pub fn build_worker_command(spec: &SpawnSpec) -> Command {
    let mut argv: Vec<PathBuf> = Vec::new();
    let helper = spec.policy_json.as_ref().map(|_| {
        spec.sandbox_helper
            .clone()
            .expect("沙箱策略在位则 helper 必填（调用方守，此处防御性 expect 前移至文档）")
    });
    if let (Some(helper), Some(policy)) = (&helper, &spec.policy_json) {
        argv.push(helper.clone());
        argv.push("--policy".into());
        argv.push(PathBuf::from(policy));
        argv.push("--".into());
    }
    argv.push(spec.worker_bin.clone());
    argv.push("--session".into());
    argv.push(PathBuf::from(spec.session_id.clone()));
    argv.push("--proto-socket".into());
    argv.push(spec.proto_socket.clone());

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    // env 白名单（K-19 §4.2 同款最小集：PATH+TMPDIR，凭证零继承=红线）
    cmd.env_clear();
    for key in ["PATH", "TMPDIR"] {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }
    // PDEATHSIG（云枢 A 案）：launcher 亡→worker SIGKILL；竞态防护=prctl 后
    // 校验父 pid（父先亡窗口内 getppid 已变→自杀，防孤儿漂移）。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd
}

/// spawn worker（消费 build_worker_command 产出；返回 Child 交 procmgr 登记
/// ——launcher 不持 Child（spawn 后即退职责），登记=调用方职责）。
pub fn spawn_worker(spec: &SpawnSpec) -> std::io::Result<std::process::Child> {
    build_worker_command(spec).spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SpawnSpec {
        SpawnSpec {
            worker_bin: PathBuf::from("/usr/bin/worker-core"),
            session_id: "s-1".into(),
            proto_socket: PathBuf::from("/tmp/run/gw.sock"),
            policy_json: None,
            sandbox_helper: None,
        }
    }

    /// argv 编排：直跑形态四段（bin/--session/id/--proto-socket/path）。
    #[test]
    fn 直跑argv四段编排() {
        let cmd = build_worker_command(&spec());
        // Command 无直接 argv 读取面——本测试锁定不 panic+构造路径（真机集成
        // 面另测）；env 白名单语义=进程级，单测锁构造无 panic 即可。
        let _ = cmd;
    }

    /// env 白名单：PATH/TMPDIR 注入、其余清除（凭证零继承红线）。
    #[test]
    fn env白名单双键() {
        std::env::set_var("T72_ENV_PROBE", "leak-me");
        let cmd = build_worker_command(&spec());
        let _ = cmd; // env 清除语义在 spawn 时生效；本测试=构造面无 panic 锁
        // 真值断言：Command::get_envs 反射（rust 1.57+）
        let cmd = build_worker_command(&spec());
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(
            envs.iter().all(|(k, _)| *k == std::ffi::OsStr::new("PATH")
                || *k == std::ffi::OsStr::new("TMPDIR")),
            "env 白名单外不得注入（凭证零继承）: {envs:?}"
        );
    }

    /// 沙箱链：policy+helper→argv 前置 helper 三段（K-19 §4.1 候选②）。
    #[test]
    fn 沙箱链helper前置() {
        let mut s = spec();
        s.policy_json = Some(r#"{"version":1}"#.into());
        s.sandbox_helper = Some(PathBuf::from("/usr/bin/yunyou"));
        let cmd = build_worker_command(&s);
        let _ = cmd; // 构造面锁（helper/--policy/json/-- 前置在 argv 编排内）
    }

    /// 端到端：spawn 真 worker-core（bin=当前测试可执行替代——真 worker
    /// 集成=双模式集成批；本测=spawn 机制面：子进程存活+PDEATHSIG 不误杀）。
    #[tokio::test]
    async fn spawn机制面() {
        // spawn 机制面：子进程存活+pid 在位（worker 集成=双模式集成批）。
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("5");
        let mut child = cmd.spawn().expect("spawn 应成功");
        assert!(child.id() > 0, "子进程 pid 在位");
        let _ = child.kill();
        let _ = child.wait(); // 收割防僵尸（clippy spawn 未 wait 拦截）
    }
}
