//! # yunyou-config —— yunyou.toml 配置加载与校验
//!
//! 组件内核架构方案 §2.5 的第一个组件：**纯库，无服务句柄**（不 provide 任何
//! 内核服务），对标 cordis.yml + Loader Entry 树的一期静态裁剪版。
//!
//! ## 职责
//! 1. 读取 `yunyou.toml` 文本，解析为 [`serde_json::Value`]（统一配置载体，方案 §2.2）；
//! 2. 用 [`schemars`](https://docs.rs/schemars) 从 [`model::DshConfig`] 生成
//!    JSON Schema 2020-12，经内核 [`yunyou::config::validate_schema`] 校验——
//!    错误聚合为内核同款 [`yunyou::config::ValidationError`]（多问题一次报齐）；
//! 3. 反序列化为强类型 [`model::DshConfig`]，应用环境变量覆盖后交出。
//!
//! ## 一期裁剪边界
//! - 顶层只认 5 个组件段（`llm` / `session` / `system_prompt` / `tools` /
//!   `agent_loop`），未知顶层键报校验错误（拼写防呆，fail-loud）；
//! - 各段**内部**字段不锁死（不 deny），3A 组件落地后按其 Config 扩展；
//!   段内深度校验届时由组件自身的 `config_schema` 承担（内核装载前执行）；
//! - 不支持 Loader 的嵌套组 / include / 表达式插值（方案风险 3：一期平面声明）。
//!
//! ## 安全纪律
//! `llm.api_key` 只从配置文件或环境变量进入内存；本 crate 及测试**绝不**把
//! 真实 key 写进代码、注释或 git（台账环境事实条款）。

pub mod credentials;
pub mod loader;
pub mod manifest_file;
pub mod model;

pub use loader::{load, load_from_str, load_optional, ConfigError};
pub use manifest_file::{discover_plugins, parse_manifest_toml, DiscoveredPlugin, ManifestFile, MANIFEST_FILE_NAME};
pub use credentials::CredentialsConfig;
pub use model::{
    apply_env_overrides, default_env_overrides, default_persona, DshConfig, AgentLoopConfig,
    LlmConfig, SessionConfig, SystemPromptConfig, ToolsConfig, ENV_LLM_API_KEY, ENV_LLM_BASE_URL,
    ENV_LLM_MODEL,
};
