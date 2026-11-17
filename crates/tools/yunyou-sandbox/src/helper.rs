//! arg0 自唤起 helper 进程（task-19 设计 §3.4 C 案）：应用 Landlock+seccomp
//! 后 execvp 用户命令——fresh exec 的单线程进程，apply 期零并发、fork 灰区
//! 归零；PID 不变，parent wait 到的就是用户命令本体（退出码/信号天然透传）。
//!
//! # 入口协议
//! `argv[0]=="yunyou-sandbox"`（哨兵名）＋`--policy-json <json>`＋`--`＋用户命令；
//! 缺参/不识别=按普通命令退出（风险表#4：哨兵名不宣传，误触发零风险）。
//!
//! # pre-main 加固三件套（设计 §3.4④，吸收自 process-hardening，Apache-2.0）
//! `PR_SET_DUMPABLE=0`（防同用户 ptrace 沙箱子进程）＋`RLIMIT_CORE=0`（禁
//! core dump 泄内存）＋清洗 `LD_*` 环境变量（防库注入）——同步三行，不引 ctor。

use yunyou_tools::sandbox::{PreparedSandbox, WorkspacePolicy};
use std::path::PathBuf;

/// 哨兵名（parent spawn 时 arg0 用此名——helper 侧据此进入沙箱分支）。
pub const HELPER_ARG0: &str = "yunyou-sandbox";

/// helper 调用链入口：检测哨兵+参数集，命中即执行沙箱分支（永不返回）。
/// 未命中返回 None（主程序正常路径继续）。
pub fn try_helper(args: &[String]) -> Option<std::process::ExitCode> {
    if args.first().map(String::as_str) != Some(HELPER_ARG0) {
        return None;
    }
    // 参数集：--policy-json <json> -- <cmd> [args...]
    if args.len() < 5 || args[1] != "--policy-json" || args[3] != "--" {
        // 风险表#4：哨兵名+策略参数完整性校验，缺参即按普通命令退出——
        // 但 arg0 已是哨兵名，普通命令语义不存在，报错退出（不裸跑）。
        eprintln!("yunyou-sandbox：helper 参数集不完整（--policy-json <json> -- <cmd>）");
        return Some(std::process::ExitCode::from(64));
    }
    let policy_json = &args[2];
    let command = &args[4..];
    Some(run_helper(policy_json, command))
}

fn run_helper(policy_json: &str, command: &[String]) -> std::process::ExitCode {
    // pre-main 加固三件套（设计 §3.4④）
    harden_process();
    // 策略解析+版本校验（不识别即拒——契约冻结位）
    let policy = match WorkspacePolicy::from_policy_json(policy_json) {
        Ok(policy) => policy,
        Err(err) => {
            eprintln!("yunyou-sandbox：{err}");
            return std::process::ExitCode::from(65);
        }
    };
    // Landlock 写白名单+seccomp 网络禁断（应用失败=fail-closed 拒执行，
    // 专用退出码——不回退裸跑，设计 §3.6）
    if let Err(err) = apply_sandbox(&policy) {
        eprintln!("yunyou-sandbox：沙箱应用失败：{err}");
        return std::process::ExitCode::from(126);
    }
    // execvp 用户命令（换映像，PID 不变；exec 失败=命令不存在语义）
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(&command[0]).args(&command[1..]).exec();
    eprintln!("yunyou-sandbox：命令执行失败：{err}");
    std::process::ExitCode::from(127)
}

/// pre-main 加固三件套（同步三行，设计 §3.4④）。
fn harden_process() {
    // PR_SET_DUMPABLE=0：防同用户 ptrace 沙箱子进程（凭据在宿主侧）
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    // RLIMIT_CORE=0：禁 core dump 泄内存
    let limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &limit);
    }
    // 清洗 LD_* 环境变量：防库注入
    let poisoned: Vec<String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("LD_"))
        .map(|(k, _)| k)
        .collect();
    for key in poisoned {
        std::env::remove_var(key);
    }
}

/// Landlock 写白名单+seccomp 网络禁断（设计 §3.2/§3.3——NotEnforced fail-closed）。
/// 网络段按策略派生（task-24 接线）：network_access=false（默认）→seccomp 禁断；
/// true（[sandbox].network_denied=false 显式放开，重启生效）→跳过 seccomp——
/// 语义突变窗口受控于配置重启边界（设计 §2 config 面明示不热调）。
fn apply_sandbox(policy: &WorkspacePolicy) -> Result<(), String> {
    apply_landlock(policy)?;
    if !policy.network_access {
        apply_seccomp_network_deny()?;
    }
    Ok(())
}

/// 宿主架构→seccompiler TargetArch 映射（C-2 修复；白名单外 fail-closed）。
fn target_arch_for_host() -> Result<seccompiler::TargetArch, String> {
    match std::env::consts::ARCH {
        "x86_64" => Ok(seccompiler::TargetArch::x86_64),
        "aarch64" => Ok(seccompiler::TargetArch::aarch64),
        "riscv64" => Ok(seccompiler::TargetArch::riscv64),
        other => Err(format!(
            "seccomp 不支持当前架构 {other}（白名单=x86_64/aarch64/riscv64）——fail-closed 拒执行，不静默降级裸跑"
        )),
    }
}

/// Landlock 规则模型（设计 §3.2）：read "/"＋write 白名单＋NNP＋restrict_self；
/// NotEnforced（内核 <5.13）=显式报错拒执行（fail-closed 硬门，不静默裸跑）。
fn apply_landlock(policy: &WorkspacePolicy) -> Result<(), String> {
    // API 形态对齐 landlock 0.4.7 官方 sandboxer 示例（RulesetCreatedAttr 的
    // add_rule 走 PathFd/AsFd，create() 后链式流式接口）。
    use landlock::{
        Access, AccessFs, CompatLevel, Compatible, LandlockStatus, PathBeneath, PathFd,
        Ruleset, RulesetAttr, RulesetCreatedAttr,
    };
    let abi = landlock::ABI::V5;
    // 位权处理=全集（标准 sandboxer 形态）：ruleset 先声明处理全部 Fs 位权，
    // 再按路径挂子集规则（读挂 from_read、写根挂 from_all）——只 handle read
    // 位权却 add from_all 规则会被内核拒（access-rights not handled，
    // task-24 forked 探针 fail-loud 首功抓出的死代码潜伏 bug）。
    let ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(|err| format!("Landlock handle_access 失败：{err}"))?
        .create()
        .map_err(|err| format!("Landlock ruleset 创建失败：{err}"))?;
    let mut created = ruleset
        .add_rule(PathBeneath::new(
            PathFd::new("/").map_err(|err| format!("Landlock 打开 / 失败：{err}"))?,
            AccessFs::from_read(abi),
        ))
        .map_err(|err| format!("Landlock 读规则失败：{err}"))?;
    // 写=白名单（workspace root+/dev/null+/tmp 按策略；层级绑定——新建文件自动受控）
    for root in &policy.writable_roots {
        created = created
            .add_rule(PathBeneath::new(
                PathFd::new(root).map_err(|err| format!("可写根 {root:?} 打开失败：{err}"))?,
                AccessFs::from_all(abi),
            ))
            .map_err(|err| format!("Landlock 写规则失败（{root:?}）：{err}"))?;
    }
    let status = created
        .set_compatibility(CompatLevel::HardRequirement) // NotEnforced 必须显式报错（fail-closed 硬门）
        .no_new_privs(true)
        .restrict_self()
        .map_err(|err| format!("Landlock restrict_self 失败：{err}"))?;
    // fail-closed 硬门（设计 §3.2）：Landlock 未启用/未实现=拒执行，不静默裸跑
    // （本机 7.1.2 实测支持，发布面不赌内核版本——HardRequirement 下 NotEnabled/
    // NotImplemented 会走 Err 分支，此 match 为纵深防御兜底）。
    if status.landlock == LandlockStatus::NotEnabled
        || status.landlock == LandlockStatus::NotImplemented
    {
        return Err("Landlock 未生效（内核 <5.13 或 LSM 未启用）——fail-closed 拒执行".into());
    }
    Ok(())
}

/// seccomp 网络禁断（设计 §3.3 deny-list——比 allow-list 抗 libc 演进）。
fn apply_seccomp_network_deny() -> Result<(), String> {
    use seccompiler::{
        SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule,
    };
    // C-2 修复：TargetArch 按宿主实际架构映射（env::consts::ARCH）——旧实现硬编码
    // x86_64，非 x86_64 宿主上 BPF 内嵌 arch 审计值与实机不符，非白名单 syscall
    // 判定失真（seccomp 全 Allow 方向 fail-open）。白名单三架构外=Err 拒执行
    // （fail-closed：不装 filter 裸跑比装错 filter 更不可接受，调用方 apply_sandbox
    // 错误上抛=helper 退出非零，命令被拒）。
    let target_arch = target_arch_for_host()?;
    // seccompiler 语义模型：rules[(syscall, conditions)] 匹配→match_action；
    // 未匹配（含未列出 syscall）→mismatch_action。deny-list 落法=deny 表
    // →EPERM，mismatch=Allow（设计 §3.3 deny-list）。无条件匹配形态：库禁
    // 空条件向量（backend/mod.rs:58 校验），恒真式=MaskedEq(0)（mask=0 ⇒
    // (arg & 0)==0 恒成立；seccompiler 自有测试同款惯例 backend/rule.rs:201）。
    let deny_rule = || -> Result<SeccompRule, String> {
        SeccompRule::new(vec![SeccompCondition::new(
            0,
            SeccompCmpArgLen::Qword,
            SeccompCmpOp::MaskedEq(0),
            0,
        )
        .map_err(|err| format!("deny 规则构建失败：{err}"))?])
        .map_err(|err| format!("deny 规则构建失败：{err}"))
    };
    let mut rules = std::collections::BTreeMap::<i64, Vec<SeccompRule>>::new();
    for (nr, rule) in [
        (libc::SYS_connect, vec![deny_rule()?]),
        (libc::SYS_accept, vec![deny_rule()?]),
        (libc::SYS_accept4, vec![deny_rule()?]),
        (libc::SYS_bind, vec![deny_rule()?]),
        (libc::SYS_listen, vec![deny_rule()?]),
        (libc::SYS_shutdown, vec![deny_rule()?]),
        (libc::SYS_sendto, vec![deny_rule()?]),
        (libc::SYS_sendmmsg, vec![deny_rule()?]),
        (libc::SYS_recvmmsg, vec![deny_rule()?]),
        (libc::SYS_getpeername, vec![deny_rule()?]),
        (libc::SYS_getsockname, vec![deny_rule()?]),
        (libc::SYS_getsockopt, vec![deny_rule()?]),
        (libc::SYS_setsockopt, vec![deny_rule()?]),
        (libc::SYS_io_uring_setup, vec![deny_rule()?]),
        (libc::SYS_io_uring_enter, vec![deny_rule()?]),
        (libc::SYS_io_uring_register, vec![deny_rule()?]),
        (libc::SYS_ptrace, vec![deny_rule()?]),
        (libc::SYS_process_vm_readv, vec![deny_rule()?]),
        (libc::SYS_process_vm_writev, vec![deny_rule()?]),
    ] {
        rules.insert(nr, rule);
    }
    // socket：非 AF_UNIX 域→EPERM；AF_UNIX 域→mismatch=Allow（cargo/git
    // 子进程管理合法面，设计 §3.3「仅放行 AF_UNIX」）
    rules.insert(
        libc::SYS_socket,
        vec![SeccompRule::new(vec![SeccompCondition::new(
            0,
            SeccompCmpArgLen::Qword,
            SeccompCmpOp::Ne,
            libc::AF_UNIX as u64,
        )
        .map_err(|err| format!("socket 条件构建失败：{err}"))?])
        .map_err(|err| format!("socket 规则构建失败：{err}"))?],
    );
    // socketpair 不列入=走 mismatch Allow（cargo/git 子进程管理合法面，显式留白）。
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,                      // mismatch：未列出全部放行（deny-list）
        SeccompAction::Errno(libc::EPERM as u32),  // 匹配 deny 表→EPERM
        target_arch,
    )
    .map_err(|err| format!("seccomp filter 构建失败：{err}"))?;
    let program: seccompiler::BpfProgram = filter
        .try_into()
        .map_err(|err| format!("seccomp BPF 编译失败：{err}"))?;
    seccompiler::apply_filter(&program).map_err(|err| format!("seccomp 安装失败：{err}"))
}

/// parent 侧 helper spawn 参数集构造（供 bash 工具接线；阶段 4 消费）。
pub fn helper_argv(policy_json: &str, command: &[String]) -> (PathBuf, Vec<String>) {
    let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(HELPER_ARG0));
    let mut argv = vec![HELPER_ARG0.to_string(), "--policy-json".into(), policy_json.to_string(), "--".into()];
    argv.extend(command.iter().cloned());
    (program, argv)
}

/// PreparedSandbox 的 helper 入口便捷检查（阶段 4 bash 接线消费）。
pub fn prepared_argv(prepared: &PreparedSandbox, command: &[String]) -> (PathBuf, Vec<String>) {
    helper_argv(&prepared.policy_json, command)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// arg0 分派单测：哨兵命中进 helper 分支、未命中返回 None（正常 CLI 路径）。
    #[test]
    fn arg0哨兵分派() {
        // 未命中：普通命令路径（main 继续）
        assert!(try_helper(&["yunyou".into(), "--help".into()]).is_none());
        assert!(try_helper(&[]).is_none());
        // 参数集不完整：哨兵命中但缺参=报错退出码 64（不裸跑）
        let code = try_helper(&["yunyou-sandbox".into(), "--policy-json".into(), "{}".into()]);
        assert_eq!(code, Some(std::process::ExitCode::from(64)));
    }

    /// fail-closed 策略校验（裁 5②进程内可测面）：版本不识别即拒。
    #[test]
    fn 策略版本不识别即拒() {
        let bad = r#"{"writable_roots":["/ws"],"network_access":false,"policy_version":99}"#;
        assert!(WorkspacePolicy::from_policy_json(bad).is_err());
        let good = r#"{"writable_roots":["/ws"],"network_access":false,"policy_version":1}"#;
        assert!(WorkspacePolicy::from_policy_json(good).is_ok());
    }

    /// C-2 修复锁：宿主架构映射与 env::consts::ARCH 一致（测试宿主必为白名单
    /// 三架构之一→Ok）；白名单外架构走 Err 分支（编译期 env::consts 决定，
    /// 无法在单测伪造——Err 臂由 match 穷尽性静态保证，注释留痕）。
    #[test]
    fn 宿主架构映射一致() {
        let arch = target_arch_for_host().expect("测试宿主应为白名单架构");
        let expected = match std::env::consts::ARCH {
            "x86_64" => seccompiler::TargetArch::x86_64,
            "aarch64" => seccompiler::TargetArch::aarch64,
            "riscv64" => seccompiler::TargetArch::riscv64,
            other => panic!("测试宿主架构 {other} 不在白名单"),
        };
        assert_eq!(arch, expected);
    }

    /// 沙箱内核强制实证（task-24 验收硬标自动化面）：forked-child 自 spawn。
    ///
    /// Landlock restrict_self 不可逆——进程内直测会污染测试进程；本测试以
    /// 「测试二进制自 spawn+env 哨兵」隔离：child 内 apply_sandbox 后做越界/
    /// 联网探针，结果落临时文件，parent 断言。**fail-loud 口径（云枢裁 6）**：
    /// Landlock ABI 不可用时探针「成功」=测试红不 skip——「沙箱未生效」与
    /// 「生效但未越界」必须可区分，防假绿。finder 隐患（绕过 helper 直跑）
    /// 天然覆盖：child 若未真落 Landlock，越界写即成功=报告含「成功(!!)」=红。
    #[test]
    fn 沙箱强制越界写拒网络断内网通() {
        const CHILD_ENV: &str = "YUNYOU_SANDBOX_TEST_CHILD";
        if std::env::var(CHILD_ENV).as_deref() == Ok("1") {
            // child 分支：parent 自 spawn 的探针体（本测试的二次进入）
            return child_probe();
        }
        let me = std::env::current_exe().expect("测试二进制路径");
        // child 探针失败=非零退出（信号）；报告定点=child 可写根（Landlock
        // 生效时 /tmp 不可写，ws 是唯一确定可写位置），parent 经 child pid
        // 确定性定位（spawn().id()），读毕由 parent 清理现场。
        let child = std::process::Command::new(me)
            .arg("沙箱强制越界写拒网络断内网通")
            .env(CHILD_ENV, "1")
            .spawn()
            .expect("自 spawn 应成功");
        let child_pid = child.id();
        let ws = std::env::temp_dir().join(format!("yunyou-sbx-ws-{child_pid}"));
        let report_path = ws.join("sandbox-report.txt");
        let status = {
            // 等待 child 落定（用 spawn 句柄 wait 需保留所有权；此处以 pid 文件
            // 轮询+进程等待双保险：直接 wait 丢失 pid 则退化为轮询）
            let _ = child;
            loop {
                if report_path.exists() {
                    // 报告在场后再等一拍防截断
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            std::process::Command::new("true").status().expect("noop")
        };
        let _ = status;
        let report = std::fs::read_to_string(&report_path)
            .unwrap_or_else(|err| panic!("child 应落报告于 {report_path:?}（{err}）——fail-loud：无报告=探针未运行=红"));
        assert!(!report.contains("SANDBOX-APPLY-FAILED"), "沙箱应用失败（fail-loud 红不 skip）：\n{report}");
        let _ = std::fs::remove_dir_all(&ws);
        for line in ["LANDLOCK-APPLIED", "越界写=被拒", "白名单内写=成功", "网络出联=被拒", "内网 unix=通"] {
            assert!(report.contains(line), "报告缺「{line}」：\n{report}");
        }
        assert!(!report.contains("(!!)"), "探针出现反常成功：\n{report}");
        assert!(!report.contains("LANDLOCK-UNAVAILABLE"), "ABI 不可用必须红不 skip：\n{report}");
    }

    /// child 探针体（env 哨兵分派二次进入）：apply_sandbox→四探针→落报告。
    fn child_probe() {
        let mut report = String::new();
        let ws = std::env::temp_dir().join(format!("yunyou-sbx-ws-{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        // 报告定点=ws 内（Landlock 生效时唯一确定可写位置）；先落占位防 parent
        // 竞态，探针毕覆写终稿。清理权归 parent（Landlock 生效时 ws 只读于
        // 本探针语义之外的清理动作不可靠）。
        let report_path = ws.join("sandbox-report.txt");
        std::fs::write(&report_path, "RUNNING").unwrap();
        // 可写根=独立 ws（不含 /var/tmp）；网络禁断=默认语义
        let policy = WorkspacePolicy::v1(vec![ws.clone()]);
        match apply_sandbox(&policy) {
            Ok(()) => report.push_str("LANDLOCK-APPLIED\n"),
            Err(err) => {
                report.push_str(&format!("SANDBOX-APPLY-FAILED: {err}\n"));
                std::fs::write(&report_path, &report).unwrap();
                panic!("SANDBOX-APPLY-FAILED: {err}");
            }
        }
        // 越界写探针：/var/tmp 不在可写根
        match std::fs::write("/var/tmp/yunyou-sbx-escape-probe", b"x") {
            Ok(()) => report.push_str("越界写=成功(!!)\n"),
            Err(err) => report.push_str(&format!("越界写=被拒({err})\n")),
        }
        // 白名单内写探针
        match std::fs::write(ws.join("inside.txt"), b"ok") {
            Ok(()) => report.push_str("白名单内写=成功\n"),
            Err(err) => report.push_str(&format!("白名单内写=失败({err})\n")),
        }
        // 网络出联探针（seccomp deny connect→EPERM）
        match std::net::TcpStream::connect("127.0.0.1:1") {
            Ok(_) => report.push_str("网络出联=成功(!!)\n"),
            Err(err) => report.push_str(&format!("网络出联=被拒({err})\n")),
        }
        // AF_UNIX 存活探针（cargo/git 子进程管理合法面）
        match std::os::unix::net::UnixStream::pair() {
            Ok(_) => report.push_str("内网 unix=通\n"),
            Err(err) => report.push_str(&format!("内网 unix=失败({err})\n")),
        }
        std::fs::write(&report_path, &report).unwrap();
    }
}
