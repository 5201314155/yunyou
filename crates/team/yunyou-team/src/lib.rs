//! # yunyou-team —— 团队插件（任务26 T2 / K-7，bundled 第 9 卡）
//!
//! 业主五点交互规格的插件面落点：①AI 自拉人（lead 经 function-calling 调
//! spawn_teammate 真拉起成员 fiber）②成员互联（peer 带名互发，路由经内核
//! MessageHub）③完工自动汇报（成员回合终结=Report 自动投递 lead，机制非习惯）
//! ④共享任务板（team_task_* 四操作，owner 落真实成员）⑤名册观测面（谁在跑/
//! 谁空闲/谁的活）。
//!
//! ## 身份与接线（依赖内核 message.v1 缝，K-6 已入库 378903a）
//!
//! - **lead 身份源=本插件自己的 fiber**：setup 内 `register_member("lead")`——
//!   自发成员语义（hub 对无条目名的 register 直接 Online）；lead 侧全部 hub
//!   操作（发消息/预注册信箱）经 [`TeamService`] 代理执行，工具实现拿不到
//!   Context 也没有身份——「hub 缝上连实现也无法伪造」在插件面的延续；
//! - **成员=独立插件 fiber**（团队 fiber 的子 fiber：禁用团队即全收，卸载顺序
//!   由内核「先消费者后提供者」保证）：setup 内以自己的名字 register——
//!   Pending 预注册信箱被认领（补投就绪）或直接 Online；
//! - **spawn 窗口**：spawn_teammate 先 open_mailbox 再建 fiber——拉起窗口内
//!   lead 的首条消息缓冲不丢，成员上线后首次 drain 取净（hub 补投语义）；
//! - **消费**：成员回合循环 drain_inbox 取净进轮输入（偏差申报 a 见 plugin.rs）；
//! - **离场**：成员 fiber dispose → hub 条目降回 Pending（信箱与未取净消息
//!   保留，同名重拉续投）；
//! - **观测**：message.v1/watch 六位事件（T3 `/team watch` 数据面免费就绪）。
//!
//! ## 词汇纪律
//!
//! member/lead/名册/任务板/消息池与任务26 台账原文单源；蓝图二期词（worker/
//! 租约/SQLite/launcher）不进 v1 面（成员内循环称「成员回合循环」）。

use yunyou::hub::DeliveredMessage;

mod auto;
mod board;
mod member;
mod member_tools;
mod plugin;
mod roster;
mod service;
mod task_types;
mod service_notify;
mod task_tools;
mod tools;

/// 回合输入合成（task-53 消费缝的注入形态，纯函数测试面）：无消息=原输入
/// 原样返回（零包装噪音）；有消息=【来自 <成员名>】行批量前缀+空行+原输入
/// ——与成员侧「【来自 lead】」入站形态同构（双向对称）。
pub fn compose_turn_input(messages: &[DeliveredMessage], user_input: &str) -> String {
    if messages.is_empty() {
        return user_input.to_string();
    }
    let mut injected = String::new();
    for message in messages {
        injected.push_str(&format!("【来自 {}】{}\n", message.from, message.body));
    }
    injected.push('\n');
    injected.push_str(user_input);
    injected
}

/// 测试支持（doc-hidden：非公开契约面，集成测试构造成员工具面用）。
#[doc(hidden)]
mod observe;
pub mod test_support {
    pub use crate::member_tools::member_registry;
}

pub use auto::{AutoClaimConfig, ClaimCandidate};
pub use observe::stalled_members;
pub use board::{BoardOpError, TeamTask, TeamTaskState, TeamTaskView};
pub use plugin::TeamPlugin;
pub use roster::RosterEntry;
pub use task_types::{ContextMode, TeamConfig, TeamService, TeammateSpec};
