//! # yunyou —— yunyou 组件内核
//!
//! 对标 TS 版 Cordis 的 Rust 组件内核：Context / Plugin / Fiber / 事件总线 / 配置注入
//! / volatile 热更底座（事件面含契约⑧「只发所属 fiber」归属分发）。
//! 行为基准：《docs/架构/组件内核架构方案.md》（已评审）；TS 参照 `vendor/cordis/src/`。
//!
//! ## 三条硬纪律（方案 §2.1.2 / §2.3，评审线一打回项）
//! 1. **Send + Sync**：所有插件与服务的实现必须 `Send + Sync`（tokio 多线程调度，
//!    TS 单线程假设不成立）。可变状态归单 owner，服务实现内禁止为绕借用检查上
//!    `Arc<Mutex>`；跨服务共享只读状态用 `Arc` 不可变快照。
//! 2. **Context 强句柄仅限 setup 执行期**：`Plugin::setup` 以值收到强句柄；
//!    setup 落定（fiber Active）后，组件与其创建的服务只许持
//!    [`WeakContext`](context::WeakContext) 或所依赖服务的 `Arc<dyn Trait>`。
//! 3. **注册即 effect**：一切注册（provide / on / effect）皆可撤销，
//!    fiber 卸载时逆序清理（方案 §2.1.4，对标 dsh「Registrations are effects」）。
//!
//! ## 并发模型
//! - 服务表：全内核唯一 `RwLock`（service.rs）；
//! - fiber 决策字段：每 fiber 一把 `Mutex`，锁序（以代码实序为准，2026-11-17 复核）
//!   `core → isolate → default_scopes → services`（dependents 为叶子锁），
//!   任何 await 前放锁；状态读取走 `tokio::sync::watch`（无锁）；
//! - 等待唤醒：`tokio::sync::Notify`（提供者等待消费者卸载）；
//! - 装卸任务 spawn 在建 Context 时捕获的运行时句柄上。
//!
//! ## panic 纪律（《插件崩溃隔离设计》§5，编译期硬门）
//! 内核 panic 边界（工具/装卸/事件监听/waterfall 环）依赖 unwind 捕获；任何方式
//! 把 panic 改成 abort（profile 或 RUSTFLAGS）都使全部隔离机制失效——本断言对
//! 所有 profile 生效，改了必炸构建，不依赖 CI 与人的自觉。

// 云枢裁决形态（2026-11-16）：const assert 比 cfg(panic) compile_error 更防
// RUSTFLAGS=-Cpanic=abort 覆盖；不加新依赖，std 能力足够。

// S-ISO-1 硬门（业主深化五条原文；云枢改义裁决 2026-11-16）：内核主干禁裸
// unwrap/expect——存量清点后仅余两类已申报例外（A 锁类中毒 tripwire+B 不变量
// 断言，逐点 allow+注释模板），新增裸调用即编译失败。tests/ 集成测试是独立
// crate root 不受本 deny 罩（K-14 观察期口径维持）。
#![deny(clippy::unwrap_used, clippy::expect_used)]
const _: () = assert!(
    cfg!(panic = "unwind"),
    "yunyou 内核必须以 unwind 编译：panic 边界（装卸/监听/工具/waterfall）依赖 unwind 捕获，abort 使全部隔离机制失效"
);

pub mod config;
pub mod context;
pub mod effect;
pub mod error;
pub mod events;
pub mod fiber;
pub mod hub;
pub mod load;
pub mod manifest;
pub mod manifest_ui;
pub mod ops;
mod panic; // 内部辅助（catch_panic/join_error 唯一集中点），面不外暴——见模块头
pub mod registry;
pub mod service;
pub mod volatile;

pub use config::{merge_configs, validate_schema, ConfigValue, ValidationEntry, ValidationError};
pub use context::{Context, ContextMeta, WeakContext};
pub use effect::{Disposer, EffectHandle};
pub use error::{CordisError, ErrorKind, Result};
pub use events::{
    keys, EventOwner, EventTable, Listener, ListenerOutcome, WaterfallBuiltin, WaterfallListener,
    WaterfallNext,
};
pub use fiber::{Epoch, FiberHandle, FiberState};
pub use hub::{
    Address, DeliveredMessage, DeliveryStatus, HistoryEntry, HistoryQuery, InProcessHub,
    MemberInfo, MessageHandler, MessageHub, MessageKind, OutgoingMessage, SentReceipt,
    registry::{validate_member_name, MEMBER_NAME_MAX_LEN},
};
pub use manifest::{projection_of, validate_manifest, PluginLevel, PluginManifest, UiEntry};
pub use registry::{FiberOutput, Plugin};
pub use service::{ScopeId, ServiceId};
pub use volatile::Volatile;
