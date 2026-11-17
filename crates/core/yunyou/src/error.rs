//! 内核统一错误类型与错误码表（对标 cordis 的 CordisError 家族）。
//!
//! `ErrorKind` 承载分类（供程序化匹配），`message` 承载中文可读细节；
//! parallel 分发的多监听器失败聚合为 [`ErrorKind::Aggregate`]（对标 `Promise.allSettled`
//! + `AggregateError`，events.ts:191-196）。

use std::fmt;

/// 错误类别码表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// `ctx.get` 请求的服务未注册，或提供者 fiber 尚未 Active（对标 reflect.ts:152）。
    ServiceNotFound,
    /// 同一作用域内重复注册同名服务（对标 reflect.ts 的 registered 错误）。
    ServiceDuplicate,
    /// 服务已注册但请求的 Rust 类型与注册值不匹配（TypeMap 下行转换失败）。
    ServiceTypeMismatch,
    /// 插件配置未通过 schema 校验（明细见 [`crate::config::ValidationError`]）。
    ConfigValidation,
    /// 插件 setup future 执行失败。
    PluginSetup,
    /// 插件代码 panic，已被内核边界捕获并隔离（《插件崩溃隔离设计》§4.1）：
    /// setup 装载任务 / waterfall 监听环 / effect 注册体三类入口，panic 归一为
    /// 本类别后走 Failed 状态转移（可经 restart 复位）。
    PluginPanicked,
    /// 对已卸载或无归属 fiber 的上下文注册 effect（对标 INACTIVE_EFFECT）。
    EffectInactive,
    /// waterfall 的 `next()` 被重复调用（对标 events.ts:254）。
    WaterfallReentry,
    /// fiber 处于 FAILED 态（await 时抛出装载错误，对标 fiber.ts:754-760）。
    FiberFailed,
    /// fiber 已 DISPOSED 后仍尝试 update/restart 等操作。
    FiberDisposed,
    /// parallel 分发下部分监听器失败（聚合根）。
    Aggregate,
    /// MessageHub（契约 message.v1）：成员名重复——open_mailbox 同名（任何态）、
    /// register 目标名已 Online 或当前 fiber 已绑定他名（重名绑定=路由欺骗，绝不静默接管）。
    HubDuplicateMember,
    /// MessageHub（契约 message.v1）：寻址/消费身份不可达——send 目标名无任何条目
    /// （typo 不静默吞），或调用 fiber 未绑定成员（无身份者发不出、越权读他人信箱被拦）。
    HubUnknownMember,
    /// MessageHub（契约 message.v1）：成员名非法——空名/全空白（空名=永不可达地址，注册时堵）。
    HubInvalidName,
    /// 其余内核内部错误（不应出现，出现即为内核缺陷）。
    Internal,
}

/// 内核错误：kind 分类 + 中文信息 + 聚合子错误。
#[derive(Debug, Clone)]
pub struct CordisError {
    kind: ErrorKind,
    message: String,
    causes: Vec<CordisError>,
}

impl CordisError {
    /// 构造指定类别的错误。
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), causes: Vec::new() }
    }

    /// 构造 parallel 聚合错误。
    pub(crate) fn aggregate(causes: Vec<CordisError>) -> Self {
        let message = format!("{} 个监听器执行失败", causes.len());
        Self { kind: ErrorKind::Aggregate, message, causes }
    }

    /// 错误类别。
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// 错误信息（中文）。
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 聚合的子错误（仅 Aggregate 类别非空）。
    pub fn causes(&self) -> &[CordisError] {
        &self.causes
    }
}

impl fmt::Display for CordisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)?;
        for cause in &self.causes {
            write!(f, "\n  - {cause}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CordisError {}

/// 内核统一结果别名。
pub type Result<T> = std::result::Result<T, CordisError>;

#[cfg(test)]
mod tests {
    use super::{CordisError, ErrorKind};

    #[test]
    fn 聚合错误的展示含子项() {
        let inner1 = CordisError::new(ErrorKind::PluginSetup, "监听器一失败");
        let inner2 = CordisError::new(ErrorKind::PluginSetup, "监听器二失败");
        let agg = CordisError::aggregate(vec![inner1, inner2]);
        assert_eq!(agg.kind(), &ErrorKind::Aggregate);
        assert_eq!(agg.causes().len(), 2);
        let text = agg.to_string();
        assert!(text.contains("监听器一失败") && text.contains("监听器二失败"));
    }
}
