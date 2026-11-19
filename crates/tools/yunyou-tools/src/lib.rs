//! 工具域（对标 TS 版 yunyou-tools 的最小裁剪，方案 §2.5 / §1.9）。
//!
//! - 契约：[`ToolDefinition`]（object-safe，异步返回 [`BoxFut`]）——对标
//!   `contracts.ts` 的 execute + output 声明；
//! - 注册表：[`ToolsRegistry`]——注册即 effect，fiber 卸载自动撤销（方案 §2.1.4）；
//! - 执行器：[`executor::execute_tool`]——超时 / 取消传播（方案 §2.3 第 3 条）；
//! - 内置工具：bash / read / write / edit / glob / grep（read/write/edit/glob/grep
//!   全部受 root_dir 路径约束；bash 起独立进程组并观察取消）。
//!
//! # 契约（评审线一强制）
//! - 工具实现必须 `Send + Sync`；`execute` 返回的 future 必须 `'static`
//!   （工具自持数据，不借用调用方）；
//! - 取消语义：长任务必须观察 [`ToolExec::cancel`] 并在触发后尽快静止返回
//!   （对齐 TS「observe exec.signal and settle after quiescence」，contracts.ts:41）；
//! - 失败语义：工具业务失败用 [`ToolOutcome::Err`]（进入会话的 tool_result
//!   is_error=true），执行器基础设施失败（超时/取消）也归一为 `Err`。

pub mod builtin;
pub mod contract;
pub mod sandbox;
// 沙箱实现面（结构归一收敛批 2026-11-19，业主裁「工具域归一」）：原
// yunyou-sandbox crate 以 module 并入（Landlock/seccomp helper 链+插件装配），
// 契约 trait 仍在 sandbox.rs（enforcer 可选消费面不变）
pub mod sandbox_impl;
pub mod sandbox_helper;
pub mod executor;
pub mod plugin;
pub mod registry;
// task-20 后台任务批（他席在途，yun-ya 机械补声明）：builtin/bash.rs 已消费
// crate::tasks::{kill_process_group,OutputSink,TaskRegistry,TaskState}，模块
// 文件在位但漏挂 lib——补声明接通编译，语义归承办席
pub mod tasks;

pub use contract::{ToolDefinition, ToolError, ToolExec, ToolOutcome};
pub use plugin::{ToolsConfig, ToolsPlugin};
pub use registry::{RegistryService, ToolsRegistry};
