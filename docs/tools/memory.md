# 记忆四操作（memory_save / memory_search / memory_list / memory_forget）

> 真源：`crates/shell/yunyou-cli/src/memory.rs`；设计：`docs/设计/memory.v1-记忆缝与工具集设计-20261118.md`。

跨会话记忆统一存取（memory.v1 缝）：

- **memory_save**：写话题文件+维护活索引（同话题覆盖正文、索引去重）；索引硬顶 200 行/25KB，超限**拒绝写入**并人话报「索引已满请整理」（禁静默截断）。
- **memory_search**：索引+话题正文关键词全文匹配（v1；语义检索留接口）。
- **memory_list**：索引条目全列。
- **memory_forget**：**唯一删除口**——删话题文件+摘索引行；未知名报错附清单。
- **目录**：`~/.yunyou/projects/<净化 cwd>/memory/`（MEMORY.md 索引+话题文件+logs/ 每日日志）；**注入**：仅 MEMORY.md 常驻进系统提示（harness:memory 段，≤400 token，超限截顶+显式尾注）。
- **作用域**：v1 仅 project；global 候审批缝（Ask，落地前人话报错）。
- **与 persona/skills 边界**：persona=我是谁（常驻身份）、skills=怎么干活（指令性知识）、memory=干过什么/用户偏好（事实性知识）。
