//! # yunyou-plugin-sdk：云悠插件开发 SDK（插件作者唯一依赖）
//!
//! 插件作者**零依赖内核源码**：声明工具+一个 `main`，编译产物=独立可执行，
//! 被云悠内核 spawn 为子进程、经 stdin/stdout JSON Lines 协议对话（握手→
//! 工具清单→工具调用→shutdown）。SDK 只消费 [`yunyou_plugin_proto`]（契约
//! 冻结面）——CI 断言锁死依赖面（`tests/sdk_zero_kernel_dep.rs`）。
//!
//! ## 15 分钟上手（最小插件全文）
//!
//! ```no_run
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//! use serde_json::json;
//! use yunyou_plugin_sdk::{tool, Plugin};
//!
//! #[derive(Deserialize, JsonSchema)]
//! struct EchoArgs {
//!     /// 要回显的文本
//!     text: String,
//! }
//!
//! fn main() -> std::process::ExitCode {
//!     Plugin::new("echo", "0.1.0")
//!         .description("回显文本的示例插件")
//!         .category("tools")
//!         .tool(tool("echo", "原样返回输入文本", |args: EchoArgs| {
//!             Ok(json!({ "echoed": args.text }))
//!         }))
//!         .serve()
//! }
//! ```
//!
//! ## 进程生命周期（serve 的帧序，镜像宿主 ProxyPlugin）
//!
//! ```text
//! 宿主 ──hello(协议版本)──▶ 插件
//!      ◀─hello_ok（或 hello_err=版本不符 exit(1)，fail-loud 不降级）
//!      ──tools_list──▶
//!      ◀─tools_list_ok（工具清单；parameters schema 自动派生自 Args 类型）
//!      ──tool_invoke(invoke_id, name, args)──▶
//!      ◀─tool_result(invoke_id, ok | err{code,message,hint})
//!      ──shutdown──▶ 插件 exit(0)（宿主 killpg 兜底，不悬挂）
//! stdin EOF（宿主退出）＝插件退出
//! ```
//!
//! 纪律：诊断一律走 **stderr**（宿主逐行透传主板日志）；stdout 是协议专用
//! 通道，禁打印任何非协议内容。MVP 能力边界（协议词汇未开闸=不可用）：
//! 无凭据访问、无网络放行、无热更新、无取消语义（单请求靠宿主超时硬断）。

use std::sync::Arc;

pub use yunyou_plugin_proto::{ToolDescriptor, PROTOCOL_VERSION};

/// 定时触发缝（scheduler.v1 S1+S2：注册/注销/内存调度循环，At/Interval/Cron 三型；
/// 会话投递缝与持久化=S4/proto v1.1，设计 §九）。
pub mod scheduler;
pub mod serve;

/// 工具失败（tools.v1 ToolError 三元组投影，错误码语义同源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure {
    /// 错误码（如 "tool/invalid-args"、"tool/exec"）。
    pub code: String,
    /// 人话错误消息。
    pub message: String,
    /// 修复提示（可选；模型面可读，写清怎么改参能成功）。
    pub hint: Option<String>,
}

impl ToolFailure {
    /// 便捷构造（无 hint）。
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), hint: None }
    }

    /// 带 hint 构造。
    pub fn with_hint(
        code: impl Into<String>,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self { code: code.into(), message: message.into(), hint: Some(hint.into()) }
    }
}

/// 工具契约（插件作者实现面；裸形态——完全掌控 schema 时用）。
///
/// 便捷路径：多数场景用 [`tool`] 便捷函数（强类型闭包+schema 自动派生）。
pub trait Tool: Send + Sync + 'static {
    /// 工具名（模型面调用名；snake_case 惯例）。
    fn name(&self) -> &str;
    /// 给 LLM 看的一句话描述（模型面）。
    fn description(&self) -> &str;
    /// 参数 JSON Schema（模型面；LLM 照此填参）。
    fn parameters(&self) -> String;
    /// 输出 JSON Schema（可选；加法演进律随 tools.v1）。
    fn output_schema(&self) -> Option<String> {
        None
    }
    /// 单次调用超时毫秒声明（缺省 None=宿主 manifest 的 request_timeout_ms 兜底）。
    fn timeout_ms(&self) -> Option<u64> {
        None
    }
    /// 并发安全声明（缺省 false=宿主串行调度）。
    fn is_concurrency_safe(&self) -> bool {
        false
    }
    /// 执行入口（同步 MVP——lead 裁决 2026-11-18；async=契约加法演进留二期版本位）。
    fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolFailure>;
}

/// 强类型工具便捷构造（推荐形态）：闭包参数类型须 `Deserialize + JsonSchema`
/// ——参数 schema 自动派生进模型面，LLM 照 schema 填参，SDK 反序列化后交闭包。
///
/// ```no_run
/// # use schemars::JsonSchema; use serde::Deserialize; use serde_json::json;
/// # use yunyou_plugin_sdk::tool;
/// #[derive(Deserialize, JsonSchema)]
/// struct HelloArgs {
///     /// 对方名字
///     name: String,
/// }
/// let t = tool("hello", "问好", |args: HelloArgs| Ok(json!({"greeting": args.name})));
/// ```
pub fn tool<A, F>(name: &str, description: &str, handler: F) -> Arc<dyn Tool>
where
    A: serde::de::DeserializeOwned + schemars::JsonSchema + 'static,
    F: Fn(A) -> Result<serde_json::Value, ToolFailure> + Send + Sync + 'static,
{
    Arc::new(TypedTool {
        name: name.to_string(),
        description: description.to_string(),
        handler,
        _marker: std::marker::PhantomData,
    })
}

struct TypedTool<A, F> {
    name: String,
    description: String,
    handler: F,
    _marker: std::marker::PhantomData<fn() -> A>,
}

impl<A, F> Tool for TypedTool<A, F>
where
    A: serde::de::DeserializeOwned + schemars::JsonSchema + 'static,
    F: Fn(A) -> Result<serde_json::Value, ToolFailure> + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> String {
        // schemars 派生参数 schema（模型面唯一参数说明来源；字段 doc-comment
        // 即字段描述——给 LLM 的填参说明写在 Args 结构体字段注释上）
        serde_json::to_string(&schemars::schema_for!(A))
            .unwrap_or_else(|_| r#"{"type":"object"}"#.to_string())
    }

    fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolFailure> {
        let typed: A = serde_json::from_value(args).map_err(|err| {
            ToolFailure::with_hint(
                "tool/invalid-args",
                format!("参数不符合 schema：{err}"),
                "对照工具 parameters 的 JSON Schema 修正参数后重试",
            )
        })?;
        (self.handler)(typed)
    }
}

/// 插件声明（构建器）：id/version 元数据 + 工具集；`.serve()` 进协议服务。
pub struct Plugin {
    id: String,
    version: String,
    category: Option<String>,
    description: Option<String>,
    tools: Vec<Arc<dyn Tool>>,
    scheduler: Option<Arc<crate::scheduler::Scheduler>>,
}

impl Plugin {
    /// 构建器入口：`id`=全局唯一插件标识（与 manifest.toml 的 `id` 一致），
    /// `version`=semver 字符串。
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self { id: id.into(), version: version.into(), category: None, description: None, tools: Vec::new(), scheduler: None }
    }

    /// 分类目录约定（install 自动归类落 `plugins/<类>/<id>/`；与 manifest.toml
    /// 的 `category` 一致）。
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// 人话描述（诊断/落盘 manifest 注释面；非模型面）。
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// 注册一个工具（多次调用累积；推荐 [`tool`] 便捷构造）。
    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    /// 挂接定时调度器（scheduler.v1；serve 时桥接投递通道——到期消息经
    /// DeliverMessage 投给宿主会话，v1.1 起真投递）。
    pub fn scheduler(mut self, scheduler: Arc<crate::scheduler::Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    /// 插件标识（诊断面用）。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 进入协议服务循环（阻塞至 shutdown/宿主退出；帧序见模块文档）。
    pub fn serve(mut self) -> std::process::ExitCode {
        let scheduler = self.scheduler.take();
        serve::serve(self, scheduler)
    }

    // ---- 以下为 serve 内部消费面（crate 内可见）----

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn category_ref(&self) -> Option<&str> {
        self.category.as_deref()
    }

    pub(crate) fn tools(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }

    pub(crate) fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools
            .iter()
            .map(|t| ToolDescriptor {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
                output_schema: t.output_schema(),
                timeout_ms: t.timeout_ms(),
                is_concurrency_safe: t.is_concurrency_safe(),
            })
            .collect()
    }

    pub(crate) fn find_tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }
}
