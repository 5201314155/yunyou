//! waterfall 分发的链机器（五分发之一，从 events.rs 拆出：events.rs 行数顶到
//! 硬顶前按职责拆模块，行为零改，符号经 events.rs re-export 保持原路径可用）。
//!
//! 语义（对标 events.ts:31,246-261）：监听器从外到内包裹 `next()`；**不调
//! `next()` 即否决**后续监听器与内置行为；重复调用 `next()` 报 WaterfallReentry
//! （对标 events.ts:254，专项测试见 tests/lifecycle.rs）。

use crate::error::{CordisError, ErrorKind, Result};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use yunyou_util::BoxFut;

/// waterfall 的内置行为（链尽时执行；如 internal/update 的「重启」）。
pub type WaterfallBuiltin = Arc<dyn Fn(&[Value]) -> BoxFut<'static, Result<Value>> + Send + Sync>;

/// waterfall 监听器：包裹后续链条。
pub type WaterfallListener =
    Arc<dyn Fn(&[Value], WaterfallNext) -> BoxFut<'static, Result<Value>> + Send + Sync>;

/// waterfall 链光标：指向「下一个待执行监听器」的续延。
struct ChainCursor {
    listeners: Arc<Vec<WaterfallListener>>,
    index: usize,
    args: Arc<Vec<Value>>,
    builtin: WaterfallBuiltin,
}

impl ChainCursor {
    fn run(self: &Arc<Self>) -> BoxFut<'static, Result<Value>> {
        let this = self.clone();
        Box::pin(async move { this.advance().await })
    }

    async fn advance(self: Arc<Self>) -> Result<Value> {
        if let Some(listener) = self.listeners.get(self.index) {
            let next = WaterfallNext::chain(Arc::new(ChainCursor {
                listeners: self.listeners.clone(),
                index: self.index + 1,
                args: self.args.clone(),
                builtin: self.builtin.clone(),
            }));
            // S-ISO-2 panic 边界（设计 §3.2.4，凶点三；C6 语义=云枢批甲 2026-11-16）：
            // 监听器环在独立任务中执行，Poll 期 panic 归一 Err(PluginPanicked)——
            // **链停、builtin 不执行、调用方收 Err**（fail-closed 保旧状态：与 serial
            // 分发 panic→Err 同语义；TS 同构=监听器 throw ⇒ update reject ⇒ 配置不落）。
            // 辨析：veto（不调 next、直接返回 Ok）同为链停但形态是 Ok 且无 builtin——
            // panic-Err 与 veto-Ok 两者 Outcome 可区分，用例分立断言（tests/crash_isolation.rs）。
            let joined = tokio::spawn(listener(&self.args, next)).await;
            match joined {
                Ok(result) => result,
                Err(join_err) => Err(crate::panic::join_error("waterfall 监听器", join_err)),
            }
        } else {
            (self.builtin)(&self.args).await
        }
    }
}

/// waterfall 的 `next()` 句柄：只能调用一次，重复调用报错（events.ts:254）。
#[derive(Clone)]
pub struct WaterfallNext {
    state: Arc<Mutex<NextSlot>>,
}

enum NextSlot {
    /// 未调用：存有链的剩余段。
    Pending(Box<dyn FnOnce() -> BoxFut<'static, Result<Value>> + Send>),
    /// 已调用：再调即重入。
    Called,
}

impl WaterfallNext {
    fn chain(cursor: Arc<ChainCursor>) -> Self {
        let slot: NextSlot = NextSlot::Pending(Box::new(move || cursor.run()));
        Self { state: Arc::new(Mutex::new(slot)) }
    }

    /// 调用链的剩余段。第二次调用返回 WaterfallReentry 错误。
    pub fn call(&self) -> BoxFut<'static, Result<Value>> {
        #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
        let mut slot = self.state.lock().expect("waterfall next 锁");
        match std::mem::replace(&mut *slot, NextSlot::Called) {
            NextSlot::Pending(run) => run(),
            NextSlot::Called => Box::pin(async {
                Err(CordisError::new(
                    ErrorKind::WaterfallReentry,
                    "waterfall 的 next() 被重复调用",
                ))
            }),
        }
    }
}

impl crate::events::EventTable {
    /// waterfall：监听器从外到内包裹链条，链尽执行内置行为。
    pub async fn waterfall(
        &self,
        event: &str,
        args: &[Value],
        builtin: WaterfallBuiltin,
    ) -> Result<Value> {
        let listeners = {
            #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
            let table = self.waterfall.lock().expect("waterfall 表锁");
            table.get(event).cloned().unwrap_or_default()
        };
        if listeners.is_empty() {
            return builtin(args).await;
        }
        let cursor = Arc::new(ChainCursor {
            listeners: Arc::new(listeners),
            index: 0,
            args: Arc::new(args.to_vec()),
            builtin,
        });
        cursor.run().await
    }

    /// 注册 waterfall 监听器。
    pub fn add_waterfall(&self, event: &str, listener: WaterfallListener) {
        #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        self.waterfall
            .lock()
            .expect("waterfall 表锁")
            .entry(event.to_string())
            .or_default()
            .push(listener);
    }

    /// 按指针相等移除 waterfall 监听器。
    pub fn remove_waterfall(&self, event: &str, listener: &WaterfallListener) -> bool {
        // S-ISO-1 逐点豁免（属性不落实参表达式，提行 let 化）
        #[allow(clippy::expect_used)] // 中毒 tripwire（A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
        let mut waterfall_guard = self.waterfall.lock().expect("waterfall 表锁");
        super::remove_by_ptr(&mut waterfall_guard, event, |h| {
            Arc::ptr_eq(h, listener)
        })
    }
}