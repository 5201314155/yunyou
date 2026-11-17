//! Fiber：插件实例的生命周期状态机与共享结构（对标 fiber.ts）。
//!
//! 状态机：`Pending → Loading → Active`；失败进 `Failed`；卸载走
//! `Unloading → Disposed`（对齐 fiber.ts:147-154）。`Loading`/`Unloading` 是
//! 装卸任务进行中的过渡态，稳定态只有 Pending/Active/Failed/Disposed。
//!
//! epoch（方案 §2.1.4）：`Revision + 每个已就绪依赖的 provision uid 序列`；
//! 依赖注册/注销 → [`crate::registry::notify_services`] 扫描全部 fiber 重算 epoch
//! → 触发装载/卸载（对齐 fiber.ts:617-646、reflect.ts:335-354）。
//!
//! 锁纪律：每 fiber 一把决策锁（`FiberCore`），锁序（以当前代码实序为准，
//! 2026-11-17 内核席复核）`core → isolate → default_scopes → services`，
//! `dependents` 为叶子锁——持 core 后经 `resolve_scope` 先取 isolate、后
//! default_scopes，再取服务表读写锁（refresh_locked/run_load 快照路径与
//! provide 注册体路径同序）；任何 await 前必须放锁；fiber 状态的读取一律走
//! `watch`（无锁），避免交叉锁死。

use crate::config::ConfigValue;
use crate::context::ContextInner;
use crate::error::{CordisError, ErrorKind, Result};
use crate::registry::Plugin;
use crate::service::ServiceEntry;
pub use crate::effect::{Disposer, EffectHandle};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// fiber 生命周期状态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FiberState {
    /// 等待必需依赖就绪。
    Pending,
    /// 装载任务执行中（setup future 运行）。
    Loading,
    /// 装载完成，正常服务中。
    Active,
    /// 装载或校验失败；只能经 `restart` 复位（对齐 fiber.ts:634）。
    Failed,
    /// 卸载任务执行中（等待消费者、逆序清理）。
    Unloading,
    /// 已处置，不可复活。
    Disposed,
}

impl FiberState {
    /// 事件/日志用的状态名。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Loading => "loading",
            Self::Active => "active",
            Self::Failed => "failed",
            Self::Unloading => "unloading",
            Self::Disposed => "disposed",
        }
    }
}

/// 装载依据：Revision + 就绪依赖的 provision uid 序列（对齐 fiber.ts:617-628 的 epoch）。
///
/// 与 TS 的差异：结构化枚举替代字符串拼接，比较语义一致（值相等 ⇔ 装载目标一致）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Epoch {
    /// 依赖不全或已卸载。
    Inactive,
    /// 依赖齐全：含重启代数与各就绪依赖的 provision uid。
    Active { revision: u64, dep_uids: Vec<u64> },
}

/// fiber 的可变决策字段（受 `FiberShared.core` 单锁保护）。
pub(crate) struct FiberCore {
    /// `None` = 已处置（uid 对标 fiber.ts 的 uid）。
    pub uid: Option<u64>,
    /// 重启代数（restart 时 +1，进入 epoch）。
    pub revision: u64,
    pub epoch: Epoch,
    /// 首个装载错误（await_ready 时抛出，对齐 fiber.ts:754-760）。
    pub error: Option<Arc<CordisError>>,
    /// 插件本体（重启需重跑 setup；root fiber 为 `None`）。
    pub plugin: Option<Arc<dyn Plugin>>,
    /// 原始配置（未合并 intercept、未校验）。
    pub config: ConfigValue,
    /// 活跃 effect（注册序；卸载时逆序清理，方案 §2.1.4「注册即 effect」）。
    pub effects: Vec<(u64, crate::effect::EffectEntry)>,
    pub next_effect: u64,
    /// 本次装载提交的依赖快照（弱引用，方案 §2.1.2）。
    pub committed: Vec<Weak<ServiceEntry>>,
    /// 本 fiber provide 出的服务条目（卸载时等待其消费者清空）。
    ///
    /// 死亡弱引用（提供者条目已随卸载摘表回收）在 run_unload ③ 与
    /// finish_load_error 收尾处 `retain` 清除——不在此二点之外的路径累积；
    /// restart 每轮重跑 setup 重新 push，同 scope 旧条目随旧表项回收而死亡、
    /// 由上述收尾点统一清扫（内-P2：防无界累积）。
    pub provisions: Vec<Weak<ServiceEntry>>,
    /// 装卸任务是否进行中（inertia；对齐 fiber 的 inertia 字段）。
    pub busy: bool,
}

/// fiber 共享结构（内核与句柄共用）。
pub(crate) struct FiberShared {
    pub id: u64,
    pub name: String,
    /// 所属上下文（强引用由内核 fiber 持有；组件侧只许 WeakContext，方案 §2.1.2）。
    pub ctx: Option<Arc<ContextInner>>,
    pub core: Mutex<FiberCore>,
    pub state_tx: watch::Sender<FiberState>,
    /// 取消传播令牌（方案 §2.3 第 3 条）：卸载/处置时触发，组件可监听。
    pub cancel: CancellationToken,
}

impl FiberShared {
    /// 构造 root fiber：无插件、恒 Active（对标 fiber.ts:316-334 root 分支）。
    pub(crate) fn new_root(ctx: Arc<ContextInner>) -> Arc<Self> {
        let (state_tx, _) = watch::channel(FiberState::Active);
        let core = FiberCore {
            uid: Some(0),
            revision: 0,
            epoch: Epoch::Active { revision: 0, dep_uids: Vec::new() },
            error: None,
            plugin: None,
            config: serde_json::Value::Null,
            effects: Vec::new(),
            next_effect: 0,
            committed: Vec::new(),
            provisions: Vec::new(),
            busy: false,
        };
        Arc::new(Self {
            id: 0,
            name: "root".into(),
            ctx: Some(ctx),
            core: Mutex::new(core),
            state_tx,
            cancel: CancellationToken::new(),
        })
    }

    /// 构造插件 fiber（对标 registry.ts:316 的 Runtime+Fiber 创建）。
    pub(crate) fn new_plugin(
        ctx: Arc<ContextInner>,
        plugin: Arc<dyn Plugin>,
        config: ConfigValue,
    ) -> Arc<Self> {
        let id = ctx.root.next_fiber.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let (state_tx, _) = watch::channel(FiberState::Pending);
        let core = FiberCore {
            uid: Some(0),
            revision: 0,
            epoch: Epoch::Inactive,
            error: None,
            plugin: Some(plugin.clone()),
            config,
            effects: Vec::new(),
            next_effect: 0,
            committed: Vec::new(),
            provisions: Vec::new(),
            busy: false,
        };
        Arc::new(Self {
            id,
            name: plugin.name().to_string(),
            ctx: Some(ctx),
            core: Mutex::new(core),
            state_tx,
            cancel: CancellationToken::new(),
        })
    }

    /// 在决策锁内执行一段操作（effect 注册体与 provide 登记需要原子性）。
    pub(crate) fn lock_core<R>(&self, f: impl FnOnce(&mut FiberCore) -> R) -> R {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let mut core = self.core.lock().expect("fiber 决策锁");
        f(&mut core)
    }

    /// 读取当前状态（走 watch，无决策锁）。
    pub fn state(&self) -> FiberState {
        *self.state_tx.borrow()
    }

    /// 取消传播令牌的克隆（卸载/处置时触发；与 [`FiberHandle::cancellation`] 同源）。
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

/// fiber 的对外句柄（对标 registry.ts:331-335 的 PromiseLike Fiber）。
#[derive(Clone)]
pub struct FiberHandle(pub(crate) Arc<FiberShared>);

impl FiberHandle {
    pub(crate) fn from_shared(shared: Arc<FiberShared>) -> Self {
        Self(shared)
    }

    /// fiber id。
    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// 插件显示名。
    pub fn name(&self) -> String {
        self.0.name.clone()
    }

    /// 当前状态。
    pub fn state(&self) -> FiberState {
        self.0.state()
    }

    /// 是否 Active。
    pub fn is_active(&self) -> bool {
        self.0.state() == FiberState::Active
    }

    /// 取消传播令牌：组件的长任务应监听并在触发时静止（方案 §2.3）。
    pub fn cancellation(&self) -> CancellationToken {
        self.0.cancel.clone()
    }

    /// 等待 fiber 落到稳定态并装载完成；`Failed` 时抛出装载错误（对标 fiber.await()）。
    ///
    /// `Pending`（依赖未就绪）是过渡态：挂起等待依赖就绪驱动装载，直到
    /// `Active`/`Failed`/`Disposed` 才返回——与 [`crate::registry::Context::plugin`]
    /// 文档的「等待装载完成」语义一致。`Disposed` 返回 `Ok`（处置不是失败）。
    pub async fn await_ready(&self) -> Result<()> {
        let mut rx = self.0.state_tx.subscribe();
        loop {
            let state = *rx.borrow_and_update();
            match state {
                FiberState::Pending | FiberState::Loading | FiberState::Unloading => {
                    rx.changed()
                        .await
                        .map_err(|_| CordisError::new(ErrorKind::Internal, "fiber 状态通道已关闭"))?;
                }
                FiberState::Failed => {
                    #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
                    let stored = self
                        .0
                        .core
                        .lock()
                        .expect("fiber 决策锁")
                        .error
                        .clone();
                    let err: CordisError = stored
                        .map(|boxed| (*boxed).clone())
                        .unwrap_or_else(|| {
                            CordisError::new(ErrorKind::FiberFailed, "fiber 处于失败态")
                        });
                    return Err(err);
                }
                _ => return Ok(()),
            }
        }
    }

    /// 等待 fiber 进入 Disposed（dispose 的收尾同步点）。
    pub async fn wait_disposed(&self) {
        let mut rx = self.0.state_tx.subscribe();
        loop {
            if *rx.borrow_and_update() == FiberState::Disposed {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

impl std::fmt::Debug for FiberHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FiberHandle <{} #{} {:?}>", self.0.name, self.0.id, self.state())
    }
}
