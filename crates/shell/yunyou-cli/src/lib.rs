//! # yunyou-cli —— yunyou 外壳与入口（任务 3B）
//!
//! 按《组件内核架构方案》§2.5「最小闭环启动序列」装配：
//!
//! ```text
//! main → tokio 多线程运行时 → root Context
//!   ├─ yunyou-config 加载 yunyou.toml（纯库；段值透传给各组件）
//!   ├─ 依序 ctx.plugin：llm(OpenAI) → sessions(JSONL) → systemPrompt → tools(内置六工具) → agentLoop
//!   │   （agentLoop 声明 inject 四服务；epoch 就绪自动 ACTIVE——顺序仅影响启动日志）
//!   ├─ 装配完成 → 就绪标记 <yunyou-home>/run/ready-<pid>（为团队/网关阶段预留）
//!   ├─ REPL / 单任务模式（--task）：对话经 dyn AgentLoop 服务执行
//!   └─ Ctrl-C → 中断本轮（cancel token）→ 摘就绪标记 → root.dispose()：整树逆序卸载
//! ```
//!
//! ## 模块职责
//! - [`cli`]：命令行参数与 yunyou-home 解析；
//! - [`humanize`]：工具结果人话摘要（完成卡动词句；退出码单点声明制）；
//! - [`run`]：总控流程与错误类型；
//! - [`assemble`]：§2.4 表驱动装配序列（内置注册表循环 + 装载前校验 + 状态表）；
//! - [`bundled`]：内置插件注册表（编译期常量清单；新增内置插件只改这里）；
//! - [`credentials`]：凭据接线（启动迁移＋llm 段引用解析，S5 收口合入）；
//! - [`status`]：插件状态表渲染与 Pending 诊断（方案 §2.4 第 5 步观测面）；
//! - [`repl`]：交互循环与单任务模式（对话经 `dyn AgentLoop` 服务执行）；
//! - [`ready`]：就绪标记文件（装配完成创建、退出删除）。
//!
//! ## 契约要点
//! - **fail-loud**：装配失败 → 已装载组件逆序卸干净 → 非 0 退出（对齐 boot）；
//! - **优雅关闭顺序**：先摘就绪标记（观测者立刻感知不再接活），再 `root.dispose()`；
//! - **Ctrl-C**：对话期先 cancel 本轮（run_turn 在流/工具边界干净收尾），本轮落定再退；
//! - 所有插件/服务实现 `Send + Sync`（tokio 多线程，方案 §2.3）。

pub mod adapter;
pub mod assemble;
pub mod market_manifest;
pub mod proxy_plugin;
pub mod bundled;
pub mod credentials;
pub mod cli;
pub mod compact_cmd;
mod command_exec;
pub mod humanize;
pub mod history;
pub mod input_source;
pub mod lineedit;
pub mod observe;
pub mod panel;
pub mod panel_text;
pub mod persona_md;
/// `yunyou plugin new/install` 插件管理子命令（task-10）。
pub mod plugin_cmd;
pub mod plugins_cmd;
pub mod ready;
pub mod render;
pub mod repl;
pub mod run;
/// /sessions 会话列表与切换 + 会话回放（task-36④；命令缝 D1）。
pub mod sessions_cmd;
pub mod shell_commands;
pub mod skill_read;
pub mod skills;
pub mod slots;
pub mod status;

pub use run::{run, AppError};
