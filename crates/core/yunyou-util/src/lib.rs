//! yunyou-util：yunyou 全家共享的零散类型与工具。
//!
//! 对标 TS 版 `cosmokit` 中被内核用到的部分（方案 §2.6）：
//! - `Awaitable` → [`BoxFut`]（装箱的线程安全 future）；
//! - `deepEqual` → [`values::deep_equal`]；
//! - 二进制编码预留 → [`binary`]（LEB128 变长整数，供未来子进程协议作长度前缀）。
//!
//! 职责边界：只放「无状态、无 I/O、被两个以上 crate 复用」的纯工具；
//! 与内核语义相关的类型（Context/Fiber/事件）一律在 yunyou，不在此处。

pub mod binary;
pub mod values;

use std::future::Future;
use std::pin::Pin;

/// 装箱的线程安全 future（对标 cosmokit 的 `Awaitable<T>`）。
///
/// 内核与组件的一切异步方法签名统一用它表达返回值：
/// trait object 安全（无 `impl Trait`），并强制 `Send`（tokio 多线程调度纪律，方案 §2.3）。
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[cfg(test)]
mod tests {
    use super::BoxFut;

    /// BoxFut 应可跨线程移动并被执行（Send 约束的冒烟验证）。
    #[tokio::test]
    async fn box_fut_is_send_and_runnable() {
        let fut: BoxFut<'static, u32> = Box::pin(async { 7 + 35 });
        let handle = tokio::spawn(fut);
        assert_eq!(handle.await.unwrap(), 42);
    }
}
