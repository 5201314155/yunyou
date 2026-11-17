//! 契约⑧「只发所属 fiber」：事件归属元数据 + 归属监听表（对标 events.ts:118-122,
//! 162-171 的「监听器带 ctx + global 标志，分发时按 context filter 过滤」）。
//!
//! 与既有五分发的关系：`emit / parallel / serial / bail / waterfall` 走**无归属**
//! 监听表（`EventTable::listeners` / `waterfall`），语义零改动——本表是在进程级
//! 单例结构上**叠加**的归属元数据，只服务契约⑧事件面（`volatile-update` 与
//! S5 凭据的 `reference-updated` 预留位）。两张表按事件键互不串台：
//! 归属分发绝不投递给 legacy 监听器，legacy 分发也绝不投递归属监听器。

use super::{current_fiber_or_inactive, Listener};
use crate::error::Result;
use crate::fiber::EffectHandle;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

/// 事件归属元数据：事件由哪个 fiber 触发（契约⑧ payload 前两位即 [`EventOwner`]）。
///
/// fiber 身份用 `id + 插件名`：`id` 做归属精确过滤，`name` 随 payload 下发，
/// 让跨 fiber 调试时能直接从事件里读出「这是谁触发的」。
///
/// serde derive 系 message.v1 冻结面所需（MemberInfo.fiber: Option\<EventOwner\>
/// 全类型 serde 全覆盖=二期 IPC schema 种子）——纯加法 derive、语义零改动，
/// 云枢 retro 项②留痕（2026-11-15 task-15 实现批挂账）。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventOwner {
    /// 所属 fiber id（root fiber 为 0）。
    pub fiber_id: u64,
    /// 所属 fiber 的插件名。
    pub fiber_name: String,
}

impl EventOwner {
    /// root fiber 归属（root 上的注册与事件，如 boot 流程的 provide）。
    pub fn root() -> Self {
        Self { fiber_id: 0, fiber_name: "root".into() }
    }

    /// 按 fiber 生成归属元数据（内核内部用；对外经 `Context::emit_owned`）。
    pub(crate) fn of(fiber: &crate::fiber::FiberShared) -> Self {
        Self { fiber_id: fiber.id, fiber_name: fiber.name.clone() }
    }
}

/// 归属监听表：`事件键 -> [(监听器, 归属 fiber id)]`（每个 fiber 一份自己的桶）。
#[derive(Default)]
pub(crate) struct ScopedTable {
    listeners: Mutex<HashMap<String, Vec<(Listener, u64)>>>,
}

impl ScopedTable {
    /// 登记归属监听器（`fiber_id` 由注册时的当前 fiber 决定，随 fiber 卸载注销）。
    pub(crate) fn add(&self, event: &str, fiber_id: u64, listener: Listener) {
        #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
        self.listeners
            .lock()
            .expect("归属监听表锁")
            .entry(event.to_string())
            .or_default()
            .push((listener, fiber_id));
    }

    /// 按指针相等移除归属监听器。
    pub(crate) fn remove(&self, event: &str, listener: &Listener) -> bool {
        #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
        let mut table = self.listeners.lock().expect("归属监听表锁");
        let Some(bucket) = table.get_mut(event) else {
            return false;
        };
        let Some(index) = bucket.iter().position(|(h, _)| Arc::ptr_eq(h, listener)) else {
            return false;
        };
        bucket.remove(index);
        if bucket.is_empty() {
            table.remove(event);
        }
        true
    }

    /// 归属分发：只投递给注册在 `owner.fiber_id` 上的监听器。
    ///
    /// 过滤发生在**快照阶段**（未归属的跨 fiber 监听器连 spawn 都不发生），
    /// 不是把事件广播出去再让监听器自己判断要不要理——跨 fiber 监听器零成本不收。
    /// payload 由内核补上前两位 `[fiber id, fiber 插件名]`，再拼调用方的 args，
    /// 与 `internal/*` 事件「先身份后数据」的排布一致。
    pub(crate) fn emit(
        &self,
        runtime: &Handle,
        event: &str,
        owner: &EventOwner,
        args: &[Value],
    ) {
        let listeners = self.snapshot(event, owner.fiber_id);
        let mut payload = Vec::with_capacity(args.len() + 2);
        payload.push(json!(owner.fiber_id));
        payload.push(Value::String(owner.fiber_name.clone()));
        payload.extend_from_slice(args);
        let shared = Arc::new(payload);
        for listener in listeners {
            let args = shared.clone();
            runtime.spawn(async move {
                let _ = listener(&args).await;
            });
        }
    }

    /// 取指定 fiber 的监听器快照（同 `Listener` clone 语义：锁只在快照期持有）。
    fn snapshot(&self, event: &str, fiber_id: u64) -> Vec<Listener> {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let table = self.listeners.lock().expect("归属监听表锁");
        table
            .get(event)
            .map(|bucket| {
                bucket
                    .iter()
                    .filter(|(_, id)| *id == fiber_id)
                    .map(|(listener, _)| listener.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl crate::events::EventTable {
    /// 归属分发（契约⑧）：只投递给注册在 `owner.fiber_id` 上的监听器。
    ///
    /// 与五分发的关系：这是进程级单例结构上**叠加**的归属元数据通道，
    /// 不过 `listeners` 表、不改五分发任何语义；filter 在快照阶段完成，
    /// 跨 fiber 的监听器连 spawn 都不发生（见 [`ScopedTable::emit`]）。
    pub fn emit_owned(&self, runtime: &Handle, event: &str, owner: &EventOwner, args: &[Value]) {
        self.scoped.emit(runtime, event, owner, args);
    }

    /// 登记归属监听器：`fiber_id` 决定它能收到谁的事件（随该 fiber 卸载注销）。
    pub fn add_owned_listener(&self, event: &str, fiber_id: u64, listener: Listener) {
        self.scoped.add(event, fiber_id, listener);
    }

    /// 按指针相等移除归属监听器。
    pub fn remove_owned_listener(&self, event: &str, listener: &Listener) -> bool {
        self.scoped.remove(event, listener)
    }
}

impl crate::context::Context {
    /// 注册「只收所属 fiber 事件」的监听器（契约⑧，S6 volatile 事件面）。
    ///
    /// 与 `Context::on` 的差别只在归属：监听器随当前 fiber 的 effect 卸载自动
    /// 注销，且只有同一 fiber 经 `Context::emit_owned` 发的事件投得到它（跨
    /// fiber 的同名事件一律收不到）。volatile 热更通知等场景用这个，避免进程级
    /// 单例把事件广播给不相干的 fiber。
    pub fn on_owned(&self, event: &str, listener: Listener) -> Result<EffectHandle> {
        let fiber = current_fiber_or_inactive(self)?;
        let root = self.inner.root.clone();
        let key = event.to_string();
        let fiber_id = fiber.id;
        let listener_for_dispose = listener.clone();
        fiber.effect(format!("ctx.on_owned({event:?})"), move || {
            root.events.add_owned_listener(&key, fiber_id, listener);
            let root_for_dispose = root.clone();
            let key_for_dispose = key;
            Ok(Some(crate::fiber::Disposer::new(move || {
                let root = root_for_dispose.clone();
                let key = key_for_dispose.clone();
                let listener = listener_for_dispose.clone();
                Box::pin(async move {
                    root.events.remove_owned_listener(&key, &listener);
                })
            })))
        })
    }

    /// 以当前归属 fiber 为 owner 发事件（契约⑧）：只有注册在**同一 fiber**上的
    /// `Context::on_owned` 监听器收得到；payload 前两位由内核补 `[fiber id, 插件名]`。
    ///
    /// 没有归属 fiber 的上下文直接空转，不广播。典型发送方是
    /// [`crate::volatile::Volatile::update`]（自带 owner 元数据、不经本便捷方法，
    /// 以免换了个 ctx 就把 owner 发错）。
    pub fn emit_owned(&self, event: &str, args: &[Value]) {
        let Some(fiber) = self.current_fiber() else {
            return;
        };
        let owner = EventOwner::of(&fiber);
        self.inner
            .root
            .events
            .emit_owned(&self.inner.root.runtime, event, &owner, args);
    }
}
