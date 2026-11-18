# bash

> 真源：`crates/tools/yunyou-tools/src/builtin/bash.rs`（description/parameters 以代码为准）。

在工作目录执行一条 bash 命令并返回合并输出与退出码；命令内的 cd 会更新后续文件工具的相对路径基准。长命令会阻塞本轮；避免交互式命令。命令结束或被取消时其全部后代进程随进程组收割，不支持借本工具启动长驻服务。

- **沙箱**（Linux）：Landlock 文件白名单+seccomp 禁网（`[sandbox]` 段）；越界写命令文本在执行前静态门禁拦截（人话+指路）。
- **后台化**：前台超时（默认 60s）自动转后台返回 `bg-xxxx`，或显式 `run_in_background`；进度用 task_output、终止用 task_stop（见 background-tasks.md）。
- **参数**：`cmd`（必填）；`cwd` 可选（相对工作区根，目录须已存在）。
- **相对路径基准**：文件类工具恒锚工作区根，不随 bash cd 漂移；bash 子进程起始=工作区根。
