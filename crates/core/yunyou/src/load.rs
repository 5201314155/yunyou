//! 装卸引擎：epoch 驱动的装载/卸载任务、状态迁移、update waterfall、restart 与
//! dispose（对标 fiber.ts 的 _setEpoch/_reload/_unload/update/restart）。
//!
//! 驱动模型（对齐 TS 的 inertia 链）：
//! - [`drive`] 在锁内重算 epoch，epoch 变化且无任务在跑时 spawn 装载/卸载任务；
//! - 装载/卸载任务收尾时再次检查 epoch：被超越（superseded）则交接给反向任务，
//!   保证「依赖快速翻转」时最终一致，不叠加并发任务；
//! - 卸载顺序：先消费者后提供者——提供者先离开 Active（边界通知触发消费者卸载），
//!   再等待自己提供的服务 `dependents` 清空，然后逆序清理 effect。

use crate::config::ConfigValue;
use crate::context::Context;
use crate::error::CordisError;
use crate::events::keys;
use crate::ops::resolve_plugin_config;
use crate::fiber::{Epoch, FiberShared, FiberState};
use crate::registry::{notify_services, Plugin};
use crate::service::{release_committed_deps, retain_as_dependent, ServiceEntry};
use serde_json::Value;
use std::sync::{Arc, Weak};

// 配置解析（resolve_plugin_config）与 update/restart/dispose 等生命周期操作
// 见 `ops.rs`；本文件只保留 epoch 驱动的装卸引擎。

/// 锁内重算 epoch（对齐 fiber.ts:617-628 `_refresh`）。
///
/// epoch = Revision + 各就绪依赖的 provision uid；缺任一依赖即 Inactive。
fn refresh_locked(fiber: &Arc<FiberShared>, core: &mut crate::fiber::FiberCore) -> Epoch {
    let Some(plugin) = core.plugin.clone() else {
        return Epoch::Active { revision: core.revision, dep_uids: Vec::new() };
    };
    let Some(ctx) = fiber.ctx.clone() else {
        return Epoch::Inactive;
    };
    let mut dep_uids = Vec::new();
    for key in plugin.inject() {
        let scope = ctx.resolve_scope(&key);
        let entry = {
            #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
            let table = ctx.root.services.read().expect("服务表锁");
            table.get_strict(scope)
        };
        match entry {
            Some(entry) => dep_uids.push(entry.uid),
            // strict 不可见（未注册 / 提供者非 Active / 作用域不符）→ 依赖未就绪
            None => return Epoch::Inactive,
        }
    }
    Epoch::Active { revision: core.revision, dep_uids }
}

impl FiberShared {
    /// epoch 变化驱动装卸（对齐 fiber.ts:630-646 `_setEpoch`）。
    ///
    /// - FAILED fiber 不自动变化，只能 restart 复位（fiber.ts:634）；
    /// - Active→Active'（restart/reload 场景）同样走卸载路径（原地卸了重装）。
    pub(crate) fn drive(self: &Arc<Self>) {
        enum Action {
            Load,
            Unload,
        }
        let action = {
            #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
            let mut core = self.core.lock().expect("fiber 决策锁");
            if core.plugin.is_none() || core.uid.is_none() || core.error.is_some() {
                return;
            }
            let new_epoch = refresh_locked(self, &mut core);
            if new_epoch == core.epoch {
                return;
            }
            let old = core.epoch.clone();
            // 关键（对齐 fiber.ts:649-651 的 _setEpoch）：epoch 更新先于 busy 检查——
            // 装卸任务运行中也要记录最新 epoch，任务收尾时据此判定被超越（superseded）；
            // 若 busy 时跳过记录，装载完成会用旧 epoch 误判 settled，留下幽灵 Active。
            core.epoch = new_epoch;
            if core.busy {
                return;
            }
            core.busy = true;
            if old == Epoch::Inactive { Action::Load } else { Action::Unload }
        };
        match action {
            Action::Load => {
                self.set_state(FiberState::Loading);
                self.spawn_load();
            }
            Action::Unload => {
                self.set_state(FiberState::Unloading);
                self.spawn_unload();
            }
        }
    }

    /// 状态迁移 + 副作用：internal/status 事件、Active 边界的依赖广播
    /// （对齐 fiber.ts:592-607 的 _updateState）。
    pub(crate) fn set_state(self: &Arc<Self>, new: FiberState) {
        let old = self.state_tx.send_replace(new);
        if old == new {
            return;
        }
        if let Some(ctx) = &self.ctx {
            let spawner = ctx.root.runtime.clone();
            let runtime = ctx.root.runtime.clone();
            let root = ctx.root.clone();
            let name = self.name.clone();
            spawner.spawn(async move {
                root.events.emit(
                    &runtime,
                    keys::STATUS,
                    &[
                        Value::String(name),
                        Value::String(old.as_str().into()),
                        Value::String(new.as_str().into()),
                    ],
                );
            });
        }
        // 仅 ACTIVE ↔ 非 ACTIVE 边界迁移需要广播本 fiber 提供的服务
        if (old == FiberState::Active) != (new == FiberState::Active) {
            let has_provisions =
                self.lock_core(|core| core.provisions.iter().any(|entry| entry.strong_count() > 0));
            if has_provisions {
                if let Some(ctx) = &self.ctx {
                    notify_services(&ctx.root);
                }
            }
        }
    }

    pub(crate) fn spawn_load(self: &Arc<Self>) {
        if let Some(ctx) = &self.ctx {
            let this = self.clone();
            ctx.root.runtime.spawn(async move { this.run_load().await });
        }
    }

    pub(crate) fn spawn_unload(self: &Arc<Self>) {
        if let Some(ctx) = &self.ctx {
            let this = self.clone();
            ctx.root.runtime.spawn(async move { this.run_unload().await });
        }
    }

    /// 装载任务（对齐 fiber.ts:681-701 `_reload`）。
    ///
    /// §2.1.2 强句柄纪律：`ctx` 强引用限定在 `prepared` 块内——setup future
    /// 落定（块结束）后，内核对 ContextInner 的临时强引用全部释放，
    /// 只剩 fiber 自身的 `FiberShared.ctx` 一份（lifecycle 强计数断言依赖此点）。
    async fn run_load(self: Arc<Self>) {
        enum Prepared {
            /// 装载成功且未被超越（携带装载目标 epoch）。
            Settled(Epoch),
            /// 快照时依赖丢失：回退卸载（不置 FAILED，属时序抖动）。
            Miss,
            /// 装载失败（配置校验/setup Err）：清本次注册后 FAILED。
            Failed(CordisError, Vec<Weak<ServiceEntry>>),
        }
        let prepared = {
            // task-29④ 预防注记：ctx 构造后恒 Some（fiber.rs new_root/new_plugin
            // 两构造点均 Some，全内核无置 None 点——构造不变量），此分支为防御性
            // 保留。若未来 ctx 语义改为可清除（如 dispose 释放），此处**必须先
            // 收尾 busy 再 return**——drive 已置 busy=true，裸 return 会让 busy
            // 永久卡 true，fiber 成幽灵 Loading（装卸全被 busy 挡死）。
            let Some(ctx) = self.ctx.clone() else { return };
            let my_target = self.lock_core(|core| core.epoch.clone());
            // ① 依赖快照并登记 dependents（对齐 fiber.ts:681-686）
            enum Prep {
                Plan { plugin: Arc<dyn Plugin>, raw: ConfigValue, committed: Vec<Weak<ServiceEntry>> },
                Miss,
            }
            let prep = {
                #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                let mut core = self.core.lock().expect("fiber 决策锁");
                let Some(plugin) = core.plugin.clone() else { return };
                // ── 第一段：验齐零登记（task-28 两段式·泄漏主竞态窗根除）──
                // 旧实现边验边 retain：注入 [A,B] 且 A 在 B 缺→break Miss→局部
                // committed 丢弃而 A.dependents 的登记残留→提供者卸载时
                // dependents 永不清空=dispose 永久挂死（task-61 根因链）。
                // 验齐阶段只读快照零副作用，Miss 即纯丢弃。
                let mut snapshot: Vec<Arc<ServiceEntry>> = Vec::new();
                for key in plugin.inject() {
                    let scope = ctx.resolve_scope(&key);
                    #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                    let entry = ctx.root.services.read().expect("服务表锁").get_strict(scope);
                    match entry {
                        Some(entry) => snapshot.push(entry),
                        // 快照瞬间依赖丢失：回退为卸载（不置 FAILED，属时序抖动）
                        None => {
                            core.epoch = Epoch::Inactive;
                            break;
                        }
                    }
                }
                if core.epoch == Epoch::Inactive {
                    Prep::Miss
                } else {
                    // ── 第二段：同锁窗统一 retain+commit（云枢完备性要件）──
                    // phase-2 登记循环中途 vanish（快照后提供者卸载摘表/换代）
                    // 触发回滚分支：partial retain 逐个 release 后收敛 Miss——
                    // 不留半登记状态（完备性底线：纯两段式不覆盖 phase-2 内部
                    // vanish；未来本循环任何错误面自动被回滚分支覆盖）。
                    let mut committed: Vec<Weak<ServiceEntry>> = Vec::new();
                    let mut vanished = false;
                    for entry in &snapshot {
                        // 复查：条目仍在表且 uid 未换代（vanish=快照后提供者
                        // 卸载摘表——get_strict 只认 Active，卸载中即 None）
                        let still_active = {
                            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                            let table = ctx.root.services.read().expect("服务表锁");
                            table.get_strict(entry.scope).is_some_and(|cur| cur.uid == entry.uid)
                        };
                        if !still_active {
                            vanished = true;
                            break;
                        }
                        retain_as_dependent(entry, self.id);
                        committed.push(Arc::downgrade(entry));
                    }
                    if vanished {
                        // 回滚分支：partial retain 逐个 release（同步形态——
                        // 仅锁内登记摘除+空则 notify_one，无 await 面）
                        crate::service::release_committed_deps_sync(self.id, std::mem::take(&mut committed));
                        core.epoch = Epoch::Inactive;
                        Prep::Miss
                    } else {
                        // 快照入 core.committed（对齐 _reload 开头的 this.committed = {...store}）；
                        // 卸载时据此释放对各依赖实现的占用并唤醒提供者
                        core.committed = committed.to_vec();
                        Prep::Plan { plugin, raw: core.config.clone(), committed }
                    }
                }
            };
            match prep {
                Prep::Miss => Prepared::Miss,
                Prep::Plan { plugin, raw, committed } => {
                    // ② 配置解析：intercept 链合并（祖先在前）+ schema 校验（方案 §2.2）
                    match resolve_plugin_config(plugin.as_ref(), &ctx, &raw) {
                        Err(err) => Prepared::Failed(err, committed),
                        Ok(resolved) => {
                            // ③ 执行 setup——组件持有 Context 强句柄的唯一合法窗口（§2.1.2）。
                            // S-ISO-2 panic 边界（设计 §3.2.4，凶点一）：setup future 在独立
                            // 任务中执行，Poll 期 panic 被任务边界截获为 JoinError——内联
                            // await 会炸穿装卸任务本身，prepared 收尾（finish_load_error）
                            // 永不执行=fiber 幽灵 Loading（busy 恒 true）。spawn 后三分支
                            // 归一：Ok(Ok)=装载 / Ok(Err)=普通失败 / Err(join)=panic 归一
                            // PluginPanicked——后两者同走 Prepared::Failed 收敛 Failed。
                            // 强句柄不变量（§2.1.2）：JoinHandle 在本块内 await 到底，
                            // 任务持有的 setup_ctx 强引用随任务结束、先于本块退出释放，
                            // 强计数轮廓与内联形态一致（lifecycle 既有断言回归锁）。
                            let setup_ctx = Context { inner: ctx.clone() };
                            let what = format!("插件 <{}> setup", plugin.name());
                            let joined =
                                tokio::spawn(Arc::clone(&plugin).setup(setup_ctx, resolved)).await;
                            match joined {
                                Ok(Ok(_)) => Prepared::Settled(my_target),
                                Ok(Err(err)) => Prepared::Failed(err, committed),
                                Err(join_err) => Prepared::Failed(
                                    crate::panic::join_error(&what, join_err),
                                    committed,
                                ),
                            }
                        }
                    }
                }
            }
            // ← ctx 强引用在此随块结束释放（先于 set_state(Active)）
        };
        match prepared {
            Prepared::Settled(my_target) => {
                let settled = self.lock_core(|core| {
                    if core.uid.is_none() {
                        false // 装载期间被 dispose
                    } else if core.epoch == my_target {
                        core.busy = false;
                        true
                    } else {
                        false // 被 restart/update 超越
                    }
                });
                if settled {
                    self.set_state(FiberState::Active);
                } else {
                    self.set_state(FiberState::Unloading);
                    self.spawn_unload();
                }
            }
            Prepared::Miss => {
                self.set_state(FiberState::Unloading);
                self.spawn_unload();
            }
            Prepared::Failed(err, committed) => self.finish_load_error(err, committed).await,
        }
    }

    /// 装载失败收尾：清理本次注册（依赖登记 + effect 逆序），置 FAILED。
    ///
    /// 与 TS 的差异：TS 把失败 fiber 的已注册 effect 留到 dispose 才清（fiber.ts:693-701
    /// 只记 error）；Rust 版立即清理——否则 FAILED fiber 的 dependents 残留会把提供者
    /// 的卸载卡死（引用计数模型下没有「留脏但不可见」的安全位）。
    async fn finish_load_error(self: &Arc<Self>, err: CordisError, committed: Vec<Weak<ServiceEntry>>) {
        release_committed_deps(self.id, committed).await;
        let disposers = self.lock_core(|core| {
            // 失败收尾同样清 provisions 死亡弱引用（内-P2，与 run_unload ③ 同口径）
            core.provisions.retain(|entry| entry.strong_count() > 0);
            core.effects.drain(..).map(|(_, entry)| entry.disposer).collect::<Vec<_>>()
        });
        // panic 隔离同 run_unload ②：单个失败不阻断其余清理
        for disposer in disposers.into_iter().rev().flatten() {
            let joined = tokio::spawn(disposer.dispose()).await;
            if let Err(join_err) = joined {
                eprintln!(
                    "cordis/警告: fiber <{}> 装载失败的清理器执行异常（已继续）: {join_err}",
                    self.name
                );
            }
        }
        // 收尾按 uid 分支（云枢复核新发现，task-29 追加刀）：dispose 已于
        // Loading 期先行（uid=None、busy=true 不 spawn unload）时，本收尾是该
        // fiber 的**最终任务**，必须落 Disposed 放行 wait_disposed——落 Failed
        // 会让 drive 被 error 挡死、无任何任务再驱动=dispose 永挂（task-61
        // 同族第三候选）。error 跳过存：fiber 已死，诊断无消费面。
        let disposing = self.lock_core(|core| {
            let disposing = core.uid.is_none();
            if disposing {
                core.epoch = Epoch::Inactive;
                core.busy = false;
            } else {
                core.error = Some(Arc::new(err));
                core.epoch = Epoch::Inactive;
                core.busy = false;
            }
            disposing
        });
        if disposing {
            self.set_state(FiberState::Disposed);
        } else {
            self.set_state(FiberState::Failed);
        }
    }

    /// 卸载任务（对齐 fiber.ts:712-746 `_unload`）。
    async fn run_unload(self: Arc<Self>) {
        // ① 先消费者后提供者：等待本 fiber 提供的服务的 dependents 全部卸载完
        loop {
            let waiter = self.lock_core(|core| {
                #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                core.provisions
                    .iter()
                    .filter_map(Weak::upgrade)
                    .find(|entry| !entry.dependents.lock().expect("dependents 锁").is_empty())
            });
            match waiter {
                Some(entry) => entry.release.notified().await,
                None => break,
            }
        }
        // ② 逆序执行本 fiber 的 disposables（fiber.ts:720-730）。
        // panic 隔离：单个 disposer panic 不阻断后续清理（tokio 任务边界捕获）。
        let disposers = self.lock_core(|core| {
            core.effects.drain(..).map(|(_, entry)| entry.disposer).collect::<Vec<_>>()
        });
        for disposer in disposers.into_iter().rev().flatten() {
            let joined = tokio::spawn(disposer.dispose()).await;
            if let Err(join_err) = joined {
                eprintln!(
                    "cordis/警告: fiber <{}> 的清理器执行异常（已继续其余清理）: {join_err}",
                    self.name
                );
            }
        }
        // ③ 释放对各依赖实现的占用，唤醒等待的提供者（fiber.ts:732-736）；
        //    顺带清掉 provisions 中已被提供者卸载回收的死亡弱引用（内-P2）：
        //    restart/失败循环每轮重跑 setup 会重复 push，不清则无界累积、
        //    ①的扫描逐轮多付 upgrade 失败成本
        let committed = self.lock_core(|core| {
            core.provisions.retain(|entry| entry.strong_count() > 0);
            std::mem::take(&mut core.committed)
        });
        release_committed_deps(self.id, committed).await;
        // ④ 收尾：按当前 epoch 决定稳定或继续装载（对齐 _unload 尾）
        enum Follow {
            Settle(FiberState),
            Reload,
        }
        let follow = self.lock_core(|core| {
            if core.uid.is_none() {
                core.busy = false;
                Follow::Settle(FiberState::Disposed)
            } else if core.epoch == Epoch::Inactive {
                core.busy = false;
                Follow::Settle(FiberState::Pending)
            } else {
                Follow::Reload // 依赖回归/重启 → 继续装载
            }
        });
        match follow {
            Follow::Settle(state) => self.set_state(state),
            Follow::Reload => {
                self.set_state(FiberState::Loading);
                self.spawn_load();
            }
        }
    }

}
