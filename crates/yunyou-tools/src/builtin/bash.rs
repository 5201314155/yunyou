//! bash 工具：子进程执行命令（独立进程组 + 全组收割 + 取消观察 + 执行后捕获
//! pwd 回写共享 CWD 服务；对标 dsh 的 shell 域裁剪）。
//!
//! # 契约
//! - 取消：观察 [`ToolExec::cancel`]，触发即 **全组收割**（killpg，含孙进程）
//!   并返回 `tool/cancelled`；
//! - 进程组：子进程独立进程组（unix，自为组长）。**收割语义：命令结束/取消/
//!   上层放弃（future drop）任一路径都对全组 SIGKILL**——组长已死时组员
//!   （后台遗留孙进程）一并收割，零孤儿；组空=ESRCH 静默幂等。因此
//!   「故意后台长驻」不在本工具契约内（`nohup … &` 类命令的后代随命令结束
//!   死亡）：工具是一次性命令执行体，模型面需要长驻服务应走宿主进程管理，
//!   不应借 bash 工具暗渡（遗留组员会持管道让读取永不 EOF，见 P2-9）。
//! - 输出：stdout+stderr 合并文本（截断 64KB），退出码随结果返回；
//! - **越界写门禁（D-17-1 止损）**：执行前静态扫描命令文本，拒绝四类直白越界写
//!   （重定向 / tee / cd / cp·mv·install，实现见同目录 [`bash_guard`] 模块）。
//!   **止损非根治**：bash 图灵完备，静态扫描挡不住 `eval`/变量间接/`$( )` 变体/
//!   `dd of=`/编码绕过等，根治＝契约⑨ sandboxPolicy（P1-11a）；误杀取向
//!   fail-closed（误杀可恢复，漏杀是安全事件）；
//! - **统一目录视图（P0 缺陷一）**：本工具是共享 CWD 服务的唯一推动源——每次
//!   执行后捕获子进程最终 pwd 写回服务（文件类工具据此解析相对路径，见
//!   builtin/cwd.rs）。捕获＝命令尾部追加三行宿主指令（pwd > 临时文件），不判
//!   断「命令是否含 cd」：cd 可藏在 if/&&/函数里，静态判断不可靠，无条件捕获
//!   的成本（一次临时文件写读）远低于漏判带来的路径迷航代价；
//! - 不做完整路径沙箱——由宿主的运行环境约束，修复模式等敏感场景禁用本工具。

use crate::builtin::capture::CaptureFile;
use crate::builtin::cwd::CwdService;
use crate::contract::{ToolDefinition, ToolExec, ToolOutcome};
use std::path::PathBuf;
use std::sync::Arc;
use yunyou_util::BoxFut;

// 对外保持 `bash::detect_escaping_write` 路径可用（门禁实现已迁至 bash_guard）。
pub use super::bash_guard::detect_escaping_write;
use super::bash_guard::reject_with_hint;

/// 单次输出截断上限。
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// 全组收割（unix）：`process_group(0)` 后子进程自为组长（PGID=子 PID），
/// `killpg` 对整组发 SIGKILL——组内孙进程（前台深链/后台遗留）一并死透，
/// 不复用孤儿；组空（全员已退）= ESRCH 静默。幂等：重复调用无害。
#[cfg(unix)]
fn kill_process_group(pgid: u32) {
    // SIGKILL 直达：TERM 需进程配合退出，收割场景（取消/超时/已放弃）
    // 不假设组内进程体面配合——先 KILL 后 reap，无常宽限（任务书信号序裁决）。
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGKILL);
    }
}

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

/// bash 命令执行工具。
pub struct BashTool {
    /// 工具域共享 CWD 服务：本工具执行后捕获 pwd 写回（唯一推动源）。
    pub cwd: Arc<CwdService>,
    /// 沙箱强制器（可选消费，task-19）：Some=helper 链沙箱执行；None=降级
    /// 直跑+结果 meta 标注 `sandboxed:false`（设计 §2 裁决 1）。
    pub enforcer: Option<Arc<dyn crate::sandbox::SandboxEnforcer>>,
}

impl ToolDefinition for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "在工作目录执行一条 bash 命令并返回合并输出与退出码；命令内的 cd 会更新后续文件工具的相对路径基准。长命令会阻塞本轮；避免交互式命令。命令结束或被取消时其全部后代进程随进程组收割，不支持借本工具启动长驻服务。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "要执行的 bash 命令"},
                "cwd": {"type": "string", "description": "可选：本次执行的工作目录（相对工作区根，目录必须已存在）"}
            },
            "required": ["cmd"]
        })
    }

    fn execute(&self, args: serde_json::Value, exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd_service = self.cwd.clone();
        let enforcer = self.enforcer.clone();
        Box::pin(async move {
            let Some(cmd) = args.get("cmd").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 cmd 参数");
            };
            // ① 命令文本静态门禁（D-17-1 止损，非根治；根治=契约⑨ sandboxPolicy）
            if let Some(reason) = detect_escaping_write(cmd) {
                let (message, hint) = reject_with_hint(&reason);
                return ToolOutcome::err_h("tool/path-outside-root", message, hint);
            }
            let cmd = cmd.to_string();
            // ② 起始目录（task-50 rev3 全族单义）：显式 cwd 参数基于工作区根
            // 解析（resolve_workspace：canonicalize+starts_with 防穿越运行时闸），
            // 目录必须已存在（spawn 硬需求，缺失人话指路）；未给则用共享 cwd
            // ——bash 会话自身的 cd 链保留（bash 域状态），fs 工具恒锚根不受
            // 影响。shell 起始 cwd=WorkspaceRoot（关系已写进各工具 description）。
            let start_dir = match args.get("cwd").and_then(|v| v.as_str()) {
                Some(cwd) => match cwd_service.resolve_workspace(cwd) {
                    Ok(dir) if dir.exists() => dir,
                    Ok(dir) => {
                        return ToolOutcome::err_h(
                            "tool/not-found",
                            format!(
                                "目录不存在：{}（相对路径 {cwd}；先创建目录或用 bash mkdir 后重试）",
                                dir.display()
                            ),
                            "先创建目录（bash mkdir -p <目录>）后重试",
                        );
                    }
                    Err(_) => {
                        let (message, hint) = reject_with_hint(&format!(
                            "cwd \"{cwd}\" 越出根目录（绝对路径与 .. 均不接受）"
                        ));
                        return ToolOutcome::err_h("tool/path-outside-root", message, hint);
                    }
                },
                None => cwd_service.cwd(),
            };
            // 沙箱接线（task-19 阶段 4）：enforcer 在位→L2 策略快照（cwd 服务的
            // workspace_root 为可写根首项+/dev/null）→enforcer.prepare 预编译
            // 参数集；None=降级（sandboxed:false meta 显式告警）。
            // C-1 修复：enforcer 在位而 prepare 失败=沙箱预编译失败，fail-closed
            // 拒执行+错误归因透传——旧 `.ok()` 把失败静默压成 None →子进程裸跑，
            // 止损层宣称失实（越界写无拦截，仅 meta 标注不拒）。
            let prepared = match enforcer.as_ref() {
                Some(enf) => {
                    let policy = crate::sandbox::WorkspacePolicy::v1(vec![
                        cwd_service.workspace_root().to_path_buf(),
                        PathBuf::from("/dev/null"),
                    ]);
                    match enf.prepare(&policy) {
                        Ok(p) => Some(p),
                        Err(err) => {
                            return ToolOutcome::err_h(
                                "tool/sandbox-prepare",
                                format!("沙箱准备失败，命令被拒（fail-closed，不降级裸跑）：{err}"),
                                "检查 [sandbox] 配置与内核 Landlock/seccomp 支持后重试；确需绕过请显式 [sandbox] enabled=false（重启生效，全量降级有 meta 告警）",
                            );
                        }
                    }
                }
                None => None,
            };
            run_bash(cmd, start_dir, exec, cwd_service, prepared).await
        })
    }
}

/// 子进程执行主体（独立函数便于取消语义集中）。
async fn run_bash(
    cmd: String,
    start_dir: PathBuf,
    exec: ToolExec,
    cwd: Arc<CwdService>,
    prepared: Option<crate::sandbox::PreparedSandbox>,
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
        None => cmd,
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
    let _group_reaper = GroupReaper(pgid as libc::pid_t);
    // 输出读取交给独立任务（句柄已从 child take，不阻塞取消路径）
    let mut stdout = child.stdout.take().expect("已设 piped");
    let mut stderr = child.stderr.take().expect("已设 piped");
    let out_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut stdout, &mut buf).await;
        buf
    });
    let err_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut stderr, &mut buf).await;
        buf
    });
    let status = tokio::select! {
        result = child.wait() => result,
        _ = exec.cancel.cancelled() => {
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
    // 孙进程可能仍持 stdout/stderr 写端——不杀则 read_to_end 永不 EOF、
    // 本函数永挂。组长已死，killpg 只命中遗留组员；组空=ESRCH。落款
    // （退出码/输出/cwd 捕获）在收割后照常取，正常完成路径零影响。
    #[cfg(unix)]
    kill_process_group(pgid);
    let out = out_task.await.unwrap_or_default();
    let err_out = err_task.await.unwrap_or_default();
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


/// 截断输出到上限（字符边界安全）。
fn truncate(text: &mut String, max_bytes: usize) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 截断的字符边界安全（多字节不裂开）。
    #[test]
    fn 输出截断安全() {
        let mut text = "中".repeat(40_000);
        truncate(&mut text, 1024);
        assert!(text.len() <= 1024 + "[输出已截断]".len() + 3);
        assert!(text.ends_with("[输出已截断]"));
    }

    /// 捕获临时文件 0600 锁权（云镜 P2-A2；对齐 store.rs 凭据测试写法）：
    /// 宿主预创建即 0600，bash `>` 截断写不提权——子进程 umask 不再决定权限。
    /// （CaptureFile 为 pub(crate)，测试须留在 crate 内；执行类测试外迁
    /// tests/bash_exec.rs 与 tests/bash_orphan.rs，公开 API 即可达。）
    #[test]
    fn 捕获文件0600锁权() {
        let capture = CaptureFile::new();
        let Some(path) = capture.as_inner() else {
            panic!("预创建应成功（Drop 兜底清理）");
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).expect("预创建文件存在").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "捕获文件必须 0600（对齐 write_secret_file 惯用法）");
        }
        #[cfg(not(unix))]
        assert!(path.exists());
    }
}
