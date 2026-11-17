//! LLM 失败结构化类型（P0-8，超时重试插件设计 §3.1）。
//!
//! 归一化原则（设计 §8 风险 2）：失败分类在 llm 契约层完成，白名单按**归一化
//! 类别码**匹配而非上游原始错误码（中转层错误码 ≠ 上游语义）。原始 message 与
//! HTTP 状态码**事实保留**，供观测与诊断。
//!
//! 与内核错误的搭载：一期不动内核 ErrorKind 扩容（既定口径），[`LlmFailure`] 经
//! [`LlmFailure::into_cordis`] 折叠为 `CordisError{ Internal, message: "llm/<code>: <detail>" }`；
//! [`LlmFailure::from_cordis`] 反向还原（供 S10 重试插件消费）。

use yunyou::{CordisError, ErrorKind};

/// 失败类别（对应设计 §3.1 分类表，**七类**）。§3.1 的「401/403 鉴权」与「请求
/// 4xx」两行默认动作一致（均不重试），此处合并为 [`Self::ClientError`] 一类，
/// 401/403 细分由 [`LlmFailure::status`] 与 message 事实保留。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmFailureCategory {
    /// 连接失败（connect/dns/握手）：请求未达上游——白名单可重试。
    Connection,
    /// HTTP 5xx（上游错误）：瞬态——白名单可重试。
    ServerError,
    /// HTTP 429（限流）：白名单可重试，退避上限取大。
    RateLimited,
    /// 流中断（SSE 半途断连，无内容落盘）：本轮无产出——白名单可重试。
    StreamInterrupted,
    /// 客户端 4xx（含 401/403 鉴权 / schema / 参数错）：确定性失败——不重试。
    ClientError,
    /// 本地超时（请求预算耗尽）：默认不重试（安全默认律：不确定结果不自动重试）。
    Timeout,
    /// 取消信号触发：用户意志，非故障——不重试。
    Cancelled,
}

impl LlmFailureCategory {
    /// 归一化类别码（白名单 `retryable_codes` 按此匹配，设计 §5）。
    pub fn code(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::ServerError => "http_5xx",
            Self::RateLimited => "http_429",
            Self::StreamInterrupted => "stream_interrupted",
            Self::ClientError => "http_4xx",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
        }
    }

    /// §3.1 默认动作：是否可重试（默认策略白名单即取此集合）。
    pub fn retryable(self) -> bool {
        matches!(self, Self::Connection | Self::ServerError | Self::RateLimited | Self::StreamInterrupted)
    }

    /// 从归一化码反查类别（[`LlmFailure::from_cordis`] 用）。
    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "connection" => Self::Connection,
            "http_5xx" => Self::ServerError,
            "http_429" => Self::RateLimited,
            "stream_interrupted" => Self::StreamInterrupted,
            "http_4xx" => Self::ClientError,
            "timeout" => Self::Timeout,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// 从 HTTP 状态码归一化类别（4xx 细分由 status 事实保留，不按原始码分白名单）。
    /// 3xx 一并归 ServerError：reqwest 默认跟随重定向，正常链路不会在此见到 3xx，
    /// 一旦出现即按上游错处理（注记备查，不为其单开类别）。
    pub fn from_status(status: u16) -> Self {
        match status {
            429 => Self::RateLimited,
            400..=499 => Self::ClientError,
            _ => Self::ServerError,
        }
    }
}

impl std::fmt::Display for LlmFailureCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

/// 结构化失败：归一化类别 + 事实保留的原始 message（+ 可选 HTTP 状态码）。
#[derive(Debug, Clone)]
pub struct LlmFailure {
    pub category: LlmFailureCategory,
    /// 原始错误 message（事实保留：上游/协议细节，不裁剪）。
    pub message: String,
    /// HTTP 状态码（若有；便于观测，4xx 细分依据）。
    pub status: Option<u16>,
}

impl LlmFailure {
    pub fn new(category: LlmFailureCategory, message: impl Into<String>) -> Self {
        Self { category, message: message.into(), status: None }
    }

    /// 归一化类别码（= [`Self::category`].code()）。
    pub fn code(&self) -> &'static str {
        self.category.code()
    }

    /// 连接失败（connect/dns/握手）。
    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(LlmFailureCategory::Connection, message)
    }

    /// 流中断（SSE 半途断连 / 分片协议损坏）。
    pub fn stream_interrupted(message: impl Into<String>) -> Self {
        Self::new(LlmFailureCategory::StreamInterrupted, message)
    }

    /// 本地超时（调用侧请求预算耗尽）。
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(LlmFailureCategory::Timeout, message)
    }

    /// 取消信号触发。
    pub fn cancelled() -> Self {
        Self::new(LlmFailureCategory::Cancelled, "调用方取消")
    }

    /// 从 HTTP 状态码归一化（携带 status 事实保留）。
    pub fn from_http_status(status: u16, message: impl Into<String>) -> Self {
        let category = LlmFailureCategory::from_status(status);
        Self { category, message: message.into(), status: Some(status) }
    }

    /// 折叠为内核错误（沿用 `llm/<code>: <detail>` 前缀，一期不扩容 ErrorKind）。
    pub fn into_cordis(self) -> CordisError {
        let detail = match self.status {
            Some(status) => format!("端点返回 {status}: {}", self.message),
            None => self.message,
        };
        CordisError::new(ErrorKind::Internal, format!("llm/{}: {detail}", self.category.code()))
    }

    /// 从内核错误还原（解析 `llm/<code>:` 前缀；非 llm 错误 / 未知码返回 `None`）。
    /// 注记：折叠消息不携带 status，还原态 `status` 恒为 `None`——白名单匹配只需
    /// 类别码；status 事实保留仅在失败当场（`into_cordis` 之前的 `LlmFailure`）有效。
    pub fn from_cordis(err: &CordisError) -> Option<Self> {
        let rest = err.message().strip_prefix("llm/")?;
        let (code, detail) = rest.split_once(": ")?;
        let category = LlmFailureCategory::from_code(code)?;
        Some(Self { category, message: detail.to_string(), status: None })
    }

    /// 用户可读的一句人话（展示层用，task-34①）：不含内部类别码/原始报文/重复措辞。
    ///
    /// 与 [`Self::into_cordis`] 并行不悖：折叠消息保持 `llm/<code>: <detail>` 机器
    /// 格式（S10 重试白名单按前缀解析 + 原始 message 事实保留），人话只在展示层
    /// 渲染——对话失败路径经 [`Self::from_cordis`] 还原后取本方法输出。
    /// 层级纪律：本方法不引用 CLI 配置文件路径（那是宿主层知识，由展示层补）。
    ///
    /// 还原态兼容：`from_cordis` 不携带 status/endpoint，此处从 detail 提取——
    /// 提取器消费的是本 workspace llm-openai 插件的确定性构造格式（带测试锁），
    /// 提取不到一律落泛化文案，不会因格式漂移而产出错误信息。
    pub fn user_message(&self) -> String {
        let status = self.status.or_else(|| extract_status(&self.message));
        match self.category {
            LlmFailureCategory::ClientError => match status {
                Some(401) | Some(403) => {
                    "服务拒绝了请求（401 未授权）：API key 缺失或无效，请检查 api_key 配置".to_string()
                }
                Some(status) => {
                    format!("服务拒绝了请求（HTTP {status}）：请检查模型名与请求参数是否正确")
                }
                None => "服务拒绝了请求：请检查模型名与请求参数是否正确".to_string(),
            },
            LlmFailureCategory::RateLimited => {
                "模型服务限流（429）：请求过于频繁，请稍后重试".to_string()
            }
            LlmFailureCategory::Connection => match extract_endpoint(&self.message) {
                Some(endpoint) => {
                    format!("无法连接模型服务 {endpoint}：请确认服务已启动且 base_url 配置正确")
                }
                None => "无法连接模型服务：请确认服务地址（base_url）正确且服务已启动".to_string(),
            },
            LlmFailureCategory::ServerError => {
                "模型服务暂时不可用（服务端错误）：请稍后重试".to_string()
            }
            LlmFailureCategory::StreamInterrupted => {
                "回复传输中断：本轮未收到完整回复，可重试".to_string()
            }
            LlmFailureCategory::Timeout => match extract_endpoint(&self.message) {
                Some(endpoint) => {
                    format!("请求超时（{endpoint}）：服务响应过慢或网络不稳定，可重试或调大超时配置")
                }
                None => "请求超时：服务响应过慢或网络不稳定，可重试或调大超时配置".to_string(),
            },
            LlmFailureCategory::Cancelled => "已取消本轮对话".to_string(),
        }
    }
}

/// 从 detail 提取 HTTP 状态码（构造契约：llm-openai plugin 对 HTTP 失败统一
/// `端点返回 {status}: …`；status 形如 `401` 或 `401 Unauthorized`）。
fn extract_status(detail: &str) -> Option<u16> {
    detail
        .strip_prefix("端点返回 ")?
        .split(':')
        .next()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// 从 detail 提取请求端点（构造契约：连接/超时失败 message = `请求 {url} …`）。
fn extract_endpoint(detail: &str) -> Option<&str> {
    detail
        .strip_prefix("请求 ")?
        .split_whitespace()
        .next()
        .filter(|url| url.starts_with("http"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 状态码归一化：429→限流、4xx/401/403→客户端错、5xx→上游错；status 事实保留。
    #[test]
    fn 状态码归一化() {
        assert_eq!(LlmFailureCategory::from_status(401), LlmFailureCategory::ClientError);
        assert_eq!(LlmFailureCategory::from_status(403), LlmFailureCategory::ClientError);
        assert_eq!(LlmFailureCategory::from_status(429), LlmFailureCategory::RateLimited);
        assert_eq!(LlmFailureCategory::from_status(400), LlmFailureCategory::ClientError);
        assert_eq!(LlmFailureCategory::from_status(404), LlmFailureCategory::ClientError);
        assert_eq!(LlmFailureCategory::from_status(500), LlmFailureCategory::ServerError);
        assert_eq!(LlmFailureCategory::from_status(503), LlmFailureCategory::ServerError);
        let f = LlmFailure::from_http_status(401, "未授权");
        assert_eq!(f.status, Some(401));
        assert_eq!(f.category, LlmFailureCategory::ClientError);
    }

    /// §3.1 默认动作表：4 类白名单可重试，其余 3 类不重试。
    #[test]
    fn 默认重试表() {
        assert!(LlmFailureCategory::Connection.retryable());
        assert!(LlmFailureCategory::ServerError.retryable());
        assert!(LlmFailureCategory::RateLimited.retryable());
        assert!(LlmFailureCategory::StreamInterrupted.retryable());
        assert!(!LlmFailureCategory::ClientError.retryable());
        assert!(!LlmFailureCategory::Timeout.retryable());
        assert!(!LlmFailureCategory::Cancelled.retryable());
    }

    /// into_cordis / from_cordis 往返：类别码与 message 均保留；非 llm 错误不还原。
    #[test]
    fn cordis往返() {
        let f = LlmFailure::connection("拨号失败");
        let err = f.into_cordis();
        assert_eq!(err.kind(), &ErrorKind::Internal);
        assert!(err.message().starts_with("llm/connection:"), "应带 llm/<code> 前缀");
        let back = LlmFailure::from_cordis(&err).expect("应可还原");
        assert_eq!(back.category, LlmFailureCategory::Connection);
        assert_eq!(back.message, "拨号失败");
        assert!(LlmFailure::from_cordis(&CordisError::new(ErrorKind::Internal, "loop/cancelled")).is_none());
    }

    /// code() 与 from_code 一一对应（白名单锚点不漂移）。
    #[test]
    fn 码表往返() {
        use LlmFailureCategory::*;
        for c in [Connection, ServerError, RateLimited, StreamInterrupted, ClientError, Timeout, Cancelled] {
            assert_eq!(LlmFailureCategory::from_code(c.code()), Some(c));
        }
        assert_eq!(LlmFailureCategory::from_code("nope"), None);
    }

    /// user_message（task-34①）：401 人话点名 key；连接给服务指路；
    /// 不含内部类别码、不直出原始报文（黑话/JSON 泄漏双防回归锁）。
    #[test]
    fn 人话消息不含内部码与原始报文() {
        let auth = LlmFailure::from_http_status(401, r#"端点返回 401: {"error":"Invalid API key"}"#);
        let msg = auth.user_message();
        assert!(msg.contains("API key"), "401 应点明 key 问题：{msg}");
        assert!(!msg.contains("llm/"), "不应含内部类别码：{msg}");
        assert!(!msg.contains("Invalid API key"), "不应直出原始报文：{msg}");
        assert!(!msg.contains("端点返回 401"), "不应重复状态行：{msg}");

        let conn = LlmFailure::connection("请求 http://127.0.0.1:9/v1/chat/completions 失败: error sending request for url (http://127.0.0.1:9/v1/chat/completions)");
        let msg = conn.user_message();
        assert!(msg.contains("无法连接"), "连接失败应指路服务：{msg}");
        assert!(msg.contains("http://127.0.0.1:9/v1/chat/completions"), "应带端点（构造契约提取）：{msg}");
        assert!(!msg.contains("error sending request"), "不应直出库原文：{msg}");

        let rate = LlmFailure::from_http_status(429, "rate limited");
        assert!(rate.user_message().contains("限流"), "{}", rate.user_message());

        // 还原态（from_cordis 丢 status/endpoint）也应渲染 401 专属文案与端点：
        // 从折叠 detail 的确定性构造格式提取（提取器双测试锁）。
        let folded = auth.clone().into_cordis();
        assert!(folded.message().starts_with("llm/http_4xx: "), "机器格式应保留");
        let restored = LlmFailure::from_cordis(&folded).expect("应可还原");
        assert_eq!(restored.category, LlmFailureCategory::ClientError);
        assert!(restored.user_message().contains("API key"), "还原态 401 也应点明 key：{}", restored.user_message());
        let conn_folded = conn.clone().into_cordis();
        let conn_restored = LlmFailure::from_cordis(&conn_folded).expect("连接错误应可还原");
        assert!(
            conn_restored.user_message().contains("http://127.0.0.1:9"),
            "还原态连接错误也应带端点：{}", conn_restored.user_message()
        );
        // 提取器防漂移：非契约格式一律 None 落泛化文案，不出错不误报
        assert_eq!(extract_status("无关文本"), None);
        assert_eq!(extract_status("端点返回 abc: x"), None);
        assert_eq!(extract_status("端点返回 401: {\"e\":1}"), Some(401), "detail 带尾冒号应可提取");
        assert_eq!(extract_status("端点返回 401 Unauthorized: x"), Some(401), "StatusCode 全文形态应可提取");
        assert_eq!(extract_endpoint("请求 ftp://x 失败"), None, "非 http 端点不提取");
    }
}