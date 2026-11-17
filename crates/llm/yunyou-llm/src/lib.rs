//! Llm 服务契约 crate（Service Definition，方案 §2.1.3 capability seam）。
//!
//! 本 crate 只定义**类型与 trait**，不含任何网络实现；OpenAI 兼容实现见
//! `yunyou-llm-openai`（Provider，可被同类 crate 替换而不动消费方）。
//!
//! # 契约（评审线一强制）
//! - [`Llm`] 与 [`LlmStream`] 必须保持 object-safe：无泛型方法、无 `Self` 返回值，
//!   异步一律返回 [`BoxFut`]（方案 §2.1.3 / 风险 6）；
//! - 实现必须 `Send + Sync`（tokio 多线程，方案 §2.3）；
//! - 流语义：[`LlmStream::next`] 返回 `None` 表示流正常结束；实现方必须保证
//!   出错时返回 `Err` 且之后的行为由实现声明（openai 实现保证此后恒为 `None`）；
//! - 失败语义：Provider 失败归一化为 [`LlmFailure`]（结构化：类别码 + 事实保留
//!   message），再经 `into_cordis` 折叠为 `CordisError{ ErrorKind::Internal, message:
//!   "llm/<code>: <detail>" }`——一期不对内核 ErrorKind 扩容，`llm/<code>` 前缀
//!   承载归一化类别，S10 重试插件经 `LlmFailure::from_cordis` 还原。

pub mod error;
pub mod retry_policy;
pub mod types;

pub use error::{LlmFailure, LlmFailureCategory};
pub use retry_policy::{RetryAction, RetryMode, RetryPolicy, RetryPolicyProvider};
pub use types::{
    GenerateRequest, Message, Role, StreamDelta, ToolCallDelta, ToolCallRequest, ToolSchema,
    Usage,
};

use tokio_util::sync::CancellationToken;
use yunyou::Result;
use yunyou_util::BoxFut;

/// LLM 服务 trait（对标 TS 版 `llm.stream(options)`，packages/llm/llm/src/index.ts:206）。
///
/// 实现方（Provider crate）经 `ctx.provide(ServiceId::of::<dyn Llm>(), ...)` 注册；
/// 消费方（agent-loop）经 `ctx.get::<dyn Llm>()` 取用。
pub trait Llm: Send + Sync {
    /// 发起一次流式生成调用。
    ///
    /// # 前置条件
    /// `req.model` 非空；`req.messages` 至少一条。
    /// # 后置条件
    /// 返回的流按序产出增量，以 `None` 结束；不缓冲全量响应。
    /// # 失败语义
    /// 连接失败 / 首包前协议错误 → `Err`（归一化 [`LlmFailure`]，如
    /// [`LlmFailureCategory::Connection`]）；流中途失败由 [`LlmStream::next`] 返回
    /// `Err` 体现（如 [`LlmFailureCategory::StreamInterrupted`]）。
    fn stream(&self, req: GenerateRequest) -> BoxFut<'static, Result<Box<dyn LlmStream>>>;

    /// 带取消信号的流式调用（R2，超时重试插件设计 §4/§5.1）。
    ///
    /// 取消信号优先级高于一切：触发即断流，失败归为
    /// [`LlmFailureCategory::Cancelled`]（不重试——取消是用户意志，非故障）。
    /// # 最小破坏形态
    /// 默认实现转发 [`Self::stream`] 并忽略令牌：未接取消的旧实现/测试替身无需
    /// 改动即可编译运行；真正消费取消的 Provider override 本方法，在 SSE 读循环
    /// `select!` 该令牌（见 `yunyou-llm-openai`）。
    fn stream_cancellable(
        &self,
        req: GenerateRequest,
        cancel: CancellationToken,
    ) -> BoxFut<'static, Result<Box<dyn LlmStream>>> {
        // 默认实现不消费取消信号（沿用 stream 语义）；显式 drop 表明有意忽略，
        // 同时满足 `-D warnings`（unused_variables）。
        drop(cancel);
        self.stream(req)
    }
}

/// 一次模型调用的增量流句柄。
pub trait LlmStream: Send {
    /// 拉取下一个增量；`None` 表示流正常结束。
    fn next(&mut self) -> BoxFut<'_, Result<Option<StreamDelta>>>;
}
