//! 服务寻址与服务表（对标 reflect.ts 的 store/Impl 与 context.ts 的 isolate 解析）。
//!
//! 所有权模型（方案 §2.1.2 的引用计数平移）：
//! - 服务表持服务值的 **强引用**（`ArcEntry.value`）；
//! - 依赖方 fiber 的 `committed` 持 **弱引用**（`Weak<ServiceEntry>`）；
//! - 提供者卸载时服务表 drop 强引用，只要还有依赖方的弱引用可升级，实例就活着；
//!   依赖方全部卸载（drop 弱引用）后实例自然析构——等价 TS 的 retain-until-teardown，
//!   释放时机由引用计数保证，无需 TS 的 `impl.release` 手工协议；
//! - 仍保留 `dependents` 集合，用于「卸载顺序：先消费者后提供者」的等待逻辑。
//!
//! 类型键规则：`ServiceId::Typed(TypeId::of::<dyn Trait>())`；trait 必须 object-safe
//! 且 `'static`（异步方法一律返回 `BoxFut`，方案 §2.1.3）。

use crate::error::{CordisError, ErrorKind, Result};
use crate::fiber::{EffectHandle, FiberShared, FiberState};
use crate::registry::notify_services;
use serde_json::Value;
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Notify;

/// 服务标识：强类型键或弱类型字符串键（方案 §2.2 双轨）。
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ServiceId {
    /// 类型键：trait object 的 `TypeId`（如 `ServiceId::of::<dyn Llm>()`）。
    Typed(TypeId),
    /// 字符串键：配置驱动的动态装配。
    Named(Arc<str>),
}

impl ServiceId {
    /// 以 trait object 类型为键。
    pub fn of<P: ?Sized + 'static>() -> Self {
        Self::Typed(TypeId::of::<P>())
    }

    /// 以字符串为键。
    pub fn named(name: impl AsRef<str>) -> Self {
        Self::Named(Arc::from(name.as_ref()))
    }

    /// 展示名（错误信息与日志用）。
    pub fn display(&self) -> String {
        match self {
            Self::Named(name) => name.to_string(),
            Self::Typed(type_id) => format!("TypeId({type_id:?})"),
        }
    }
}

/// 服务隔离作用域：同一 `ScopeId` 即同一作用域（对标 TS 的 isolate symbol）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScopeId(NonZeroU64);

impl ScopeId {
    /// 由 root 计数器分配新作用域（见 [`crate::context::Context::allocate_scope`]）。
    pub(crate) fn from_counter(next: &std::sync::atomic::AtomicU64) -> Self {
        let raw = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        // S-ISO-1 B 不变量断言（K-14 B 类口径）：fetch_add 自 1 起（+1 后永≥1），
        // expect 触发=内核计数器被破坏，非外部可预期路径。
        #[allow(clippy::expect_used)]
        Self(NonZeroU64::new(raw).expect("作用域计数器永不产生 0"))
    }
}

/// 服务条目（对标 reflect.ts 的 `Impl`）。
pub(crate) struct ServiceEntry {
    /// provision 唯一编号（epoch 的组成成分，方案 §2.1.4）。
    pub uid: u64,
    /// 条目固有的服务键（对标 TS Impl.name；诊断与日志保留位，一期错误信息走外层 display）。
    #[allow(dead_code)]
    pub key: ServiceId,
    /// 服务值（强引用）。内容是 `Arc<P>` 的二次装箱：`P` 可为 trait object
    ///（`Arc<dyn Trait>` 本身是 Sized 且 `'static`），取回时 `downcast::<Arc<P>>()` 还原。
    pub value: Arc<dyn Any + Send + Sync>,
    /// 提供者 fiber（strict 读取要求其状态为 Active，reflect.ts:251-257）。
    pub provider: Weak<FiberShared>,
    pub scope: ScopeId,
    /// 正在消费本服务的 fiber id（卸载顺序：先消费者后提供者）。
    pub dependents: Mutex<HashSet<u64>>,
    /// 提供者卸载等待「dependents 清空」的唤醒器。
    pub release: Arc<Notify>,
}

/// 服务表：作用域 → 条目。全内核只此一张表锁（纪律：方案 §2.3）。
#[derive(Default)]
pub(crate) struct ServiceTable {
    pub entries: HashMap<ScopeId, Arc<ServiceEntry>>,
}

impl ServiceTable {
    /// 读取并校验条目可用性：提供者必须仍存活且处于 Active。
    pub(crate) fn get_strict(&self, scope: ScopeId) -> Option<Arc<ServiceEntry>> {
        let entry = self.entries.get(&scope).cloned()?;
        let ready = entry
            .provider
            .upgrade()
            .is_some_and(|p| p.state() == FiberState::Active);
        ready.then_some(entry)
    }
}

impl crate::context::ContextInner {
    /// 解析服务的作用域：沿上下文链（子→父）找 isolate 覆盖，否则用 root 默认表
    /// （首次使用时分配，对标 root.isolate[name] ??= Symbol(name)）。
    pub(crate) fn resolve_scope(&self, key: &ServiceId) -> ScopeId {
        let mut node = Some(self);
        while let Some(current) = node {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            if let Some(scope) = current.isolate.lock().expect("isolate 锁").get(key) {
                return *scope;
            }
            node = current.parent.as_deref();
        }
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let mut defaults = self.root.default_scopes.lock().expect("默认作用域锁");
        *defaults.entry(key.clone()).or_insert_with(|| ScopeId::from_counter(&self.root.next_scope))
    }

    /// 收集某服务的 intercept 配置段，**祖先在前、自身在后**（对齐 service.ts:86-102
    /// 沿原型链 unshift 的顺序；合并时后者覆盖前者）。
    pub(crate) fn collect_intercept(&self, key: &ServiceId) -> Vec<Value> {
        let mut out = Vec::new();
        let mut node = Some(self);
        while let Some(current) = node {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            if let Some(value) = current.intercept.lock().expect("intercept 锁").get(key) {
                out.push(value.clone());
            }
            node = current.parent.as_deref();
        }
        out.reverse();
        out
    }
}

impl crate::context::Context {
    /// 注册服务实现（对标 reflect.ts:291 `provide`）。
    ///
    /// 注册体作为当前 fiber 的一个 effect：fiber 卸载时自动注销（先等消费者卸载）。
    /// 若 fiber 已 Active（迟注册场景），立即广播服务表变更。
    pub fn provide<P>(
        &self,
        key: ServiceId,
        value: Arc<P>,
    ) -> Result<EffectHandle>
    where
        P: ?Sized + Send + Sync + 'static,
    {
        let fiber = self.current_fiber().ok_or_else(|| {
            CordisError::new(ErrorKind::EffectInactive, "当前上下文没有归属的 fiber，无法 provide")
        })?;
        let display = key.display();
        let inner = self.inner.clone();
        let key_for_body = key.clone();
        let fiber_for_body = fiber.clone();
        let handle = fiber.effect_with(format!("ctx.provide({display})"), move || {
            let scope = inner.resolve_scope(&key_for_body);
            // 二次装箱：Arc<P>（P 可为 dyn Trait）整体擦除为 Any；取回见 get
            let erased: Arc<dyn Any + Send + Sync> = Arc::new(value);
            let entry = Arc::new(ServiceEntry {
                uid: inner.root.next_uid.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                key: key_for_body.clone(),
                value: erased,
                provider: Arc::downgrade(&fiber_for_body),
                scope,
                dependents: Mutex::new(HashSet::new()),
                release: Arc::new(Notify::new()),
            });
            {
                #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                let mut table = inner.root.services.write().expect("服务表锁");
                if let Some(existing) = table.entries.get(&scope) {
                    let owner = existing
                        .provider
                        .upgrade()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "<已卸载的提供者>".into());
                    return Err(CordisError::new(
                        ErrorKind::ServiceDuplicate,
                        format!("服务 \"{display}\" 已由 <{owner}> 注册，作用域冲突"),
                    ));
                }
                table.entries.insert(scope, entry.clone());
            }
            // 注销器：摘表 → 广播 → 兜底等待消费者清空（对齐 reflect.ts:311-325）。
            // §2.1.2 约束：disposer 只持 Arc<RootState>（scope 已存 entry），
            // 绝不捕获 ContextInner 强引用——否则 setup 落定后强计数不回落。
            let root_for_dispose = inner.root.clone();
            let entry_for_dispose = Arc::downgrade(&entry);
            let disposer = crate::fiber::Disposer::new(move || {
                let root = root_for_dispose.clone();
                let entry = entry_for_dispose.clone();
                Box::pin(async move {
                    let Some(entry) = entry.upgrade() else { return };
                    {
                        // 摘表限定在独立作用域：写锁绝不跨 await（纪律：§2.3）
                        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                        let mut table = root.services.write().expect("服务表锁");
                        if table.entries.get(&entry.scope).is_some_and(|e| e.uid == entry.uid) {
                            table.entries.remove(&entry.scope);
                        }
                    }
                    notify_services(&root);
                    // 兜底：极端时序下消费者可能刚登记，等它卸载完再返回
                    loop {
                        // 锁不得跨 await：先取快照再等待
                        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                        let busy = !entry.dependents.lock().expect("dependents 锁").is_empty();
                        if !busy {
                            break;
                        }
                        entry.release.notified().await;
                    }
                })
            });
            // provisions 登记由 effect_with 在同一决策锁窗口内完成（勿在此 lock_core）
            Ok(crate::effect::EffectOutcome {
                disposer: Some(disposer),
                provision: Some(Arc::downgrade(&entry)),
            })
        })?;
        // 迟注册（fiber 已 Active）需要立即唤醒等待该服务的依赖方
        if fiber.state() == FiberState::Active {
            notify_services(&self.inner.root);
        }
        Ok(handle)
    }

    /// 读取服务实现（对标 reflect.ts:152/247）。strict 语义：仅提供者 Active 时可见，
    /// 唯一例外见 [`Context::admitted_entry`]（已准入保护，装载竞态修复）。
    pub fn get<P>(&self, key: ServiceId) -> Result<Arc<P>>
    where
        P: ?Sized + Send + Sync + 'static,
    {
        let scope = self.inner.resolve_scope(&key);
        let entry = {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            let table = self.inner.root.services.read().expect("服务表锁");
            table.get_strict(scope)
        }
        .or_else(|| self.admitted_entry(scope))
        .ok_or_else(|| {
            CordisError::new(
                ErrorKind::ServiceNotFound,
                format!("服务 \"{}\" 未注册或提供者未就绪", key.display()),
            )
        })?;
        let holder = Arc::clone(&entry.value);
        holder
            .downcast::<Arc<P>>()
            .map(|wrapped| Arc::clone(&*wrapped))
            .map_err(|_| {
                CordisError::new(
                    ErrorKind::ServiceTypeMismatch,
                    format!("服务 \"{}\" 的注册类型与请求类型不符", key.display()),
                )
            })
    }

    /// 已准入保护（任务41 P1·装载竞态本体修复；内核语义变更，云枢 retro 线留痕，
    /// lead 2026-11-15 批「已准入保护」方案）。
    ///
    /// 装载引擎准入序是「快照成功即 retain_as_dependent 登记」（load.rs run_load ①），
    /// 「先消费者后提供者」据此让提供者卸载等待被准入的消费者；但 [`ServiceTable::get_strict`]
    /// 只查提供者 state==Active，而提供者 dispose 先置 Unloading 再进卸载任务等
    /// dependents 清空——消费者快照与 setup 的 get 之间被抢占即撞 ServiceNotFound，
    /// setup Err → Failed（而非收敛 Pending）。本保护只放宽「已被本 fiber 准入且
    /// 条目仍存活」的读取：committed 快照里的条目在消费者持 dependent 登记期间
    /// 必然仍在表（提供者 disposer 等 dependents 清空后才摘表，登记未释放=摘表被
    /// 阻塞），upgrade 成功即实例存活可安全取用——与所有权模型「依赖方弱引用维持
    /// 实例存活」同源。epoch 重算与 run_load 快照仍走 get_strict 原语义（零改动）；
    /// 收敛路径：setup 完成→epoch 已被重算为 Inactive→卸载→release_committed_deps
    /// →提供者续卸→消费者终态 Pending，无死锁。回归锁见 tests/lifecycle_race.rs。
    fn admitted_entry(&self, scope: ScopeId) -> Option<Arc<ServiceEntry>> {
        let fiber = self.current_fiber()?;
        fiber.lock_core(|core| {
            core.committed.iter().find_map(|weak| {
                weak.upgrade().filter(|entry| entry.scope == scope)
            })
        })
    }
}

/// 从 `committed` 快照中摘除本 fiber 的依赖登记；当某条目的消费者清空时唤醒其提供者
/// （对齐 fiber.ts:732-736）。装卸引擎与 provide 注销器共用。
pub(crate) async fn release_committed_deps(fiber_id: u64, committed: Vec<Weak<ServiceEntry>>) {
    release_committed_deps_sync(fiber_id, committed);
}

/// 同步释放内函数（task-28 回滚分支消费：rollback 在决策锁内运行，锁内不得
/// await——本函数体本无 await 面，锁摘除+notify_one 天然同步；async 版保留
/// 为既有调用方的签名兼容面）。
pub(crate) fn release_committed_deps_sync(fiber_id: u64, committed: Vec<Weak<ServiceEntry>>) {
    for dep in committed {
        let Some(entry) = dep.upgrade() else { continue };
        let now_empty = {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            let mut deps = entry.dependents.lock().expect("dependents 锁");
            deps.remove(&fiber_id);
            deps.is_empty()
        };
        if now_empty {
            entry.release.notify_one();
        }
    }
}

/// 供装卸引擎使用：把一批弱引用条目登记为本 fiber 的依赖方（对齐 fiber.ts:681-686）。
pub(crate) fn retain_as_dependent(entry: &Arc<ServiceEntry>, fiber_id: u64) {
    #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
    entry.dependents.lock().expect("dependents 锁").insert(fiber_id);
}
