//! 权限审批策略插件（task-16，池卡；设计稿=docs/设计/task-16-权限审批插件设计-
//! 判定链与规则引擎-20261118.md，lead 三待裁全批+S1 入库 ccce206f）。
//!
//! # 定位（主板论）
//! 审批缝=内核机制（yunyou-agent-loop::gate，S1 已入库）；**规则/模式/边界/
//! 拦截全部是本插件策略**——内核零规则知识。本插件是 GateRegistry 的第一个
//! 真实策略卡。
//!
//! # 判定链本插件负责的层（S2 范围）
//! - L0 高危免疫清单（代码内置不可配置，bypass 也拦）；
//! - L1/L2/L6 规则三值（settings [permissions] deny/ask/allow，`Tool(content)`
//!   语法+括号转义，四族匹配器）；
//! - 无命中=Pass（缺省行为归 S3 模式层；L4 沙箱穿越归 S4；L5 工作区边界归 S4；
//!   L8 Ask 兜底的 suggestions 持久化归 S5）。

mod authorization;
mod boundary;
mod denylist;
mod gate;
mod mode_gate;
mod matchers;
mod mode;
mod persistence;
mod plugin;
mod rules;

pub use gate::PermissionGate;
pub use authorization::{authorize_plugin, granted, AuthorizationsFile, GrantRequest, PluginGrant};
pub use mode_gate::ModeGate;
pub use persistence::SettingsPersistence;
pub use plugin::PermissionPlugin;
pub use mode::{Mode, ToolClass, Unmatched};
pub use rules::{
    escape_rule_content, Behavior, PermissionsSection, Rule, RuleSet,
};
