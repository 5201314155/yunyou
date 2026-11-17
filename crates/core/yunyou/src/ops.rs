//! 生命周期操作 API：配置解析、配置更新瀑布、原地重启与处置
//! （对标 fiber.ts 的 update/restart 与 root.dispose，方案 §2.2）。
//!
//! 与 `load.rs` 的分工：本文件是**外部驱动的生命周期操作**（用户/宿主调用），
//! load.rs 是 epoch 变化驱动的**自动装卸引擎**；两者共享 [`FiberShared`] 决策锁。

use crate::config::{merge_configs, validate_schema, ConfigValue};
use crate::context::ContextInner;
use crate::error::{CordisError, ErrorKind, Result};
use crate::events::{keys, WaterfallBuiltin};
use crate::fiber::{Epoch, FiberHandle, FiberShared, FiberState};
use crate::registry::Plugin;
use serde_json::Value;
use std::sync::Arc;

/// 合并 intercept 链并执行 schema 校验（方案 §2.2）。
///
/// 合并顺序：插件自带 config（base）最前 → 各消费服务的 intercept 段
///（祖先在前、自身在后）→ 后者覆盖前者（对齐 service.ts:86-102）。
pub(crate) fn resolve_plugin_config(
    plugin: &dyn Plugin,
    ctx: &ContextInner,
    raw: &ConfigValue,
) -> Result<ConfigValue> {
    let mut segments: Vec<ConfigValue> = vec![raw.clone()];
    for key in plugin.intercepts() {
        segments.extend(ctx.collect_intercept(&key));
    }
    let refs: Vec<&Value> = segments.iter().collect();
    let merged = merge_configs(&refs);
    if let Some(schema) = plugin.config_schema() {
        validate_schema(&schema, &merged)?;
    }
    Ok(merged)
}

impl FiberShared {
    /// 原地重启（对齐 fiber.ts:768-774 restart）：revision+1 → 重算 epoch →
    /// 经卸载路径原地重装；同时是 FAILED fiber 的唯一复位手段。
    async fn restart_inner(self: &Arc<Self>) -> Result<()> {
        // 旧错误 take 快照（task-29③）：复位旧错误是 drive 解除 FAILED 挡门的
        // 必要步骤（load.rs drive 的 error.is_some() 不装载），但「开头清零」会让
        // 装载未写新错误即返回的路径静默吞掉原始诊断——失败时条件恢复，成功时
        // 不恢复（成功=旧错误已过时）。典型未写错误路径=drive 空转早退（FAILED
        // fiber 依赖仍缺：epoch Inactive→Inactive 不变早退、不走装载），Miss
        // 回退本身走 Pending/await Ok 不触发恢复分支。
        let old_error = {
            #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
            let mut core = self.core.lock().expect("fiber 决策锁");
            if core.uid.is_none() {
                return Err(CordisError::new(
                    ErrorKind::FiberDisposed,
                    format!("fiber <{}> 已处置，无法重启", self.name),
                ));
            }
            core.revision += 1;
            core.error.take()
        };
        self.drive();
        FiberHandle::from_shared(self.clone())
            .await_ready()
            .await
            .inspect_err(|_| self.restore_error_if_unwritten(old_error))
    }

    /// 装载失败时恢复原始诊断（task-29③）：仅当本次装载**未写新错误**时恢复
    /// （Miss 回退不写 error→恢复旧值保诊断；Failed 分支已写新错误=最新事实
    /// 优先，不覆盖）。
    fn restore_error_if_unwritten(&self, old: Option<Arc<CordisError>>) {
        #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
        let mut core = self.core.lock().expect("fiber 决策锁");
        if core.error.is_none() {
            core.error = old;
        }
    }

    /// 配置更新（对齐 fiber.ts:783-819 update）：
    /// internal/commit 通知 → internal/update waterfall——监听者不调 `next()`
    /// 即保留实例只换配置，调 `next()` 则重启；未装载的 fiber 直接走重启。
    pub(crate) async fn update(self: &Arc<Self>, config: ConfigValue) -> Result<()> {
        // 前置只读检查（锁内零突变）：存活 + 装载态
        let loaded = {
            #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
            let core = self.core.lock().expect("fiber 决策锁");
            if core.uid.is_none() {
                return Err(CordisError::new(
                    ErrorKind::FiberDisposed,
                    format!("fiber <{}> 已处置，无法更新配置", self.name),
                ));
            }
            core.epoch != Epoch::Inactive
        };
        let Some(ctx) = self.ctx.clone() else {
            return Err(CordisError::new(ErrorKind::Internal, "fiber 缺少上下文"));
        };
        // S-ISO-1 C 刀判定前置（task-29① 一刀收两口）：root fiber 无插件在
        // **零突变**时报错（原实现此处后置——core.config 已被污染+COMMIT 已发）。
        let plugin = self.lock_core(|core| core.plugin.clone()).ok_or_else(|| {
            CordisError::new(
                ErrorKind::FiberDisposed,
                "root fiber 无插件，update 不适用（update/restart 面向插件 fiber）",
            )
        })?;
        // 候选配置解析与校验全部前置（task-29①：原实现先 core.config=config
        // 突变+COMMIT 事件、后校验——校验失败时 core 已持坏配置+提交通知已发，
        // restart 必用坏配置装载再败→Failed。「不动实例」注释由失实转真；
        // 同根收口云枢 task-16 nit①（守卫位后置）。对齐 fiber.ts:676-679
        // 校验失败抛 ValidationError 的语义：失败=实例零接触）
        let resolved = resolve_plugin_config(plugin.as_ref(), &ctx, &config)?;
        // ---- 校验全过：自此至突变完成无失败路径（dispose 竞态由二次闸挡）----
        {
            #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
            let mut core = self.core.lock().expect("fiber 决策锁");
            // 二次存活闸：校验在锁外跑，期间 dispose 可能已发生——不落新配置
            if core.uid.is_none() {
                return Err(CordisError::new(
                    ErrorKind::FiberDisposed,
                    format!("fiber <{}> 校验期间已被处置，配置未提交", self.name),
                ));
            }
            core.config = config;
            core.error = None; // 对齐 fiber.ts:800：update 清除旧错误
        }
        // internal/commit：持久化通知（突变成功后发出，「已提交」语义为真；
        // 事件段保留，一期无持久化订阅者）
        ctx.root.events.emit(
            &ctx.root.runtime,
            keys::COMMIT,
            &[Value::String(self.name.clone())],
        );
        if !loaded {
            return self.restart_inner().await;
        }
        // internal/update waterfall：内置行为 = 重启
        let this = self.clone();
        let builtin: WaterfallBuiltin = Arc::new(move |_args| {
            let task = this.clone();
            Box::pin(async move {
                task.restart_inner().await?;
                Ok(Value::Null)
            })
        });
        ctx.root
            .events
            .waterfall(keys::UPDATE, std::slice::from_ref(&resolved), builtin)
            .await
            .map(|_| ())
    }

    /// 卸载并处置本 fiber（幂等）：uid 置空 → 触发卸载路径 → 等 Disposed。
    pub(crate) async fn dispose(self: &Arc<Self>) -> Result<()> {
        let start_unload = {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            let mut core = self.core.lock().expect("fiber 决策锁");
            if core.uid.is_none() {
                return Ok(());
            }
            core.uid = None;
            core.error = None;
            core.epoch = Epoch::Inactive;
            !core.busy
        };
        self.cancel.cancel(); // 取消传播（方案 §2.3：组件长任务监听后静止）
        if start_unload {
            self.set_state(FiberState::Unloading);
            self.spawn_unload();
        }
        // busy 场景由运行中的装卸任务收尾（见 run_load/run_unload 的 uid 检查）
        FiberHandle::from_shared(self.clone()).wait_disposed().await;
        Ok(())
    }
}

impl FiberHandle {
    /// 原地重启（公开 API）。
    pub async fn restart(&self) -> Result<()> {
        self.0.restart_inner().await
    }

    /// 配置更新（公开 API，语义见 [`FiberShared::update`]）。
    pub async fn update(&self, config: ConfigValue) -> Result<()> {
        self.0.update(config).await
    }

    /// 卸载并处置本 fiber（公开 API，幂等）。
    pub async fn dispose(&self) -> Result<()> {
        self.0.dispose().await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试断言惯用 unwrap/expect；mod 级单点豁免（S-ISO-1 硬门口径，防逐处散落）
mod tests {
    use super::*;
    use crate::context::Context;

    /// S-ISO-1 C 类清零验收：root fiber（plugin=None）上调 update 返回 Err
    /// 而非 panic（原 ops.rs:87 `.expect("插件存在")` 是全内核唯一用户可预期
    /// 路径的 expect——云枢挑刺 P3-1）。
    #[tokio::test]
    async fn root_fiber_update返回err非panic() {
        let ctx = Context::new();
        let root_shared = ctx
            .inner
            .root
            .root_fiber
            .lock()
            .unwrap()
            .clone()
            .expect("root fiber 锚点应在场");
        let handle = FiberHandle::from_shared(root_shared);
        let err = handle
            .update(serde_json::json!({}))
            .await
            .expect_err("root fiber update 应返回 Err 而非 panic");
        assert!(
            err.message().contains("root fiber 无插件"),
            "人话指路: {err}"
        );
        // 主板存活：root 仍 Active（错误不炸现场）
        assert_eq!(handle.state(), crate::fiber::FiberState::Active);
    }

    /// task-29③ 条件恢复直测：restore_error_if_unwritten 只在「本次装载未写
    /// 新错误」时恢复旧诊断——①error=None（Miss 回退场景）恢复 old；
    /// ②error=Some（Failed 已写新错误）保留新不覆盖（最新事实优先）。
    #[tokio::test]
    async fn 装载失败条件恢复原始诊断() {
        let ctx = Context::new();
        let root_shared = ctx
            .inner
            .root
            .root_fiber
            .lock()
            .unwrap()
            .clone()
            .expect("root fiber 锚点应在场");
        let old: Arc<CordisError> = Arc::new(CordisError::new(
            ErrorKind::Internal,
            "原始装载错误 E1",
        ));
        let fresh: Arc<CordisError> = Arc::new(CordisError::new(
            ErrorKind::Internal,
            "本次装载新错误 E2",
        ));
        // ①error=None（Miss 场景模拟）→恢复 E1（诊断不静默丢失）
        {
            let mut core = root_shared.core.lock().unwrap();
            core.error = None;
        }
        root_shared.restore_error_if_unwritten(Some(old.clone()));
        let restored = root_shared.core.lock().unwrap().error.clone();
        assert!(restored.is_some(), "Miss 场景应恢复原始诊断");
        assert!(
            restored.unwrap().message().contains("E1"),
            "恢复的应是旧错误 E1"
        );
        // ②error=Some(E2)（Failed 场景模拟）→保留 E2 不被 old 覆盖
        {
            let mut core = root_shared.core.lock().unwrap();
            core.error = Some(fresh.clone());
        }
        root_shared.restore_error_if_unwritten(Some(old));
        let kept = root_shared.core.lock().unwrap().error.clone();
        assert!(
            kept.unwrap().message().contains("E2"),
            "Failed 已写新错误应保留（最新事实优先）"
        );
    }
}
