//! volatile 配置底座（插件化契约⑧ / 内核方案 §2.2，对标 cosmokit `volatile.ts:9-45`）。
//!
//! 语义（方案 §2.2「volatile 热更新」：TS 机制里 Rust 可**无损平移**的一块）：
//! - 消费方持 [`Volatile`]（一般 `Arc<Volatile<T>>`）反复 `get()`，每次拿到一个
//!   **不可变快照** `Arc<T>`——`update` 原子换槽后，已取出的旧快照依旧有效，
//!   不会出现读到「半新半旧」的中间态；
//! - `update` 换槽后向**所属 fiber** 发 `volatile-update` 事件（契约⑧，只发所属
//!   fiber），通知消费方重读快照，**不触发重挂载**（重启走 `internal/update`
//!   waterfall，不归本模块管）；
//! - 凭据的 `reference-updated` 事件位已在 [`crate::events::keys`] 预留（S5
//!   凭据服务落地的消费方），本模块只负责 volatile 这一半，不实现凭据服务。
//!
//! 生命周期纪律（方案 §2.1.2）：构造时只借 `&Context`，随即降级 `Weak` 持有——
//! 组件长期只许持 [`crate::context::WeakContext`] 或服务句柄，不得留
//! `Arc<Context>` 强引用（lifecycle 测试有强计数断言）。

use crate::context::{Context, ContextInner};
use crate::events::{keys, EventOwner};
use arc_swap::ArcSwap;
use std::sync::{Arc, Weak};

/// volatile 配置槽（对标 cosmokit `Volatile<T>`，方案 §2.2 用 `ArcSwap` 平移）。
///
/// `T` 受 `Send + Sync + 'static` 约束（内核硬纪律：一切共享状态多线程安全，
/// 方案 §2.3）。槽本体可 `Clone` 前的 `Arc` 化由调用方决定（服务导出通常
/// 直接存 `Arc<Volatile<T>>`）。
pub struct Volatile<T: Send + Sync + 'static> {
    inner: ArcSwap<T>,
    /// 归属 fiber（`volatile-update` 的 owner；跨 fiber 监听器不会收到）。
    owner: EventOwner,
    /// 来源上下文（弱持有：只在发事件时升级，绝不长期持强句柄）。
    ctx: Weak<ContextInner>,
    /// 可读标签：随事件第三位下发，便于一个 fiber 持多槽时辨认改了哪一槽。
    label: Option<String>,
}

impl<T: Send + Sync + 'static> Volatile<T> {
    /// 在当前上下文（即当前归属 fiber）上构造一个无标签 volatile 槽。
    pub fn new(ctx: &Context, value: T) -> Self {
        Self::labeled(ctx, value, None::<String>)
    }

    /// 带标签构造：标签建议填配置路径或字段名（如 `"llm.model"`）。
    pub fn labeled(ctx: &Context, value: T, label: Option<impl Into<String>>) -> Self {
        let owner = match ctx.current_fiber() {
            Some(fiber) => EventOwner::of(&fiber),
            None => EventOwner::root(),
        };
        Self {
            inner: ArcSwap::new(Arc::new(value)),
            owner,
            ctx: Arc::downgrade(&ctx.inner),
            label: label.map(Into::into),
        }
    }

    /// 取不可变快照：`update` 之后旧快照依旧有效（`ArcSwap` 语义，读不锁、写不锁）。
    pub fn get(&self) -> Arc<T> {
        self.inner.load_full()
    }

    /// 原子替换并通知所属 fiber（不重挂载）；返回新快照。
    ///
    /// 事件在 `update` 内同步发送（fire-and-forget，与 `Context::emit` 同口径：
    /// 不等监听器结果，异步任务里的错误按 TS 语义丢弃）。
    pub fn update(&self, value: T) -> Arc<T> {
        let next = Arc::new(value);
        self.inner.store(next.clone());
        self.emit_update();
        next
    }

    /// 本槽的归属 fiber 元数据（诊断/测试用：确认事件会发给谁）。
    pub fn owner(&self) -> &EventOwner {
        &self.owner
    }

    /// 本槽标签（进入 `volatile-update` 事件 payload 第三位）。
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// 向所属 fiber 发 `volatile-update`：payload `[fiber id, fiber 名, 标签]`。
    ///
    /// ctx 已随所属 fiber 消亡（`Weak` 升级失败）时静默跳过：没有 fiber 就没有
    /// 「所属」可言，不广播到别家。
    fn emit_update(&self) {
        let Some(ctx) = self.ctx.upgrade() else {
            return;
        };
        let tag = match &self.label {
            Some(label) => serde_json::Value::String(label.clone()),
            None => serde_json::Value::Null,
        };
        ctx.root
            .events
            .emit_owned(&ctx.root.runtime, keys::VOLATILE_UPDATE, &self.owner, &[tag]);
    }
}