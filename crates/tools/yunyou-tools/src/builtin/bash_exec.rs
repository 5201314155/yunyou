//! bash 执行主体（池 task-20 拆分：bash.rs 只留工具定义面，spawn/等待/移交/
//! 收割基建在此——行数红线 400 拆分，第四条铁律按职责切）。

use crate::builtin::capture::CaptureFile;
use crate::builtin::cwd::CwdService;
use crate::contract::{ToolExec, ToolOutcome};
use crate::tasks::{kill_process_group, OutputSink, TaskRegistry, TaskState};
use std::path::PathBuf;
use std::sync::Arc;

/// 单次输出截断上限。
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// 前台自动转后台默认阈值（秒；可配 [tools].background_after_secs，0=关）。
pub const BACKGROUND_AFTER_DEFAULT_SECS: u64 = 60;

/// 组收割 RAII：spawn 成功即持有，任意出口（正常返回/提前 return/panic
/// unwind/上层 abort future）drop 时全组 SIGKILL——补 [`std::process::
/// Child`] 系 kill_on_drop 只杀直子的孙进程盲区，drop 路径零孤儿。
#[cfg(unix)]
struct GroupReaper(libc::pid_t);

#[cfg(unix)]
impl Drop for GroupReaper {
    fn drop(&mut self) {
        unsafe {
            libc::killpg(self.0, libc::SIGKILL);
        }
    }
}

/// 子进程执行主体（独立函数便于取消语义集中）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_bash(
    cmd: String,
    start_dir: PathBuf,
    exec: ToolExec,
    cwd: Arc<CwdService>,
    prepared: Option<crate::sandbox::PreparedSandbox>,
    tasks: Option<Arc<TaskRegistry>>,
    background_after_secs: u64,
    run_in_background: bool,
) -> ToolOutcome {
    // cwd 捕获临时文件：RAII 兜底删除（取消/spawn 失败/wait 失败任何路径不留
    // 垃圾）；宿主预创建 0600 锁权（见 CaptureFile::new）；路径不在模型面——
    // 防伪不靠路径隐蔽（同机进程可枚举 /tmp/yunyou-cwd-* 通配），靠捕获值
    // clamp 的收益论证（见 builtin/cwd.rs；云镜 P2-A2 措辞降调）
    let capture = CaptureFile::new();
    // wrapper＝命令尾部追加三行宿主指令（门禁扫描之后拼接，模型不可见）：
    // 不用花括号包裹——花括号会打断 heredoc（终止标记后同行不能接 `; }`），
    // 追加行与任何命令形态（多行/heredoc/管道）正交；`exit $__yunyou_exit`
    // 保持原退出码语义。捕获文件缺席（预创建失败降级）＝不追加捕获行，命令
    // 原样执行。已知代价：cmd 以反斜杠续行结尾或 set -e 提前退出时捕获缺失
    // ＝cwd 维持不变（降级安全，note_uncaptured 留痕）。
    let wrapped = match capture.as_inner() {
        Some(path) => format!(
            "{cmd}\n__yunyou_exit=$?\npwd > '{}'\nexit $__yunyou_exit",
            path.display()
        ),
        None => cmd.clone(),
    };
    // 沙箱 helper 链（task-19 阶段 4）：prepared 在位→arg0 自唤起 helper
    //（fresh exec 应用 Landlock+seccomp 后 execvp bash，设计 §3.4 C 案）；
    // None（降级）→直跑+结果 meta 标注 sandboxed:false（设计 §3.6 降级显式告警）。
    // task-24 接线修正（死代码从未运行暴露的两处潜伏雷）：
    // ①命令向量必须带 shell 本体——helper execvp 的 command[0] 是用户命令，
    //   旧码传 ["-c", wrapped] 会 execvp("-c") 必 ENOENT；
    // ②arg0 哨兵分派——Command::new(program) 的 argv[0] 是程序路径而非哨兵名，
    //   必须 CommandExt::arg0 显式设哨兵，否则 try_helper 不命中、策略 JSON 被
    //   当普通 CLI 参数（finder 隐患形态：接 enforcer 不调 try_helper=喂 clap）。
    let (program, helper_args, sandboxed) = match &prepared {
        Some(p) => {
            let (program, argv) = p.helper_argv(&["bash".into(), "-c".into(), wrapped.clone()]);
            (program, argv, true)
        }
        None => (PathBuf::from("bash"), vec!["-c".into(), wrapped.clone()], false),
    };
    let mut command = if sandboxed {
        use std::os::unix::process::CommandExt;
        let mut c = tokio::process::Command::new(&program);
        // arg0=哨兵名（try_helper 入口协议），argv[1..]=--policy-json… 用户命令
        c.as_std_mut().arg0(&helper_args[0]);
        c.args(&helper_args[1..]);
        c
    } else {
        let mut c = tokio::process::Command::new(&program);
        c.arg("-c").arg(&helper_args[1]);
        c
    };
    #[cfg(unix)]
    command.process_group(0);
    command.kill_on_drop(true);
    command.current_dir(&start_dir);
    let mut child = match command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return ToolOutcome::err_h(
                "tool/spawn",
                format!("bash 启动失败: {err}"),
                "bash 启动失败，请重试或简化命令",
            );
        }
    };
    // 全组收割接线（task-14）：process_group(0) 使子进程自为组长（PGID=子
    // PID），spawn 即记组号——wait 完成后 `Child::id()` 变 None，组号必须
    // 先存；RAII reaper 覆盖 panic/上层 abort future 的 drop 路径，取消与
    // 正常完成路径显式收割（kill_on_drop 仅杀直子，为孙进程盲区，仅作
    // 非 unix 降级兜底保留）。
    let pgid = child.id().expect("spawn 成功后 pid 必在位");
    #[cfg(unix)]
    let group_reaper = GroupReaper(pgid as libc::pid_t);
    // 输出读取交给独立任务（句柄已从 child take，不阻塞取消路径）——
    // 池 task-20：读任务推 OutputSink（前台缓冲，转后台后直写日志文件）；
    // stdout/stderr 分双汇（stderr 单独承载沙箱 marker 判定），转后台时
    // 双汇并写同一日志文件。
    let sink_out = Arc::new(OutputSink::new());
    let sink_err = Arc::new(OutputSink::new());
    let mut stdout = child.stdout.take().expect("已设 piped");
    let mut stderr = child.stderr.take().expect("已设 piped");
    let out_task = {
        let sink = sink_out.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 8192];
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => sink.push(&buf[..n]),
                }
            }
        })
    };
    let err_task = {
        let sink = sink_err.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 8192];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => sink.push(&buf[..n]),
                }
            }
        })
    };
    // 池 task-20 转后台：显式参数立即转；否则前台三臂 select——wait/sleep(阈值)/
    // cancel。阈值 0 或注册表缺席=关自动转（双臂，行为与 task-20 前一致）。
    if run_in_background {
        if let Some(registry) = &tasks {
            // 移交=放弃前台收割权：RAII reaper 随 return 的 killpg 会把刚转
            // 后台的任务当场杀死（实测：移交即 done）——forget 移交收割权，
            // 后续由注册表/退场收割接管
            #[cfg(unix)]
            std::mem::forget(group_reaper);
            return handoff_background(
                &cmd, child, pgid, &sink_out, &sink_err, out_task, err_task, registry, &cwd,
            )
            .await;
        }
    }
    let wait_result = if !run_in_background && background_after_secs > 0 && tasks.is_some() {
        // 自动转后台窗口（转后台后 cancel 不再作用于该任务——设计裁决①边界：
        // 对话取消≠杀后台编译；已移交任务只由 task_stop 或会话退场收割）
        let tasks_for_wait = tasks.clone();
        let background_after_secs = background_after_secs;
        tokio::select! {
            result = child.wait() => Some(result),
            _ = tokio::time::sleep(std::time::Duration::from_secs(background_after_secs)) => {
                let registry = tasks_for_wait.expect("is_some 已判");
                // 同上：移交收割权，防 RAII reaper 杀死后台任务
                #[cfg(unix)]
                std::mem::forget(group_reaper);
                return handoff_background(
                    &cmd, child, pgid, &sink_out, &sink_err, out_task, err_task,
                    &registry, &cwd,
                )
                .await;
            }
            _ = exec.cancel.cancelled() => None,
        }
    } else {
        tokio::select! {
            result = child.wait() => Some(result),
            _ = exec.cancel.cancelled() => None,
        }
    };
    let status = match wait_result {
        Some(result) => result,
        None => {
            // 全组收割（孙进程一并死透，不复用孤儿）：先 KILL 后 reap；
            // RAII reaper 随 return 的再次 killpg 组空=ESRCH，幂等无害。
            #[cfg(unix)]
            kill_process_group(pgid);
            #[cfg(not(unix))]
            let _ = child.start_kill();
            let _ = child.wait().await;
            return ToolOutcome::err("tool/cancelled", "bash 执行被取消，进程已收割");
        }
    };
    // 正常完成也收割组内遗留（P2-9 + 零孤儿语义）：bash 已退，后台遗留
    // 孙进程可能仍持 stdout/stderr 写端——不杀则读任务永不 EOF、
    // 本函数永挂。组长已死，killpg 只命中遗留组员；组空=ESRCH。落款
    // （退出码/输出/cwd 捕获）在收割后照常取，正常完成路径零影响。
    #[cfg(unix)]
    kill_process_group(pgid);
    let _ = out_task.await;
    let _ = err_task.await;
    let out = sink_out.snapshot();
    let err_out = sink_err.snapshot();
    let status = match status {
        Ok(status) => status,
        Err(err) => {
            cwd.note_uncaptured();
            return ToolOutcome::err("tool/wait", format!("等待子进程失败: {err}"));
        }
    };
    // 沙箱 helper 失败归一（task-24 裁 5，设计 §3.6）：helper 预执行失败
    // （参数集不完整 64/策略不识别 65/沙箱应用失败 126）以 stderr 前缀 marker
    // 「yunyou-sandbox：」+专用退出码出现——归一 tool/sandbox-failed 拒执行，
    // 不回退裸跑。消歧：用户命令自身退出 64/65/126 无 marker=真实结果原样
    // 透传（设计区分：内核拒绝 EACCES/EPERM 是真实结果，非沙箱故障）。
    let err_text = String::from_utf8_lossy(&err_out);
    if sandboxed
        && matches!(status.code(), Some(64) | Some(65) | Some(126))
        && err_text.contains("yunyou-sandbox：")
    {
        cwd.note_uncaptured();
        return ToolOutcome::err(
            "tool/sandbox-failed",
            format!("沙箱应用失败，命令未执行（fail-closed 不回退裸跑）：{}", err_text.trim()),
        );
    }
    // 统一目录视图：命令结束后取子进程最终 pwd 写回共享服务（越界由服务
    // clamp 回 root 并留痕；捕获缺失＝命令提前退出或临时文件缺席降级，cwd
    // 维持不变）。空内容也按未捕获处理：预创建的空档被 `exit` 提前退出等
    // 场景留下（捕获行未执行），不是合法 pwd——不得走 clamp 回根。
    let captured = match capture.as_inner() {
        Some(path) => std::fs::read_to_string(path).ok().filter(|raw| !raw.trim().is_empty()),
        None => None,
    };
    match captured {
        Some(raw) => {
            cwd.set_captured(&raw);
        }
        None => cwd.note_uncaptured(),
    }
    let mut output = String::from_utf8_lossy(&out).into_owned();
    let err_text = String::from_utf8_lossy(&err_out);
    if !err_text.trim().is_empty() {
        output.push_str(&format!("\n[stderr]\n{err_text}"));
    }
    truncate(&mut output, MAX_OUTPUT_BYTES);
    ToolOutcome::ok(serde_json::json!({
        "exit_code": status.code(),
        "output": output,
        "sandboxed": sandboxed,
    }))
}


/// 转后台移交（池 task-20，设计三点全批）：输出双汇切落盘、注册表登记
/// （pgid 移交瞬间已抓取）、child/wait/pump 全权移交独立任务（wait→组内
/// 遗留收割→终态登记）。前台已缓冲段在 go_background 内先落盘。返回三要素
/// 闭环（lead 批注 b）：task_id+日志路径+怎么查。
#[allow(clippy::too_many_arguments)]
async fn handoff_background(
    cmd: &str,
    mut child: tokio::process::Child,
    pgid: u32,
    sink_out: &Arc<OutputSink>,
    sink_err: &Arc<OutputSink>,
    out_task: tokio::task::JoinHandle<()>,
    err_task: tokio::task::JoinHandle<()>,
    registry: &Arc<TaskRegistry>,
    cwd: &Arc<CwdService>,
) -> ToolOutcome {
    // 日志路径：<workspace>/.yunyou/tasks/<task_id>.log（裁决点②）
    let task_id = TaskRegistry::new_task_id();
    let dir = cwd.workspace_root().join(".yunyou").join("tasks");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        // 日志目录建不出来=后台形态落不了盘：不转后台的静默降级不可接受
        //（任务会变成无人可查的野进程）——收割后如实报错。
        #[cfg(unix)]
        kill_process_group(pgid);
        let _ = child.start_kill();
        let _ = child.wait().await;
        return ToolOutcome::err_h(
            "tool/io",
            format!("后台任务日志目录创建失败，命令未执行（fail-closed）：{err}"),
            "检查工作区写权限后重试；或去掉 run_in_background 前台执行",
        );
    }
    let log_path = dir.join(format!("{task_id}.log"));
    if let (Ok(()), Ok(())) = (
        sink_out.go_background(&log_path),
        sink_err.go_background(&log_path),
    ) {
        // 双汇并写同一文件成功；写失败（磁盘满等）也照常移交——日志缺段
        // 可从 log_path 排查，好过杀掉已经跑起来的命令
    }
    registry.register(&task_id, Some(pgid), log_path.clone(), cmd);
    // 后台任务 cwd 捕获不可得（它可能仍在 cd，捕获行随命令生命周期走）：
    // note_uncaptured 留痕，共享 cwd 维持命令转后台前的值。
    cwd.note_uncaptured();
    // 交接收割：wait→组内遗留收割（P2-9 同语义）→终态登记。out/err pump
    // 任务随管道 EOF 自然收尾，不 await（后台化=不阻塞本工具返回）。
    let registry = registry.clone();
    let id = task_id.clone();
    tokio::spawn(async move {
        let status = child.wait().await;
        #[cfg(unix)]
        kill_process_group(pgid);
        let code = status.ok().and_then(|s| s.code());
        registry.transition(&id, TaskState::Done, code);
        let _ = (out_task, err_task); // detach：EOF 后自行结束
    });
    ToolOutcome::ok(serde_json::json!({
        "backgrounded": true,
        "task_id": task_id,
        "log_path": log_path.display().to_string(),
        "message": format!(
            "命令已转后台运行（task_id={task_id}，日志 {}）：用 task_output(task_id) 轮询输出与状态（block_secs≤30 可等待），task_stop 终止，task_list 列全部",
            log_path.display()
        ),
    }))
}

/// 截断输出到上限（字符边界安全）。
pub(crate) fn truncate(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str("\n[输出已截断]");
}

