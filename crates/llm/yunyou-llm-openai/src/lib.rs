//! OpenAI 兼容 LLM Provider（`yunyou-llm` 契约的实现，方案 §2.5）。
//!
//! # 实测端点容忍清单（来自台账环境事实的实测记录，回归见 wire.rs 单测）
//! - 推理模型的 `delta.reasoning_content` 字段：原样透传为
//!   [`StreamDelta::Reasoning`]，**不回传**给端点（消息体不含思考内容）；
//! - `delta.content` 为 `null`（纯工具调用分片 / 纯 reasoning 分片常见）：
//!   视为无文本增量，不产生空串 Text；
//! - `choices` 为空数组（仅 usage 的终止块）：跳过 choices，正常产出 Done；
//! - `usage` 缺失：Done 的 usage 为 None；
//! - 未知字段一律忽略（serde 默认），[DONE] 哨兵行结束流。
//!
//! # 配置
//! `base_url` / `api_key` / `model` 全部走插件配置（schemars 派生 schema），
//! 绝不硬编码任何密钥；api_key 建议经环境注入后由装配层写入配置。

mod plugin;
mod request;
mod stream;
mod wire;

pub use plugin::{OpenAiConfig, OpenAiPlugin};

/// 构造 Provider 侧错误的统一入口（`ErrorKind::Internal` + `llm/` 前缀分类，
/// 一期不对内核 ErrorKind 扩容，见 yunyou-llm lib 文档）。
pub(crate) fn llm_error(code: &str, message: impl Into<String>) -> yunyou::CordisError {
    yunyou::CordisError::new(
        yunyou::ErrorKind::Internal,
        format!("llm/{code}: {}", message.into()),
    )
}
