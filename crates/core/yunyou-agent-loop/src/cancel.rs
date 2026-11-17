//! 取消源合并（方案 §5.1 R1，热插拔「残尸」根源之一）：对话轮取消与 fiber 卸载
//! 取消本是两条独立线——插件被禁用时在途轮不会收流，即「残尸」的具体来源。
//! 本模块把两源合一到**调用方令牌的 child token** 上：任一触发即整轮收流
//! （令牌一路传到工具执行，下游只见合一后的一个令牌）。

use tokio_util::sync::CancellationToken;

/// 合并两路取消源，返回「合一令牌 + 桥接守卫」。
///
/// - **合并点**：调用方令牌的 child token——调用方取消经父子链自动传播；
/// - **fiber 卸载取消**：桥接任务监听该源并触发 child；fiber 已先取消时
///   **立即**触发 child（不等桥接任务被调度，杜绝漏信号）；
/// - **无孤儿**：守卫 drop 即中止桥接任务——本轮收流不残留监听任务。
pub(crate) fn merge_cancel(
    caller: &CancellationToken,
    fiber: &CancellationToken,
) -> (CancellationToken, CancelBridge) {
    let merged = caller.child_token();
    let watcher = if fiber.is_cancelled() {
        merged.cancel();
        None
    } else {
        let fiber = fiber.clone();
        let child = merged.clone();
        Some(tokio::spawn(async move {
            fiber.cancelled().await;
            child.cancel();
        }))
    };
    (merged, CancelBridge { watcher })
}

/// 桥接任务守卫：drop 时中止 fiber 取消监听任务。
///
/// 存活于整轮对话（调用方以 `_bridge` 绑定即随函数作用域收尾）：本轮结束
/// 监听即止，不会留下跨轮存活的任务（孤儿任务=另一类残尸）。
pub(crate) struct CancelBridge {
    watcher: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for CancelBridge {
    fn drop(&mut self) {
        if let Some(watcher) = &self.watcher {
            watcher.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 轮询等待合一令牌生效（桥接任务需被调度；多线程下单次 yield 不定序）。
    async fn await_cancelled(token: &CancellationToken) -> bool {
        for _ in 0..200 {
            if token.is_cancelled() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        token.is_cancelled()
    }

    /// 两源静默：合一令牌不取消（合并不误伤正常轮）。
    #[tokio::test]
    async fn 两源静默时合一令牌不取消() {
        let caller = CancellationToken::new();
        let fiber = CancellationToken::new();
        let (merged, _bridge) = merge_cancel(&caller, &fiber);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!merged.is_cancelled(), "两源均未触发，合一令牌不应取消");
    }

    /// 对话轮取消源：经 child 父子链独立触发合一令牌，且不反向波及 fiber 源。
    #[tokio::test]
    async fn 对话轮取消源独立触发() {
        let caller = CancellationToken::new();
        let fiber = CancellationToken::new();
        let (merged, _bridge) = merge_cancel(&caller, &fiber);
        caller.cancel();
        assert!(merged.is_cancelled(), "父子链应传播调用方取消");
        assert!(!fiber.is_cancelled(), "调用方取消不应反向触发 fiber 源");
    }

    /// fiber 卸载取消源：独立触发合一令牌（本轮收流的 R1 主路径）。
    #[tokio::test]
    async fn fiber卸载取消源独立触发() {
        let caller = CancellationToken::new();
        let fiber = CancellationToken::new();
        let (merged, _bridge) = merge_cancel(&caller, &fiber);
        fiber.cancel();
        assert!(await_cancelled(&merged).await, "fiber 取消应经桥接触发合一令牌");
        assert!(!caller.is_cancelled(), "fiber 取消不应反向触发调用方源");
    }

    /// fiber 已先取消：合一令牌立即取消（漏信号防护，不等桥接任务调度）。
    #[tokio::test]
    async fn fiber先取消则立即合一() {
        let fiber = CancellationToken::new();
        fiber.cancel();
        let (merged, _bridge) = merge_cancel(&CancellationToken::new(), &fiber);
        assert!(merged.is_cancelled(), "fiber 已取消时合一令牌应立即取消");
    }

    /// 守卫 drop：迟到的 fiber 取消不再触发合一令牌（桥接任务已中止，无孤儿监听）。
    #[tokio::test]
    async fn 守卫drop后迟到取消不误触发() {
        let caller = CancellationToken::new();
        let fiber = CancellationToken::new();
        let merged = {
            let (merged, _bridge) = merge_cancel(&caller, &fiber);
            merged
            // _bridge 在此 drop → 桥接任务中止
        };
        fiber.cancel();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!merged.is_cancelled(), "守卫已 drop：迟到取消不应触发合一令牌");
    }
}