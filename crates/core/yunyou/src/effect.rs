//! effect：一切注册皆可撤销（对标 fiber.ts:69-120，方案 §2.1.4）。
//!
//! `effect(execute)` 立即执行注册体并收集 disposer；fiber 卸载时**逆序**清理。
//! 显式撤销走 [`EffectHandle::dispose`]（幂等，与卸载清理互斥摘除）。

use crate::error::{CordisError, ErrorKind, Result};
use crate::fiber::{FiberHandle, FiberShared};
use crate::service::ServiceEntry;
use yunyou_util::BoxFut;
use std::sync::{Arc, Weak};

/// 可撤销副作用的清理器（异步）。fiber 卸载时逆序执行。
pub struct Disposer(Box<dyn FnOnce() -> BoxFut<'static, ()> + Send>);

impl Disposer {
    /// 用「返回清理 future 的工厂」构造。
    pub fn new<F>(factory: F) -> Self
    where
        F: FnOnce() -> BoxFut<'static, ()> + Send + 'static,
    {
        Self(Box::new(factory))
    }

    /// 执行清理。
    pub async fn dispose(self) {
        (self.0)().await;
    }
}

impl std::fmt::Debug for Disposer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Disposer")
    }
}

/// effect 表条目（注册序保存，卸载逆序消费）。
pub(crate) struct EffectEntry {
    #[allow(dead_code)]
    pub label: String,
    pub disposer: Option<Disposer>,
}

/// effect 注册体的产出：清理器 + 可选的同窗口 provision 登记（provide 专用）。
pub(crate) struct EffectOutcome {
    pub disposer: Option<Disposer>,
    pub provision: Option<Weak<ServiceEntry>>,
}

impl EffectOutcome {
    /// 通用形态：只有清理器。
    pub fn plain(disposer: Option<Disposer>) -> Self {
        Self { disposer, provision: None }
    }
}

/// effect 句柄：显式撤销（对标 TS 手持 disposer 调用）。
#[derive(Clone)]
pub struct EffectHandle {
    fiber: Weak<FiberShared>,
    effect_id: u64,
}

impl EffectHandle {
    pub(crate) fn new(fiber: &Arc<FiberShared>, effect_id: u64) -> Self {
        Self { fiber: Arc::downgrade(fiber), effect_id }
    }

    /// 立即撤销：从 fiber 摘除并执行清理。幂等；已卸载的 fiber 上为 no-op。
    pub async fn dispose(&self) {
        let Some(fiber) = self.fiber.upgrade() else { return };
        let disposer = fiber.lock_core(|core| {
            if let Some(index) = core.effects.iter().position(|(id, _)| *id == self.effect_id) {
                core.effects.remove(index).1.disposer
            } else {
                None
            }
        });
        if let Some(disposer) = disposer {
            disposer.dispose().await;
        }
    }
}

impl std::fmt::Debug for EffectHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EffectHandle({})", self.effect_id)
    }
}

impl FiberShared {
    /// 注册 effect（带同窗口 provision 登记，provide 专用路径）。
    ///
    /// 注册体在本 fiber 决策锁内执行（保证「插表 + provisions 登记 + effect 登记」
    /// 原子可见）；锁序约定（以代码实序为准）`core → isolate → default_scopes → services`，注册体内不得 await，
    /// 也**不得再次 lock_core**（std Mutex 非重入）。
    ///
    /// # panic 边界（S-ISO-2，设计 §3.2.3/凶点二）
    /// 注册体是插件代码进决策锁的唯一公开入口——panic 经 [`crate::panic::catch_panic`]
    /// 在锁内截获归一 `Err(PluginPanicked)`，**不穿 guard=决策锁不中毒**；调用方
    /// （setup 的 `?` 链）拿到 Err 后走 finish_load_error → Failed 转移。
    pub(crate) fn effect_with(
        self: &Arc<Self>,
        label: impl Into<String>,
        execute: impl FnOnce() -> Result<EffectOutcome>,
    ) -> Result<EffectHandle> {
        let label = label.into();
        let id = self.lock_core(|core| {
            if core.uid.is_none() {
                return Err(CordisError::new(
                    ErrorKind::EffectInactive,
                    format!("fiber <{}> 已处置，拒绝注册 effect", self.name),
                ));
            }
            let outcome = crate::panic::catch_panic(
                &format!("fiber <{}> effect「{label}」注册体", self.name),
                execute,
            )?;
            if let Some(provision) = outcome.provision {
                core.provisions.push(provision);
            }
            let id = core.next_effect;
            core.next_effect += 1;
            core.effects.push((id, EffectEntry { label, disposer: outcome.disposer }));
            Ok(id)
        })?;
        Ok(EffectHandle::new(self, id))
    }

    /// 注册 effect（通用形态：无 provision 登记，见 [`FiberShared::effect_with`] 契约）。
    pub(crate) fn effect(
        self: &Arc<Self>,
        label: impl Into<String>,
        execute: impl FnOnce() -> Result<Option<Disposer>>,
    ) -> Result<EffectHandle> {
        self.effect_with(label, || Ok(EffectOutcome::plain(execute()?)))
    }
}

impl FiberHandle {
    /// 注册 effect（见 [`FiberShared::effect`] 的契约说明）。
    pub fn effect(
        &self,
        label: impl Into<String>,
        execute: impl FnOnce() -> Result<Option<Disposer>>,
    ) -> Result<EffectHandle> {
        self.0.effect(label, execute)
    }
}
