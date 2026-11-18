# 工具说明（每工具一份，索引）

> **同源单源纪律（比 claude 好③，池 task-25）**：工具的权威描述=代码内
> `ToolDefinition::description()`（模型面真源）；本目录文档是**人读投影**——
> 写作以代码描述为准，工具描述变更同批更新对应文档（收口 checklist 纪律，
> 与 plugins-seams skill 同罚则）。工具行为契约的机器可读版见 `docs/接口/`。

| 工具 | 一句话 | 文档 | 真源（代码） |
| --- | --- | --- | --- |
| bash | 工作区内执行命令（沙箱+后台化） | [bash.md](bash.md) | `crates/tools/yunyou-tools/src/builtin/bash.rs` |
| read | 读工作区内文件 | [read.md](read.md) | `…/builtin/fs_tools.rs` |
| write | 写工作区内文件（防覆盖校验） | [write.md](write.md) | `…/builtin/fs_tools.rs` |
| edit | 精确替换编辑文件 | [edit.md](edit.md) | `…/builtin/fs_tools.rs` |
| glob | 按模式找文件 | [glob.md](glob.md) | `…/builtin/search.rs` |
| grep | 按正则搜内容 | [grep.md](grep.md) | `…/builtin/search.rs` |
| apply_patch | 多文件补丁（原子试算） | [apply_patch.md](apply_patch.md) | `…/builtin/patch_tool.rs` |
| task_output / task_stop / task_list | 后台任务观测/停止/列表 | [background-tasks.md](background-tasks.md) | `…/builtin/tasks_tool.rs` |
| skill_read | 按名读技能全文（渐进披露） | [skill_read.md](skill_read.md) | `crates/shell/yunyou-cli/src/skill_read.rs` |
| memory_save / memory_search / memory_list / memory_forget | 跨会话记忆四操作 | [memory.md](memory.md) | `crates/shell/yunyou-cli/src/memory.rs` |
| web_fetch / web_search | 网页抓取/搜索（若启用） | — | 上游工具域 |
