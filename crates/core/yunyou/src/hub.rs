//! 契约 message.v1：进程内消息枢纽（发布订阅 + 寻址 + 留痕 + 信箱补投）。
//!
//! 设计依据：`docs/架构/MessageHub契约设计-任务26T1.md`（融合终版 v1.1，lead 批复）；
//! 词汇单源=任务26 台账原文（teammate/lead/member/名册/消息池/发布订阅+寻址+留痕），
//! worker/租约/SQLite 等蓝图二期词不进本契约面。数据形态与 trait 签名为冻结面
//! （破坏性变更=v2+兼容期，不静默改）。
//!
//! ## 身份派发机制（lead 2026-11-15 批：方案甲；云枢 retro 项①留痕）
//!
//! 冻结签名七方法全无 from/ctx 参数，而内核「当前 fiber」唯一来源是
//! Context（ContextInner.fiber 槽，全库无 task/thread_local 先例）——故运载用
//! 本模块**私有** task_local（不导出，调用方无法设置=伪造面关闭）：内核在
//! [`Context`] 身份入口内用既有 `current_fiber_or_inactive` 解析调用 fiber 后
//! `sync_scope` 进入冻结 trait 方法。无运载调用=注册类报 EffectInactive、
//! send/drain 报 HubUnknownMember——恰是「无身份者发不出/越权读被拦」的防伪造
//! 语义诚实呈现（lead 裁决确认要点②）。
//!
//! 注：T2 成员 fiber spawn 时的 task scope 搭建点属 task-16（K-7）接口位，本卡
//! 不越界实现——成员插件在自身 setup/fiber 上下文内直接用 Context 身份入口即可，
//! 无需自建 scope（此注释即锚点）。

mod delivery;
mod history;
pub mod registry;
mod service;
mod types;

pub use service::InProcessHub;
pub use types::{
    Address, DeliveredMessage, DeliveryStatus, HistoryEntry, HistoryQuery, MemberInfo,
    MessageHandler, MessageKind, OutgoingMessage, SentReceipt, WATCH_KEY,
};

use crate::context::Context;
use crate::error::Result;
use crate::events::current_fiber_or_inactive;
use crate::fiber::{EffectHandle, FiberShared};
use crate::service::ServiceId;
use std::sync::Arc;

tokio::task_local! {
    /// 调用方 fiber 的私有运载（方案甲）：只在本模块身份入口内设置，hub 外
    /// 不可见不可设——hub 由此成为唯一身份源（防伪造定案的机制化）。
    /// 值=调用 fiber 共享句柄；sync_scope 的闭包为同步短临界区（锁内不 await）。
    static CALLING_FIBER: Arc<FiberShared>;
}

/// 契约 message.v1 服务 trait（§1.2 原样冻结；T2 团队插件编译依赖面）。
///
/// 全同步短临界区：锁内落账、锁外观测；object-safe（无泛型类型参数、无 Self
/// 返回值、全同步返回——推模式降级为尽力观测后 await 语义消失，保异步=预造）。
/// **身份方法不经 Context 入口（无运载）调用即报错**：见模块注释「身份派发机制」。
pub trait MessageHub: Send + Sync + 'static {
    /// 预注册信箱（Pending 态）：spawn_teammate 拉起窗口的消息落点。
    /// EffectHandle 挂调用 fiber——spawn 失败可提前 dispose 回收名字；
    /// 重名（任何态）/空白名 fail-loud。
    fn open_mailbox(&self, name: &str) -> Result<EffectHandle>;

    /// 成员上线：绑定当前 fiber。Pending→Online（补投就绪）；无条目→直接
    /// Online（自发成员，如 lead）；已 Online→HubDuplicateMember。
    /// EffectHandle 语义：fiber 死亡→条目降回 Pending（信箱与未取净消息保留，
    /// 同名重注册即续投）——非删除。
    fn register(&self, name: &str) -> Result<EffectHandle>;

    /// 名册消息侧投影（online 标志+归属；Pending 无归属=spawn 窗口可见性）。
    fn members(&self) -> Vec<MemberInfo>;

    /// 观测面订阅（推，尽力而为）：当前 fiber 绑定成员的入站镜像；
    /// handler 失败记日志不上抛（消费保证在 drain）。
    fn subscribe(&self, handler: MessageHandler) -> Result<EffectHandle>;

    /// 寻址投递：身份由 hub 按调用 fiber 派发（防伪造定案）；Online=入信箱
    /// +观测镜像，Pending=信箱缓冲，未在册=HubUnknownMember。
    /// 同步短临界区（锁内不 await）；广播 All 不含 from 自己。
    fn send(&self, to: Address, message: OutgoingMessage) -> Result<SentReceipt>;

    /// 消费通道（拉）：取净当前 fiber 绑定成员的信箱（log_id 升序）；
    /// 非成员 fiber=HubUnknownMember（防越权读他人信箱）。
    fn drain(&self) -> Result<Vec<DeliveredMessage>>;

    /// 留痕查询：环形 1000 丢最旧；完整历史=各成员会话 JSONL（会话域）。
    fn history(&self, query: HistoryQuery) -> Result<Vec<HistoryEntry>>;
}

/// 私有运载读取口（service.rs 判「无身份」分型用；错误分型在 require_* 层做）。
pub(super) fn calling_fiber() -> Option<Arc<FiberShared>> {
    CALLING_FIBER.try_with(Arc::clone).ok()
}

/// 在身份运载内执行同步调用（Context 身份入口专用；sync_scope 的唯一设置点，
/// 私有=调用方无法绕过内核伪造身份）。
fn with_calling_fiber<T>(fiber: Arc<FiberShared>, op: impl FnOnce() -> T) -> T {
    CALLING_FIBER.sync_scope(fiber, op)
}

impl Context {
    /// 身份入口通用形：解析当前 fiber → 私有运载内调 hub。
    /// 无归属 fiber 的上下文在此处即 EffectInactive（注册类语义，测试矩阵④）；
    /// send/drain 的无绑定分型由 trait 实现层补足（有 fiber 无绑定=HubUnknownMember）。
    fn in_hub_scope<T>(&self, op: impl FnOnce(Arc<dyn MessageHub>) -> Result<T>) -> Result<T> {
        let fiber = current_fiber_or_inactive(self)?;
        // Arc<InProcessHub> → Arc<dyn MessageHub> 的 unsized 强转需两步（Arc::clone
        // 的泛型推断不做自动强转）。
        let hub: Arc<dyn MessageHub> = self.inner.root.hub.clone();
        with_calling_fiber(fiber, || op(hub))
    }

    /// 预注册信箱（契约 open_mailbox 的身份入口）：spawn 拉起窗口的消息落点。
    pub fn open_mailbox(&self, name: &str) -> Result<EffectHandle> {
        self.in_hub_scope(|hub| hub.open_mailbox(name))
    }

    /// 成员上线（契约 register 的身份入口）：绑定当前 fiber，补投就绪。
    pub fn register_member(&self, name: &str) -> Result<EffectHandle> {
        self.in_hub_scope(|hub| hub.register(name))
    }

    /// 观测面订阅（契约 subscribe 的身份入口）：当前 fiber 绑定成员的入站镜像。
    pub fn subscribe_messages(&self, handler: MessageHandler) -> Result<EffectHandle> {
        self.in_hub_scope(|hub| hub.subscribe(handler))
    }

    /// 寻址投递（契约 send 的身份入口）：发送者=当前 fiber 绑定成员（防伪造）。
    pub fn hub_send(&self, to: Address, message: OutgoingMessage) -> Result<SentReceipt> {
        self.in_hub_scope(|hub| hub.send(to, message))
    }

    /// 消费拉取（契约 drain 的身份入口）：取净当前 fiber 绑定成员的信箱。
    pub fn drain_inbox(&self) -> Result<Vec<DeliveredMessage>> {
        self.in_hub_scope(|hub| hub.drain())
    }
}

/// 内核 root 装配（Context::new 调用）：以 `ServiceId::of::<dyn MessageHub>()`
/// 在 root fiber provide——「缝由内核供：team 插件禁用不得杀缝」（三补③原文，
/// 融合终版 §3 表 f 定案）；root fiber 常在=EffectInactive 免疫。消费方照
/// compaction 消费 llm 同款范式 inject+`ctx.get` 取用。
pub(crate) fn mount_root(ctx: &Context) -> Result<EffectHandle> {
    let hub: Arc<dyn MessageHub> = ctx.inner.root.hub.clone();
    ctx.provide(ServiceId::of::<dyn MessageHub>(), hub)
}
