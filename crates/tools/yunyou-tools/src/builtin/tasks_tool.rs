//! 后台任务管理三工具（池 task-20）：task_output / task_stop / task_list。
//! 注册表 Arc 由工厂串线（与 bash 后台移交共用同一实例）；权限链与前台同
//! ——本三工具不执行命令，只管理已通过 bash 门禁+沙箱 prepare 的任务。

use crate::contract::{ToolDefinition, ToolExec, ToolOutcome};
use crate::tasks::{tail_read, TaskRegistry, TaskState, BLOCK_SECS_MAX, TAIL_DEFAULT_BYTES, TAIL_MAX_BYTES};
use std::sync::Arc;
use std::time::Duration;
use yunyou_util::BoxFut;

/// 按名查任务（三工具共用错误形态）。
fn lookup(registry: &TaskRegistry, task_id: &str) -> Result<crate::tasks::TaskEntry, ToolOutcome> {
    registry.get(task_id).ok_or_else(|| {
        ToolOutcome::err_h(
            "tool/not-found",
            format!("后台任务 \"{task_id}\" 不存在（本会话注册表内无此任务）"),
            "用 task_list 查看当前全部后台任务；后台任务不跨会话存活",
        )
    })
}

// ---------------------------------------------------------------------------
// task_output
// ---------------------------------------------------------------------------

/// 后台任务输出查询：尾部窗口+可选阻塞等待。
pub struct TaskOutputTool {
    pub tasks: Arc<TaskRegistry>,
}

impl ToolDefinition for TaskOutputTool {
    fn name(&self) -> &str {
        "task_output"
    }

    fn description(&self) -> &str {
        "查询后台任务的输出与状态（running/done/stopped+退出码）。后台 bash 命令用此工具看进度，不要用 bash sleep 轮询。支持阻塞等待新输出。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string", "description": "后台任务 id（转后台返回的 bg-xxxx）"},
                "tail_bytes": {"type": "integer", "description": "可选：取输出尾部字节数（默认 4096，上限 65536，超窗头部截断明示）"},
                "block_secs": {"type": "integer", "description": "可选：阻塞等待新输出或进程退出的秒数（0=即时快照；上限 30，传更大值按 30 处理——防傻等，需要更久就分次调用）"}
            },
            "required": ["task_id"]
        })
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let registry = self.tasks.clone();
        Box::pin(async move {
            let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 task_id 参数");
            };
            let tail_bytes = args
                .get("tail_bytes")
                .and_then(|v| v.as_u64())
                .map(|v| v.min(TAIL_MAX_BYTES as u64) as usize)
                .unwrap_or(TAIL_DEFAULT_BYTES);
            let block_secs = args
                .get("block_secs")
                .and_then(|v| v.as_u64())
                .map(|v| v.min(BLOCK_SECS_MAX))
                .unwrap_or(0);
            {
                let entry = match lookup(&registry, task_id) {
                    Ok(entry) => entry,
                    Err(outcome) => return outcome,
                };
                // 非运行态或零等待：直接快照
                if entry.state != TaskState::Running || block_secs == 0 {
                    return snapshot_outcome(&registry, task_id, tail_bytes);
                }
            }
            // 阻塞等待：轮询注册表直到终态或超时（250ms 步进；上限已 clamp）
            let deadline = std::time::Instant::now() + Duration::from_secs(block_secs);
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let entry = match lookup(&registry, task_id) {
                    Ok(entry) => entry,
                    Err(outcome) => return outcome,
                };
                if entry.state != TaskState::Running || std::time::Instant::now() >= deadline {
                    return snapshot_outcome(&registry, task_id, tail_bytes);
                }
            }
        })
    }
}

/// 快照响应：state/exit_code/尾部输出/日志路径（人话自解释闭环）。
fn snapshot_outcome(registry: &TaskRegistry, task_id: &str, tail_bytes: usize) -> ToolOutcome {
    let entry = registry.get(task_id).expect("lookup 已确认在表");
    let (truncated, output) = match tail_read(&entry.log_path, tail_bytes) {
        Ok(result) => result,
        Err(err) => {
            return ToolOutcome::err(
                "tool/io",
                format!("读取后台任务日志失败：{err}（路径 {}）", entry.log_path.display()),
            );
        }
    };
    ToolOutcome::ok(serde_json::json!({
        "task_id": task_id,
        "state": entry.state.as_str(),
        "exit_code": entry.exit_code,
        "output": if truncated { format!("…前段已截断\n{output}") } else { output },
        "log_path": entry.log_path.display().to_string(),
    }))
}

// ---------------------------------------------------------------------------
// task_stop
// ---------------------------------------------------------------------------

/// 终止后台任务：killpg 全组收割（复用 bash 域基建，孙进程一并死透）。
pub struct TaskStopTool {
    pub tasks: Arc<TaskRegistry>,
}

impl ToolDefinition for TaskStopTool {
    fn name(&self) -> &str {
        "task_stop"
    }

    fn description(&self) -> &str {
        "终止一个后台任务（对整条命令进程组发 SIGKILL，含其全部子进程）。任务终止后仍可用 task_output 查看死前输出。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string", "description": "要终止的后台任务 id"}
            },
            "required": ["task_id"]
        })
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let registry = self.tasks.clone();
        Box::pin(async move {
            let Some(task_id) = args.get("task_id").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 task_id 参数");
            };
            let entry = match lookup(&registry, task_id) {
                Ok(entry) => entry,
                Err(outcome) => return outcome,
            };
            if entry.state != TaskState::Running {
                return ToolOutcome::ok(serde_json::json!({
                    "task_id": task_id,
                    "state": entry.state.as_str(),
                    "message": format!("任务已处于 {} 态，无需终止", entry.state.as_str()),
                }));
            }
            // 全组收割（组空=ESRCH 幂等）；非 unix 无进程组语义——如实告知
            // kill_on_drop 兜底只覆盖直子。
            #[cfg(unix)]
            if let Some(pgid) = entry.pgid {
                crate::tasks::kill_process_group(pgid);
            }
            registry.transition(task_id, TaskState::Stopped, None);
            ToolOutcome::ok(serde_json::json!({
                "task_id": task_id,
                "state": "stopped",
                "message": "后台任务已终止（进程组 SIGKILL）；死前输出可用 task_output 查看",
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// task_list
// ---------------------------------------------------------------------------

/// 列出本会话全部后台任务。
pub struct TaskListTool {
    pub tasks: Arc<TaskRegistry>,
}

impl ToolDefinition for TaskListTool {
    fn name(&self) -> &str {
        "task_list"
    }

    fn description(&self) -> &str {
        "列出本会话全部后台任务（id/状态/已运行时长/命令摘要）。后台任务随会话退场统一收割，不跨会话存活。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}, "required": []})
    }

    fn execute(&self, _args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let registry = self.tasks.clone();
        Box::pin(async move {
            let items: Vec<serde_json::Value> = registry
                .list()
                .into_iter()
                .map(|(id, entry)| {
                    serde_json::json!({
                        "task_id": id,
                        "state": entry.state.as_str(),
                        "exit_code": entry.exit_code,
                        "elapsed_secs": entry.started_at.elapsed().as_secs(),
                        "command": entry.command,
                        "log_path": entry.log_path.display().to_string(),
                    })
                })
                .collect();
            ToolOutcome::ok(serde_json::json!({ "tasks": items }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskRegistry;

    fn registry_with(id: &str, command: &str) -> Arc<TaskRegistry> {
        let reg = Arc::new(TaskRegistry::new());
        reg.register(id, Some(u32::MAX), std::env::temp_dir().join("yt3.log"), command);
        reg
    }

    /// task_output：即时快照（done 态）+未知 id 拒绝。
    #[tokio::test(flavor = "multi_thread")]
    async fn task_output快照与未知id() {
        let reg = registry_with("bg-000001", "echo hi");
        reg.transition("bg-000001", TaskState::Done, Some(0));
        std::fs::write(std::env::temp_dir().join("yt3.log"), "hello out").unwrap();
        let tool = TaskOutputTool { tasks: reg };
        let out = tool.execute(serde_json::json!({"task_id": "bg-000001"}), exec()).await;
        match out {
            ToolOutcome::Ok(v) => {
                assert_eq!(v["state"], "done");
                assert_eq!(v["output"], "hello out");
                assert_eq!(v["exit_code"], 0);
            }
            other => panic!("应快照成功：{other:?}"),
        }
        let out = tool.execute(serde_json::json!({"task_id": "bg-nope"}), exec()).await;
        assert!(matches!(out, ToolOutcome::Err(_)), "未知 id 应拒");
    }

    /// task_stop：Running→stopped 收割标记（pgid=组空 ESRCH 无害）+幂等二次停。
    #[tokio::test(flavor = "multi_thread")]
    async fn task_stop收割与幂等() {
        let reg = registry_with("bg-000002", "sleep 1000");
        let tool = TaskStopTool { tasks: reg };
        let out = tool.execute(serde_json::json!({"task_id": "bg-000002"}), exec()).await;
        match out {
            ToolOutcome::Ok(v) => assert_eq!(v["state"], "stopped"),
            other => panic!("应停止成功：{other:?}"),
        }
        let out = tool.execute(serde_json::json!({"task_id": "bg-000002"}), exec()).await;
        match out {
            ToolOutcome::Ok(v) => assert!(v["message"].as_str().unwrap().contains("无需终止")),
            other => panic!("二次停止应幂等确认：{other:?}"),
        }
    }

    /// task_list：全量条目含时长与摘要。
    #[tokio::test(flavor = "multi_thread")]
    async fn task_list全量() {
        let reg = registry_with("bg-000003", "cargo build --release 很长很长的命令摘要截断测试");
        let tool = TaskListTool { tasks: reg };
        let out = tool.execute(serde_json::json!({}), exec()).await;
        match out {
            ToolOutcome::Ok(v) => {
                let item = &v["tasks"][0];
                assert_eq!(item["task_id"], "bg-000003");
                assert_eq!(item["state"], "running");
                assert!(item["command"].as_str().unwrap().starts_with("cargo build"));
            }
            other => panic!("应列出任务：{other:?}"),
        }
    }

    fn exec() -> ToolExec {
        ToolExec {
            call_id: "t20".into(),
            cancel: tokio_util::sync::CancellationToken::new(),
            workspace: None,
        }
    }
}
