//! 事件总线：五种分发模式 + waterfall 重入防线 + 契约⑧「只发所属 fiber」归属分发。
//!
//! - `emit`：立即分发不等结果（TS 同步 fire-and-forget；异步任务里的错误按 TS 语义丢弃）；
//! - `parallel`：并发等全部，错误聚合为 Aggregate（对标 allSettled + AggregateError）；
//! - `serial`：顺序等待，bail 即停；`bail`：语义同 serial——Rust 监听器统一异步形态，
//!   TS 的「同步 bail 快路径」不适用，保留两个名字对齐五分发口径（差异已报备）；
//! - `waterfall`：监听器包裹 `next()`；不调 `next()` 即否决（含内置行为），
//!   重复调用报 WaterfallReentry（对标 events.ts:254，专项测试见 tests/lifecycle.rs）
//!   ——该分发的链机器已按职责拆到子模块 [`waterfall`]，本文件只留五分发口径；
//! - **归属分发（契约⑧，S6）**：`emit_owned` 只投递给注册在**同一 fiber** 上的
//!   `on_owned` 监听器（对标 events.ts:118-122,162-171 的 context filter；过滤在
//!   快照阶段完成，跨 fiber 监听器零成本不收），细节见子模块 [`scope`]。
//!
//! 事件名为字符串，`internal/*` 为内核保留段；payload 统一 [`serde_json::Value`]。
//! 归属分发的 payload 由内核补前两位 `[fiber id, fiber 插件名]`，再拼调用方数据。

use crate::error::{CordisError, ErrorKind, Result};
use crate::fiber::{EffectHandle, FiberShared};
use yunyou_util::BoxFut;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::task::JoinSet;

mod scope;
mod waterfall;

pub use scope::EventOwner;
pub use waterfall::{WaterfallBuiltin, WaterfallListener, WaterfallNext};

/// 事件键（字符串）。
pub type EventKey = str;

/// 内核事件名常量：`internal/*` 保留段（对标 events.ts:334-371 的 internal/* 家族）
/// + 契约⑧对外事件面（S6 volatile 与 S5 凭据预留位）。
pub mod keys {
    /// fiber 生命周期状态迁移：`[插件名, 旧状态, 新状态]`。
    pub const STATUS: &str = "internal/status";
    /// 配置更新瀑布（保留实例或重启）。
    pub const UPDATE: &str = "internal/update";
    /// 配置提交（持久化）通知：`[插件名]`。
    pub const COMMIT: &str = "internal/commit";
    /// volatile 配置热替换（契约⑧，对标官方 `loader/volatile-update`）。
    ///
    /// 只发所属 fiber（[`EventOwner`]），payload 前两位 `[fiber id, fiber 插件名]`、
    /// 第三位为 volatile 槽标签（无则 `null`）。语义=**通知消费方重读快照，不重挂载**
    /// （重启走 [`UPDATE`] waterfall，不归本事件管）。发送方：[`crate::volatile::Volatile::update`]。
    pub const VOLATILE_UPDATE: &str = "volatile-update";
    /// 凭据引用变更（契约⑧，S5 凭据服务预留事件位）。
    ///
    /// **当前只定义不发送**：凭据服务（S5）落地时，凭据刷新按官方语义
    /// 「只通知持有该引用的所属 fiber 重解析」经 `emit_owned` 发送，与
    /// volatile 同出一条归属分发机制。S6 只占位，不实现凭据服务。
    pub const CREDENTIALS_REFERENCE_UPDATED: &str = "reference-updated";
}

/// 普通监听器的返回：继续，或带值否决（对齐 isBailed 的判定语义）。
#[derive(Debug, Clone, PartialEq)]
pub enum ListenerOutcome {
    /// 继续后续监听器。
    Continue,
    /// 停止分发并携带 bail 值。
    Bail(Value),
}

/// 普通监听器（emit/parallel/serial/bail 用）。
pub type Listener =
    Arc<dyn Fn(&[Value]) -> BoxFut<'static, Result<ListenerOutcome>> + Send + Sync>;

/// 事件监听表（挂在 [`crate::context::RootState`]，全内核一份）。
#[derive(Default)]
pub struct EventTable {
    /// 无归属监听器（五分发用；进程级单例，与归属表按事件键互不串台）。
    listeners: Mutex<HashMap<String, Vec<Listener>>>,
    /// waterfall 监听器（注册/移除/链执行在子模块 [`waterfall`]）。
    waterfall: Mutex<HashMap<String, Vec<WaterfallListener>>>,
    /// 归属监听器（契约⑧「只发所属 fiber」；携带归属 fiber id，见 [`scope::ScopedTable`]）。
    scoped: scope::ScopedTable,
}

impl EventTable {
    /// emit：立即分发、不等结果（错误按 TS fire-and-forget 语义丢弃；需要观测用 parallel）。
    pub fn emit(&self, runtime: &Handle, event: &str, args: &[Value]) {
        let listeners = self.snapshot_listeners(event);
        let shared = Arc::new(args.to_vec());
        for listener in listeners {
            let args = shared.clone();
            runtime.spawn(async move {
                let _ = listener(&args).await;
            });
        }
    }

    /// parallel：并发执行全部监听器并等待，失败聚合为 Aggregate。
    pub async fn parallel(&self, event: &str, args: &[Value]) -> Result<()> {
        let listeners = self.snapshot_listeners(event);
        // payload 共享（task-36⑤，挑刺 P3-5）：与 emit 同款 Arc<Vec<Value>>——
        // N 监听器 N 次深拷贝收口为 1 次分配+N 次 Arc 升引用。
        let shared = Arc::new(args.to_vec());
        let mut set: JoinSet<Result<ListenerOutcome>> = JoinSet::new();
        for listener in listeners {
            let args = shared.clone();
            set.spawn(async move { listener(&args).await });
        }
        let mut errors = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => errors.push(err),
                Err(join_err) => {
                    errors.push(CordisError::new(ErrorKind::Internal, join_err.to_string()))
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(CordisError::aggregate(errors))
        }
    }

    /// serial / bail：顺序等待，bail 即停并携带 bail 值。
    ///
    /// 监听器在独立任务中执行：单个监听器 panic 不会击穿分发（归入 Err 返回）。
    pub async fn serial(&self, event: &str, args: &[Value]) -> Result<Option<Value>> {
        let listeners = self.snapshot_listeners(event);
        let shared = Arc::new(args.to_vec());
        for listener in listeners {
            let args = shared.clone();
            let joined = tokio::spawn(async move { listener(&args).await }).await;
            match joined {
                Ok(Ok(ListenerOutcome::Continue)) => {}
                Ok(Ok(ListenerOutcome::Bail(value))) => return Ok(Some(value)),
                Ok(Err(err)) => return Err(err),
                Err(join_err) => {
                    return Err(CordisError::new(
                        ErrorKind::Internal,
                        format!("监听器执行异常: {join_err}"),
                    ))
                }
            }
        }
        Ok(None)
    }

    /// 注册普通监听器。
    pub fn add_listener(&self, event: &str, listener: Listener) {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        self.listeners
            .lock()
            .expect("监听表锁")
            .entry(event.to_string())
            .or_default()
            .push(listener);
    }

    /// 按指针相等移除普通监听器。
    pub fn remove_listener(&self, event: &str, listener: &Listener) -> bool {
        // S-ISO-1 逐点豁免（属性不落实参表达式，提行 let 化）
        #[allow(clippy::expect_used)] // 中毒 tripwire（A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let mut listeners_guard = self.listeners.lock().expect("监听表锁");
        remove_by_ptr(&mut listeners_guard, event, |h| {
            Arc::ptr_eq(h, listener)
        })
    }

    fn snapshot_listeners(&self, event: &str) -> Vec<Listener> {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let listeners_guard = self.listeners.lock().expect("监听表锁");
        listeners_guard
            .get(event)
            .cloned()
            .unwrap_or_default()
    }
}

/// 通用摘除：键存在则按谓词移除首个匹配，空表顺手清键。
pub(crate) fn remove_by_ptr<T>(
    table: &mut HashMap<String, Vec<T>>,
    event: &str,
    matches: impl Fn(&T) -> bool,
) -> bool {
    let Some(bucket) = table.get_mut(event) else {
        return false;
    };
    if let Some(index) = bucket.iter().position(matches) {
        bucket.remove(index);
        if bucket.is_empty() {
            table.remove(event);
        }
        true
    } else {
        false
    }
}

impl crate::context::Context {
    /// 立即分发事件（见 [`EventTable::emit`]）。
    pub fn emit(&self, event: &str, args: &[Value]) {
        self.inner
            .root
            .events
            .emit(&self.inner.root.runtime, event, args);
    }

    /// 并发分发并等待全部（见 [`EventTable::parallel`]）。
    pub async fn parallel(&self, event: &str, args: &[Value]) -> Result<()> {
        self.inner.root.events.parallel(event, args).await
    }

    /// 顺序分发，bail 即停（见 [`EventTable::serial`]）。
    pub async fn serial(&self, event: &str, args: &[Value]) -> Result<Option<Value>> {
        self.inner.root.events.serial(event, args).await
    }

    /// 同 [`Context::serial`]（保留 TS 五分发的名字；Rust 下行为一致，见模块注释）。
    pub async fn bail(&self, event: &str, args: &[Value]) -> Result<Option<Value>> {
        self.inner.root.events.serial(event, args).await
    }

    /// waterfall 分发（见 [`EventTable::waterfall`]）。
    pub async fn waterfall(
        &self,
        event: &str,
        args: &[Value],
        builtin: WaterfallBuiltin,
    ) -> Result<Value> {
        self.inner.root.events.waterfall(event, args, builtin).await
    }

    /// 注册普通监听器，归属当前 fiber 的 effect（卸载自动注销，方案 §2.1.4）。
    pub fn on(&self, event: &str, listener: Listener) -> Result<EffectHandle> {
        let fiber = current_fiber_or_inactive(self)?;
        let root = self.inner.root.clone();
        let key = event.to_string();
        let listener_for_dispose = listener.clone();
        fiber.effect(format!("ctx.on({event:?})"), move || {
            root.events.add_listener(&key, listener);
            let root_for_dispose = root.clone();
            let key_for_dispose = key;
            Ok(Some(crate::fiber::Disposer::new(move || {
                let root = root_for_dispose.clone();
                let key = key_for_dispose.clone();
                let listener = listener_for_dispose.clone();
                Box::pin(async move {
                    root.events.remove_listener(&key, &listener);
                })
            })))
        })
    }

    /// 注册 waterfall 监听器，归属当前 fiber 的 effect。
    pub fn on_waterfall(&self, event: &str, listener: WaterfallListener) -> Result<EffectHandle> {
        let fiber = current_fiber_or_inactive(self)?;
        let root = self.inner.root.clone();
        let key = event.to_string();
        let listener_for_dispose = listener.clone();
        fiber.effect(format!("ctx.on_waterfall({event:?})"), move || {
            root.events.add_waterfall(&key, listener);
            let root_for_dispose = root.clone();
            let key_for_dispose = key;
            Ok(Some(crate::fiber::Disposer::new(move || {
                let root = root_for_dispose.clone();
                let key = key_for_dispose.clone();
                let listener = listener_for_dispose.clone();
                Box::pin(async move {
                    root.events.remove_waterfall(&key, &listener);
                })
            })))
        })
    }
}

/// 取当前归属 fiber，否则报 EffectInactive（对齐 assertActive）。
pub(crate) fn current_fiber_or_inactive(
    ctx: &crate::context::Context,
) -> Result<Arc<FiberShared>> {
    ctx.current_fiber().ok_or_else(|| {
        CordisError::new(ErrorKind::EffectInactive, "当前上下文没有归属的 fiber")
    })
}
