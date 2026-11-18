# 后台任务三件（task_output / task_stop / task_list）

> 真源：`crates/tools/yunyou-tools/src/builtin/tasks_tool.rs`。

bash 长命令自动/显式转后台后（`bg-xxxx`）：

- **task_output**：查询输出与状态（running/done/stopped+退出码）；支持阻塞等待新输出——看进度用它，不要 bash sleep 轮询。
- **task_stop**：终止后台任务（整条命令进程组 SIGKILL，含全部子进程）；死前输出仍可 task_output 查看。
- **task_list**：本会话全部后台任务（id/状态/时长/命令摘要）；随会话退场统一收割，不跨会话存活。
