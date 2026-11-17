//! panic 边界辅助（《插件崩溃隔离设计》§3.2.3/§4.1，S-ISO-2）。
//!
//! 职责单一：把「插件代码 panic」从三种执行容器归一为 [`ErrorKind::PluginPanicked`]：
//! - tokio 任务边界（setup 装载、waterfall 监听环）→ [`join_error`]；
//! - 同步注册体（effect/provide，决策锁内）→ [`catch_panic`]。
//!
//! **全内核唯一的 `AssertUnwindSafe` 集中点**（设计 §9 风险 6：扩散即审查线一打回）。
//! 红线：仅限内核同步回调面使用——插件代码任意写，该声明只对「panic 后本次注册
//! 整体丢弃、已半程写入由 finish_load_error 逆序清理收敛」的场景成立。

use crate::error::{CordisError, ErrorKind, Result};

/// 同步回调面的 panic 边界：注册体 panic 归一为 `Err(PluginPanicked)`。
///
/// 关键性质（std 锁方案的存亡条款，云枢裁决 2026-11-16）——**拓扑敏感**：
/// `catch_unwind` 把 panic 拦在「catch 边界」处，边界**之内**展开路径上的
/// std 锁 guard 仍会被 Drop 标记中毒；**边界之外**（调用方先持有）的 guard
/// 不经过展开 → 不中毒。effect_with 的真实拓扑恰为后者（lock_core 先拿锁、
/// 本函数在闭包内）→ 决策锁不中毒、同锁后续操作照常。推论（契约的机制原因）：
/// **注册体内不得再次 lock_core**——界内自取的锁必中毒（tests 锁两分支）。
pub(crate) fn catch_panic<T>(label: &str, execute: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(execute))
        .map_err(|payload| panic_error(label, payload))
        .and_then(std::convert::identity)
}

/// tokio 任务边界的 panic 归一：`JoinError`（`is_panic`）→ `PluginPanicked`。
pub(crate) fn join_error(what: &str, join_err: tokio::task::JoinError) -> CordisError {
    if join_err.is_panic() {
        panic_error(what, join_err.into_panic())
    } else {
        // 非 panic 的任务取消：本边界两处（setup 装载/waterfall 环）都是同步
        // await 收尾、无取消源，出现即内核缺陷——仍归 PluginPanicked 语义族，
        // 消息明示形态差异，不静默吞（失败 loud 纪律）。
        CordisError::new(
            ErrorKind::PluginPanicked,
            format!("{what}：任务异常结束（非 panic 形态：{join_err}）"),
        )
    }
}

/// panic payload → 人话消息（`&str`/`String` 之外未知 payload 降级占位，不 panic）。
fn panic_error(what: &str, payload: Box<dyn std::any::Any + Send>) -> CordisError {
    let detail = if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "（非文本 panic payload）".to_string()
    };
    CordisError::new(
        ErrorKind::PluginPanicked,
        format!("{what} 发生 panic，已被内核边界隔离：{detail}"),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试断言惯用 unwrap/expect；mod 级单点豁免（S-ISO-1 硬门口径，防逐处散落）
mod tests {
    use super::*;

    /// catch_panic：panic 归一为 PluginPanicked，正常值与 Err 原样透传。
    #[test]
    fn panic归一与透传() {
        let boom: Result<u8> = catch_panic("测试点", || panic!("爆炸字符串"));
        let err = boom.expect_err("panic 应归一为 Err");
        assert_eq!(err.kind(), &ErrorKind::PluginPanicked);
        assert!(err.message().contains("爆炸字符串"), "{}", err.message());
        assert!(err.message().contains("测试点"), "{}", err.message());

        let boxed: Result<u8> = catch_panic("测试点", || -> Result<u8> {
            panic!("{}", String::from("Box<String> 爆炸"))
        });
        assert!(boxed.expect_err("").message().contains("Box<String> 爆炸"));

        let non_text: Result<u8> =
            catch_panic("测试点", || -> Result<u8> { std::panic::panic_any(7u32) });
        assert!(
            non_text.expect_err("").message().contains("非文本 panic payload"),
            "未知 payload 应降级占位"
        );

        assert_eq!(catch_panic("测试点", || Ok::<_, CordisError>(41)).unwrap(), 41);
        let pass_err: Result<u8> = catch_panic("测试点", || {
            Err(CordisError::new(ErrorKind::PluginSetup, "普通错误"))
        });
        assert_eq!(pass_err.unwrap_err().kind(), &ErrorKind::PluginSetup, "非 panic 的 Err 原样透传");
    }

    /// 锁不毒的直接证明（std 锁方案存亡条款）——**拓扑敏感，两分支分立**：
    /// ① guard 在 catch 边界之外（=effect_with 真实拓扑：lock_core 先拿锁、
    ///    catch_panic 在闭包内）→ panic 展开只到 catch 边界、不穿 guard → 锁不中毒；
    /// ② guard 在边界之内自取 → panic 展开穿过 guard Drop → 中毒（反面证明）。
    /// 分支②正是「注册体内不得再次 lock_core」契约（effect.rs 锁纪律）必须存在的
    /// 机制原因——契约若破，本测试分支②的中毒面即回归。
    #[test]
    fn 锁不毒拓扑两分支() {
        let lock = std::sync::Mutex::new(0u8);
        // 分支①：guard 在界外持有（effect_with 拓扑）
        let result = {
            let guard = lock.lock().unwrap();
            let r = catch_panic("锁内注入", || -> Result<()> {
                panic!("注册体 panic（guard 在界外持有）")
            });
            (r, guard)
        };
        assert!(result.0.is_err(), "panic 应归一 Err");
        drop(result.1);
        {
            let mut guard = lock.lock().expect("分支①：锁应可用（未中毒）");
            *guard += 1;
        }
        // 分支②：guard 在边界内自取——中毒复现（反面证明，锁污染留到用例尾部）
        let _: Result<()> = catch_panic("边界内自取锁", || {
            let _guard = lock.lock().unwrap();
            panic!("此 panic 展开穿过 guard")
        });
        assert!(
            lock.lock().is_err(),
            "分支②：边界内自取的锁应已中毒（契约「注册体内禁 lock_core」的机制原因）"
        );
    }
}
