//! MessageHub 服务实现：InProcessHub（成员表+信箱+留痕环，单一短临界区锁）。
//!
//! 结构：`Arc<Mutex<CoreInner>>` 全 hub 一份可共享态——send/drain 为同步短临界区
//! （锁内落账、锁内不 await）；观测通知（watch emit / 镜像 spawn）在锁释放后执行。
//! core 用独立 Arc 持有而非内联：open_mailbox/register/subscribe 的 disposer 需要
//! 'static 捕获才能在 fiber 卸载时回写状态（注册即 effect，零新生命周期机制）。
//!
//! 生命周期语义（冻结）：open_mailbox 的 effect 死亡=回收 Pending 名字；
//! register 的 effect 死亡=条目降回 Pending（信箱与未取净消息保留，重注册续投）；
//! subscribe 的 effect 死亡=摘除观测回调。

use super::delivery::Acceptance;
use super::history::HistoryRing;
use super::registry::{self, Registry};
use super::types::{
    Address, DeliveredMessage, HistoryEntry, HistoryQuery, MemberInfo, MessageHandler,
    OutgoingMessage, SentReceipt,
};
use super::{calling_fiber, MessageHub, WATCH_KEY};
use crate::context::RootState;
use crate::error::{CordisError, ErrorKind, Result};
use crate::events::EventOwner;
use crate::fiber::{Disposer, EffectHandle, FiberShared};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// 锁内共享态：成员表+留痕环+全局 log 计数器（全局发送序的唯一真源）。
pub(super) struct CoreInner {
    pub(super) registry: Registry,
    pub(super) ring: HistoryRing,
    /// 下一条消息的全局 log_id（单调递增；受理与入箱在同一锁内完成，保证
    /// 信箱追加序=log 序）。
    pub(super) next_log: u64,
}

/// 契约 message.v1 的进程内实现（内核 root 装配，RootState 挂载）。
pub struct InProcessHub {
    /// 共享态（独立 Arc：disposer 的 'static 捕获来源，见模块注释）。
    core: Arc<Mutex<CoreInner>>,
    /// root 弱引用：观测通知取 runtime 与事件表用。弱引用防 Arc 环
    /// （RootState→hub 强；hub→RootState 弱）；root 消亡（进程收尾）后观测静默
    /// =「尽力而为」语义的自然兜底，不影响投递与留痕。
    root: OnceLock<Weak<RootState>>,
}

impl InProcessHub {
    /// 构造（Context::new 装配点调用；观测通道随后经 [`Self::attach_root`] 接通）。
    pub(crate) fn new() -> Self {
        Self {
            core: Arc::new(Mutex::new(CoreInner {
                registry: Registry::new(),
                ring: HistoryRing::new(),
                next_log: 1,
            })),
            root: OnceLock::new(),
        }
    }

    /// 挂 root 弱引用（Context::new 在 RootState 成 Arc 后调用一次；OnceLock 幂等）。
    pub(crate) fn attach_root(&self, root: &Arc<RootState>) {
        let _ = self.root.set(Arc::downgrade(root));
    }

    /// 锁内快照的锁外通知：watch emit（五分发，尽力而为）+ subscribe 镜像 spawn。
    /// handler 失败记日志不上抛——消费保证在 drain（观测面例外，冻结总纲）。
    fn notify(&self, acceptance: &Acceptance) {
        let Some(root) = self.root.get().and_then(Weak::upgrade) else { return };
        root.events.emit(&root.runtime, WATCH_KEY, &acceptance.watch);
        for (handlers, message) in &acceptance.mirrors {
            for handler in handlers.clone() {
                let message = message.clone();
                root.runtime.spawn(async move {
                    if let Err(err) = handler(&message).await {
                        eprintln!("cordis/警告: message.v1 观测回调失败（尽力而为，不上抛）: {err}");
                    }
                });
            }
        }
    }
}

/// 取调用 fiber（注册类）：无运载=无归属 fiber 上下文 → EffectInactive
/// （对齐 current_fiber_or_inactive / assertActive 既有判据，测试矩阵④）。
fn require_registration_fiber() -> Result<Arc<FiberShared>> {
    calling_fiber().ok_or_else(|| {
        CordisError::new(
            ErrorKind::EffectInactive,
            "当前不在任何 fiber 上下文内：注册类操作须经 Context 身份入口调用",
        )
    })
}

/// 取调用 fiber（消费类）：无运载=无身份 → HubUnknownMember。注册类与消费类的
/// 分型差异是防伪造语义的诚实呈现：无 fiber 无从「绑定成员」，自然无身份可派发。
fn require_member_fiber(action: &str) -> Result<Arc<FiberShared>> {
    calling_fiber().ok_or_else(|| {
        CordisError::new(
            ErrorKind::HubUnknownMember,
            format!("当前不在任何 fiber 上下文内，{action} 被拒：身份由 hub 按调用 fiber 派发"),
        )
    })
}

impl MessageHub for InProcessHub {
    /// 预注册信箱（Pending 态）：spawn_teammate 拉起窗口的消息落点。
    /// EffectHandle 挂调用 fiber——spawn 失败可提前 dispose 回收名字；
    /// 重名（任何态）/空白名 fail-loud。
    fn open_mailbox(&self, name: &str) -> Result<EffectHandle> {
        let fiber = require_registration_fiber()?;
        let name = registry::validate_member_name(name)?;
        let core_for_body = Arc::clone(&self.core);
        let name_for_body = name.clone();
        // 登记动作放进 effect 注册体内（与 ctx.provide 同构）：fiber 已死时
        // effect_with 直接 EffectInactive，状态零残留——先登记后挂 effect 会漏
        // 孤儿 Pending 条目。
        fiber.effect(format!("hub.open_mailbox({name_for_body:?})"), move || {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            core_for_body.lock().expect("MessageHub 状态锁").registry.open_mailbox(&name_for_body)?;
            let core_for_dispose = Arc::clone(&core_for_body);
            let name_for_dispose = name_for_body;
            Ok(Some(Disposer::new(move || {
                let core = Arc::clone(&core_for_dispose);
                let name = name_for_dispose;
                Box::pin(async move {
                    #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                    core.lock().expect("MessageHub 状态锁").registry.reclaim_if_pending(&name);
                })
            })))
        })
    }

    /// 成员上线：绑定当前 fiber。Pending→Online（补投就绪）；无条目→直接
    /// Online（自发成员，如 lead）；已 Online→HubDuplicateMember。
    /// EffectHandle 语义：fiber 死亡→条目降回 Pending（信箱与未取净消息保留，
    /// 同名重注册即续投）——非删除。
    fn register(&self, name: &str) -> Result<EffectHandle> {
        let fiber = require_registration_fiber()?;
        let name = registry::validate_member_name(name)?;
        let owner = EventOwner::of(&fiber);
        let core_for_body = Arc::clone(&self.core);
        let name_for_body = name.clone();
        let fiber_id = owner.fiber_id;
        fiber.effect(format!("hub.register({name_for_body:?})"), move || {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            core_for_body
                .lock()
                .expect("MessageHub 状态锁")
                .registry
                .register(&name_for_body, owner)?;
            let core_for_dispose = Arc::clone(&core_for_body);
            let name_for_dispose = name_for_body;
            Ok(Some(Disposer::new(move || {
                let core = Arc::clone(&core_for_dispose);
                let name = name_for_dispose;
                Box::pin(async move {
                    #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                    core.lock().expect("MessageHub 状态锁").registry.demote(&name, fiber_id);
                })
            })))
        })
    }

    /// 名册消息侧投影（online 标志+归属；Pending 无归属=spawn 窗口可见性）。
    fn members(&self) -> Vec<MemberInfo> {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let core = self.core.lock().expect("MessageHub 状态锁");
        core.registry.members()
    }

    /// 观测面订阅（推，尽力而为）：当前 fiber 绑定成员的入站镜像；handler 失败
    /// 记日志不上抛（消费保证在 drain）。非成员 fiber=HubUnknownMember（与 drain
    /// 同判：观测也须有身份，防匿名挂监听）。
    fn subscribe(&self, handler: MessageHandler) -> Result<EffectHandle> {
        let fiber = require_registration_fiber()?;
        let fiber_id = fiber.id;
        let core_for_body = Arc::clone(&self.core);
        // 身份反查与登记在同一锁窗口内原子完成（防「查完被降级」的窗口错位）。
        fiber.effect("hub.subscribe", move || {
            let name = {
                #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                let mut core = core_for_body.lock().expect("MessageHub 状态锁");
                let name = core.registry.identity_of(fiber_id).ok_or_else(|| {
                    CordisError::new(
                        ErrorKind::HubUnknownMember,
                        "当前 fiber 未绑定成员：观测订阅须在成员 fiber 上下文内",
                    )
                })?;
                core.registry.attach_observer(&name, handler.clone())?;
                name
            };
            let core_for_dispose = Arc::clone(&core_for_body);
            Ok(Some(Disposer::new(move || {
                let core = Arc::clone(&core_for_dispose);
                let handler_for_dispose = handler.clone();
                Box::pin(async move {
                    #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                    core.lock()
                        .expect("MessageHub 状态锁")
                        .registry
                        .detach_observer(&name, &handler_for_dispose);
                })
            })))
        })
    }

    /// 寻址投递：身份由 hub 按调用 fiber 派发（防伪造定案）；Online=入信箱
    /// +观测镜像，Pending=信箱缓冲，未在册=HubUnknownMember。
    /// 同步短临界区（锁内不 await）；广播 All 不含 from 自己。
    fn send(&self, to: Address, message: OutgoingMessage) -> Result<SentReceipt> {
        let fiber = require_member_fiber("send")?;
        let acceptance = {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            let mut core = self.core.lock().expect("MessageHub 状态锁");
            core.send(fiber.id, to, message)?
        }; // 锁在此释放：观测通知绝不持锁（锁外 spawn 纪律）
        self.notify(&acceptance);
        Ok(acceptance.receipt)
    }

    /// 消费通道（拉）：取净当前 fiber 绑定成员的信箱（log_id 升序）；
    /// 非成员 fiber=HubUnknownMember（防越权读他人信箱）。
    fn drain(&self) -> Result<Vec<DeliveredMessage>> {
        let fiber = require_member_fiber("drain")?;
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let mut core = self.core.lock().expect("MessageHub 状态锁");
        core.drain(fiber.id)
    }

    /// 留痕查询：环形 1000 丢最旧；完整历史=各成员会话 JSONL（会话域）。
    fn history(&self, query: HistoryQuery) -> Result<Vec<HistoryEntry>> {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        Ok(self.core.lock().expect("MessageHub 状态锁").ring.query(&query))
    }
}
