//! 插件 trait、注册入口与服务表变更广播（对标 registry.ts / reflect.ts:335-354）。
//!
//! 装配动作只有两个（方案 §2.2）：[`Context::plugin`]（启动）与
//! [`Context::effect`]/`provide`/`on`（装载期注册，见 context/effect 模块）。

use crate::config::ConfigValue;
use crate::context::{Context, ContextMeta, RootState};
use crate::error::Result;
use crate::fiber::{FiberHandle, FiberShared};
use crate::manifest::PluginManifest;
use yunyou_util::BoxFut;
use std::sync::{Arc, Weak};

/// 插件 trait：TS 的函数/类/对象三形态合并为一个 trait（方案 §2.1.3 / §2.4 D5）。
///
/// # 契约（评审线一强制，违者打回）
/// 1. **强句柄窗口**：[`Plugin::setup`] 以值收到 `Context` 强句柄——这是组件持有
///    强句柄的唯一合法窗口；setup 落定（fiber Active）后，组件实例与其创建的
///    一切服务不得存留任何 `Arc<Context>` 强引用，只允许 [`crate::context::WeakContext`]
///    或所依赖服务的 `Arc<dyn Trait>`（方案 §2.1.2；lifecycle 测试有强计数断言）。
/// 2. **Send + Sync**：插件与服务实现必须 `Send + Sync`（tokio 多线程，方案 §2.3）；
///    服务实现内不得为绕过借用检查上 `Arc<Mutex>`，可变状态归单 owner。
/// 3. **注册即 effect**：setup 里的一切注册（provide/on/effect）随 fiber 卸载自动撤销。
/// 4. [`Plugin::inject`] 必须返回稳定序列（多次调用结果一致），epoch 依赖它计算。
pub trait Plugin: Send + Sync + 'static {
    /// 插件显示名（fiber 名、internal/status 事件与日志用）。
    fn name(&self) -> &str;

    /// 插件声明面（方案 §2.2）：装载前静态校验（[`crate::manifest::validate_manifest`]）
    /// 与面板/诊断展示的元数据源。
    ///
    /// # 默认实现
    /// 按 [`Plugin::name`] 与本内核 crate 的 `CARGO_PKG_VERSION` 推导最小 manifest
    /// （level=bundled、auto_enable=true、其余空）。**注意**：默认 version 是 yunyou
    /// 内核的版本；组件应覆盖本方法，用自身 `env!("CARGO_PKG_VERSION")` 与真实
    /// 契约名/服务键投影补全（服务键字符串一律经 [`crate::manifest::projection_of`] 生成）。
    fn manifest(&self) -> PluginManifest {
        PluginManifest::minimal(self.name().to_string(), env!("CARGO_PKG_VERSION").to_string())
    }

    /// 依赖的服务键（对标 static inject；缺任一则 fiber 保持 Pending 直到就绪）。
    fn inject(&self) -> Vec<crate::service::ServiceId> {
        Vec::new()
    }

    /// 声明消费的拦截配置段（对标 static intercept；合并时祖先段在前，方案 §2.2）。
    fn intercepts(&self) -> Vec<crate::service::ServiceId> {
        Vec::new()
    }

    /// 配置 schema（JSON Schema；组件侧由 schemars 派生后转 `Value`，方案 §2.2）。
    /// 存在时装载前执行校验，失败即 ValidationError 并中止装载。
    fn config_schema(&self) -> Option<ConfigValue> {
        None
    }

    /// 装载体（对标 apply / constructor）：返回的 future 落定即视为装载完成。
    ///
    /// 失败（Err）→ fiber 进入 Failed，已注册的 effect 立即逆序清理；
    /// 只能经 [`FiberHandle::restart`] 复位（对齐 fiber.ts:634）。
    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue)
        -> BoxFut<'static, Result<FiberOutput>>;
}

/// setup 的产出（一期仅标记完成；服务导出一律走 `ctx.provide`，方案 §2.4 D1）。
#[derive(Debug, Clone, Copy)]
pub struct FiberOutput;

impl Context {
    /// 注册插件并启动其 fiber（对标 registry.ts:316 `ctx.plugin()`）。
    ///
    /// - 立即创建子上下文与 fiber 并开始依赖解析：依赖齐 → 装载；缺 → Pending 等待；
    /// - 子 fiber 的处置挂在父 fiber 的一个 effect 上（fiber.ts:271-303）：
    ///   父卸载连带子卸载（effect 逆序保证后注册的子先卸）。
    /// - 返回的 [`FiberHandle`] 用 `await_ready()` 等待装载完成（对标 await fiber）。
    pub fn plugin<P: Plugin>(&self, plugin: P, config: ConfigValue) -> FiberHandle {
        let plugin: Arc<dyn Plugin> = Arc::new(plugin);
        let name = plugin.name().to_string();
        // ① 子上下文（extend：父不被改写）+ fiber 双向挂钩（子持父 Arc，fiber 挂子 ctx）
        let child = self.extend(ContextMeta { name: Some(name.clone()) });
        let fiber = FiberShared::new_plugin(child.inner.clone(), plugin, config);
        // S-ISO-1 A 锁类 tripwire（逐点豁免）：属性不可落在表达式语句上，故以
        // let 形态持有 guard——锁行仍在 expect 调用点，语义不变。
        #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（std 锁保留裁决）
        let fiber_slot = &mut *child.inner.fiber.lock().expect("fiber 槽锁");
        *fiber_slot = Some(Arc::downgrade(&fiber));
        // ② 登记进全局 fiber 表（服务表变更时扫描重算 epoch）
        #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（std 锁保留裁决）
        let fiber_table = &mut self.inner.root.fibers.lock().expect("fiber 表锁");
        fiber_table.push(Arc::downgrade(&fiber));
        // ③ 父卸载连带子卸载：子 fiber 的处置挂在父 fiber 的 effect 上，
        //    并由该 effect **持有子 fiber 强引用**（注册即挂靠：调用方是否保留
        //    FiberHandle 不影响子 fiber 生命周期，对齐 TS 的父 effect 挂载语义）
        if let Some(parent) = self.current_fiber() {
            let child = fiber.clone();
            // task-29②：EffectInactive（父 fiber 已死）不得静默吞——吞掉=子 fiber
            // 失去「父卸载连带子卸载」挂靠，成生命周期孤儿（父死后子永活无兜底）。
            // 此处不上抛：子 fiber 已过槽位与全局表登记点（半成品），上抛会给
            // 调用方「未建成」假象而 fiber 实际在册——孤儿风险转诊断面暴露，
            // 调用方据此 dispose 回收。
            if let Err(err) = parent.effect(format!("子 fiber <{name}>"), move || {
                Ok(Some(crate::fiber::Disposer::new(move || {
                    let child = child.clone();
                    Box::pin(async move {
                        FiberHandle::from_shared(child).dispose().await.ok();
                    })
                })))
            }) {
                eprintln!(
                    "[yunyou][registry] 子 fiber <{name}> 的父卸载挂靠失败（{err}）：\
                     父 fiber 已处置，该子 fiber 不随父卸载——需显式 dispose 回收"
                );
            }
        }
        // ④ 初始依赖检查并驱动（Pending 或直接装载）
        fiber.drive();
        FiberHandle::from_shared(fiber)
    }
}

/// 服务表变更广播（对标 reflect.ts:335-354 notify）：扫描全部存活 fiber 重算 epoch，
/// epoch 变化者触发装卸。全量扫描语义等价于 TS 的按名过滤（epoch 未变者无操作），
/// 一期取简单正确，规模优化留待出现热点时再做。
pub(crate) fn notify_services(root: &Arc<RootState>) {
    let snapshot: Vec<Arc<FiberShared>> = {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let mut table = root.fibers.lock().expect("fiber 表锁");
        let snapshot = table.iter().filter_map(Weak::upgrade).collect();
        table.retain(|entry| entry.strong_count() > 0); // 顺手清理已消亡的弱引用
        snapshot
    };
    for fiber in snapshot {
        fiber.drive();
    }
}
