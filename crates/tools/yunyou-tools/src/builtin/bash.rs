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

use crate::builtin::cwd::CwdService;
use crate::contract::{ToolDefinition, ToolExec, ToolOutcome};
use crate::tasks::TaskRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use yunyou_util::BoxFut;

// 对外保持 `bash::detect_escaping_write` 路径可用（门禁实现已迁至 bash_guard）。
pub use super::bash_guard::detect_escaping_write;
use super::bash_guard::reject_with_hint;
pub(crate) use super::bash_exec::BACKGROUND_AFTER_DEFAULT_SECS;
use super::bash_exec::run_bash;

/// bash 命令执行工具。
pub struct BashTool {
    /// 工具域共享 CWD 服务：本工具执行后捕获 pwd 写回（唯一推动源）。
    pub cwd: Arc<CwdService>,
    /// 沙箱强制器（可选消费，task-19）：Some=helper 链沙箱执行；None=降级
    /// 直跑+结果 meta 标注 `sandboxed:false`（设计 §2 裁决 1）。
    pub enforcer: Option<Arc<dyn crate::sandbox::SandboxEnforcer>>,
    /// 后台任务注册表（池 task-20）：Some=超时/显式参数可转后台；None=纯前台
    /// （三臂 select 退化为双臂，行为与 task-20 前完全一致）。转后台与前台
    /// 同权限判定链（guard/沙箱 prepare 全在 spawn 前），不因转后台豁免。
    pub tasks: Option<Arc<TaskRegistry>>,
    /// 前台自动转后台阈值（秒，0=关自动转；显式 run_in_background 不受此限）。
    pub background_after_secs: u64,
}

impl ToolDefinition for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "在工作目录执行一条 bash 命令并返回合并输出与退出码；命令内的 cd 会更新后续文件工具的相对路径基准。长命令（默认超 60 秒，可配）自动转后台并立即返回 task_id，用 task_output 查询进度、task_stop 终止、task_list 列表；也可用 run_in_background 显式后台执行。命令结束或被取消时其前台后代进程随进程组收割；已转后台的任务不随对话取消终止（只由 task_stop 或会话退场收割）。避免交互式命令。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "要执行的 bash 命令"},
                "cwd": {"type": "string", "description": "可选：本次执行的工作目录（相对工作区根，目录必须已存在）"},
                "run_in_background": {"type": "boolean", "description": "可选：true=立即转后台执行，返回 task_id 后用 task_output 轮询输出、task_stop 终止（长编译/长测试推荐，不阻塞对话）"}
            },
            "required": ["cmd"]
        })
    }

    fn execute(&self, args: serde_json::Value, exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd_service = self.cwd.clone();
        let enforcer = self.enforcer.clone();
        let tasks = self.tasks.clone();
        let background_after_secs = self.background_after_secs;
        Box::pin(async move {
            let run_in_background = args
                .get("run_in_background")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
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
            run_bash(cmd, start_dir, exec, cwd_service, prepared, tasks, background_after_secs, run_in_background).await
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::builtin::capture::CaptureFile;
    use super::super::bash_exec::truncate;

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
