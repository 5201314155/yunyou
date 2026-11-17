//! Context：子上下文持久链与内核共享状态（对标 context.ts）。
//!
//! - Context 树是**不可变持久结构**：`extend`/`isolate`/`intercept` 都返回新节点，
//!   子节点持父节点的 `Arc`，父节点永不改写（对齐 TS 原型链 `Object.create(父)`）；
//! - `RootState` 是全内核共享态：唯一的服务表锁、事件表、fiber 表、uid/scope 计数器
//!   与 tokio 运行时句柄；
//! - 组件长期持有 Context 只允许 [`WeakContext`]；强句柄（`Context` 值）仅限
//!   `Plugin::setup` 执行期（方案 §2.1.2，评审线一打回项）。

use crate::error::Result;
use crate::events::EventTable;
use crate::fiber::{EffectHandle, FiberShared};
use crate::service::{ScopeId, ServiceId, ServiceTable};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64};
use std::sync::{Arc, Mutex, RwLock, Weak};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

/// 子上下文元信息（对标 `extend(meta)` 的自有属性）。
#[derive(Clone, Debug, Default)]
pub struct ContextMeta {
    /// 显示名（fiber 名、internal/status 事件与日志用）。
    pub name: Option<String>,
}

/// 全内核共享态（root 持有）。
pub(crate) struct RootState {
    /// 全内核唯一的服务表锁（纪律：方案 §2.3）。
    pub services: RwLock<ServiceTable>,
    /// 各服务的默认作用域（首次使用时分配）。
    pub default_scopes: Mutex<HashMap<ServiceId, ScopeId>>,
    /// 事件总线（监听表）。
    pub events: EventTable,
    /// MessageHub（契约 message.v1）挂载（与 events 同位：全内核一份，hub.rs 定义）。
    /// 缝由内核供：团队插件禁用不得杀缝（三补③原文，融合终版 §3 表 f 定案）。
    pub hub: Arc<crate::hub::InProcessHub>,
    /// 全部存活 fiber 的弱引用（服务表变更时扫描重算 epoch）。
    pub fibers: Mutex<Vec<Weak<FiberShared>>>,
    /// provision uid 计数器（epoch 成分）。
    pub next_uid: AtomicU64,
    /// fiber id 计数器。
    pub next_fiber: AtomicU64,
    /// 作用域 id 计数器。
    pub next_scope: AtomicU64,
    /// 内核运行时句柄：装卸任务在此 spawn（要求 Context::new 在 tokio runtime 内调用）。
    pub runtime: Handle,
    /// root fiber 的强引用锚点：root fiber 是全树注册（provide/on/effect）的归属，
    /// 必须与进程同生命周期，否则 root 上的注册一律报 EffectInactive。
    pub root_fiber: Mutex<Option<Arc<FiberShared>>>,
}

/// 上下文节点内部结构。
pub(crate) struct ContextInner {
    pub root: Arc<RootState>,
    pub parent: Option<Arc<ContextInner>>,
    /// 本节点覆盖的 isolate 表（子覆盖父，沿链解析）。
    pub isolate: Mutex<HashMap<ServiceId, ScopeId>>,
    /// 本节点覆盖的 intercept 表。
    pub intercept: Mutex<HashMap<ServiceId, Value>>,
    /// 本上下文归属的 fiber（root 与插件子上下文各有归属）。
    pub fiber: Mutex<Option<Weak<FiberShared>>>,
    pub meta: ContextMeta,
}

/// 组件内核上下文（Clone 廉价：共享内部节点）。
#[derive(Clone)]
pub struct Context {
    pub(crate) inner: Arc<ContextInner>,
}

impl Context {
    /// 建根上下文。
    ///
    /// # Panics
    /// 必须在 tokio runtime 内调用（装卸任务要用 `Handle::current`；方案 §2.3）。
    pub fn new() -> Context {
        // MessageHub 状态先于 RootState 构造（root 弱引用在 Arc 成形后回接，防自引）。
        let hub = Arc::new(crate::hub::InProcessHub::new());
        let root_state = Arc::new(RootState {
            services: RwLock::new(ServiceTable::default()),
            default_scopes: Mutex::new(HashMap::new()),
            events: EventTable::default(),
            hub: hub.clone(),
            fibers: Mutex::new(Vec::new()),
            next_uid: AtomicU64::new(1),
            next_fiber: AtomicU64::new(1),
            next_scope: AtomicU64::new(1),
            runtime: Handle::current(),
            root_fiber: Mutex::new(None),
        });
        let root_inner = Arc::new(ContextInner {
            root: root_state.clone(),
            parent: None,
            isolate: Mutex::new(HashMap::new()),
            intercept: Mutex::new(HashMap::new()),
            fiber: Mutex::new(None),
            meta: ContextMeta { name: Some("root".into()) },
        });
        // root fiber：无插件、恒 Active（对标 fiber.ts:316-334 的 root 分支），
        // 承载 root 上的注册（如 boot 流程的 provide），dispose 即整树优雅关闭。
        let root_fiber = FiberShared::new_root(root_inner.clone());
        // S-ISO-1 逐点豁免（属性不落表达式语句，let 重构+作用域块即时放锁，
        // 防临时 guard 生命周期延到块尾与后续 root_inner/root_state 移动冲突）：
        {
            #[allow(clippy::expect_used)] // 中毒 tripwire（A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            let slot = &mut *root_inner.fiber.lock().expect("fiber 槽锁");
            *slot = Some(Arc::downgrade(&root_fiber));
        }
        #[allow(clippy::expect_used)] // 中毒 tripwire（A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        root_inner.root.fibers.lock().expect("fiber 表锁").push(Arc::downgrade(&root_fiber));
        // 强引用锚点：root fiber 必须活到 Context 本身消亡
        {
            #[allow(clippy::expect_used)] // 中毒 tripwire（A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            let slot = &mut *root_state.root_fiber.lock().expect("root fiber 槽锁");
            *slot = Some(root_fiber);
        }
        // 观测通道接通（弱引用，Arc 成形后回接）+ root 装配 message.v1 缝：
        // hub 由内核供、独立于任何组件卡存活——团队插件禁用不得杀缝（融合终版 §3 表 f）。
        hub.attach_root(&root_state);
        // S-ISO-1 B 不变量断言（K-14 B 类口径）：boot 装配不变量——全新 root
        // 上 mount_root 仅在服务表已含同键时失败，构造路径唯一无竞争。
        #[allow(clippy::expect_used)]
        crate::hub::mount_root(&Context { inner: root_inner.clone() })
            .expect("内核 root 装配 MessageHub 服务");
        Context { inner: root_inner }
    }

    /// 派生子上下文（对标 context.ts:99 `extend`）：父节点不被改写。
    pub fn extend(&self, meta: ContextMeta) -> Context {
        Context {
            inner: Arc::new(ContextInner {
                root: self.inner.root.clone(),
                parent: Some(self.inner.clone()),
                isolate: Mutex::new(HashMap::new()),
                intercept: Mutex::new(HashMap::new()),
                // S-ISO-1 逐点豁免（属性不落表达式，提行 let 化）
                fiber: Mutex::new({
                    #[allow(clippy::expect_used)] // 中毒 tripwire（A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                    let parent_fiber = self.inner.fiber.lock().expect("fiber 槽锁").clone();
                    parent_fiber
                }),
                meta,
            }),
        }
    }

    /// 为服务 `key` 换作用域（对标 context.ts:121 `isolate`）。
    /// 相同 `ScopeId` 的多次 isolate 即合并作用域（同标签 = 同作用域）。
    pub fn isolate(&self, key: ServiceId, scope: ScopeId) -> Context {
        let child = self.extend(ContextMeta::default());
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        child.inner.isolate.lock().expect("isolate 锁").insert(key, scope);
        child
    }

    /// 追加拦截配置（对标 context.ts:139 `intercept`）；合并时祖先段在前、本段在后。
    pub fn intercept(&self, key: ServiceId, config: Value) -> Context {
        let child = self.extend(ContextMeta::default());
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        child.inner.intercept.lock().expect("intercept 锁").insert(key, config);
        child
    }

    /// 分配新的作用域标签（配合 [`Context::isolate`] 使用）。
    pub fn allocate_scope(&self) -> ScopeId {
        ScopeId::from_counter(&self.inner.root.next_scope)
    }

    /// 显示名：沿链向上找最近的 `meta.name`，root 兜底 `"root"`。
    pub fn name(&self) -> String {
        let mut node = Some(self.inner.as_ref());
        while let Some(current) = node {
            if let Some(name) = &current.meta.name {
                return name.clone();
            }
            node = current.parent.as_deref();
        }
        "root".to_string()
    }

    /// 当前归属的 fiber（root 与插件子上下文均有；强弱见方案 §2.1.2）。
    pub(crate) fn current_fiber(&self) -> Option<Arc<FiberShared>> {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let fiber_guard = self.inner.fiber.lock().expect("fiber 槽锁");
        fiber_guard.as_ref().and_then(Weak::upgrade)
    }

    /// 当前归属 fiber 的取消传播令牌（方案 §2.3 第 3 条 + §5.1 R1）：fiber
    /// 卸载/处置时触发，组件在 setup 内克隆进自有服务后，长任务监听即静止。
    ///
    /// **非 fiber 上下文**（无归属 fiber）返回的是空令牌——永不触发，调用方
    /// 据此兜底即可，无需分支处理。注意空令牌是**每次调用新造**的独立实例
    /// （各次调用互不相同；行为等价：都不可能被触发）。
    pub fn cancellation(&self) -> CancellationToken {
        self.current_fiber()
            .map(|fiber| fiber.cancellation())
            .unwrap_or_default()
    }

    /// 在当前归属 fiber 上注册 effect（对标 ctx.effect 的 mixin）。
    pub fn effect(
        &self,
        label: impl Into<String>,
        execute: impl FnOnce() -> Result<Option<crate::fiber::Disposer>>,
    ) -> Result<EffectHandle> {
        let fiber = self
            .current_fiber()
            .ok_or_else(|| crate::error::CordisError::new(
                crate::error::ErrorKind::EffectInactive,
                "当前上下文没有归属的 fiber，无法注册 effect",
            ))?;
        fiber.effect(label, execute)
    }

    /// 降级为弱句柄：组件长期持有 Context 的唯一合法形态（方案 §2.1.2）。
    pub fn downgrade(&self) -> WeakContext {
        WeakContext { inner: Arc::downgrade(&self.inner) }
    }

    /// 处置当前归属的 fiber（对标 root.dispose 的整树优雅关闭，方案 §2.5）：
    /// root 上调用即整树逆序卸载（先消费者后提供者）；子上下文上调用即卸载该子树。
    pub async fn dispose(&self) -> Result<()> {
        let fiber = self.current_fiber().ok_or_else(|| {
            crate::error::CordisError::new(
                crate::error::ErrorKind::EffectInactive,
                "当前上下文没有归属的 fiber，无法处置",
            )
        })?;
        crate::fiber::FiberHandle::from_shared(fiber).dispose().await
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

/// Context 的弱句柄。setup 落定后组件只许持有它或服务句柄（方案 §2.1.2）。
#[derive(Clone)]
pub struct WeakContext {
    inner: Weak<ContextInner>,
}

impl WeakContext {
    /// 升级为强句柄（内核存活期内组件可放心升级；root 与进程同生命周期）。
    pub fn upgrade(&self) -> Option<Context> {
        self.inner.upgrade().map(|inner| Context { inner })
    }

    /// 观测底层节点的强引用计数（审计与 lifecycle 测试用：断言 setup 落定后
    /// 内核不再持有 setup 传入的强句柄）。
    pub fn strong_count(&self) -> usize {
        self.inner.strong_count()
    }
}
