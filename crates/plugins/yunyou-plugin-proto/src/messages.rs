//! 插件协议消息族单源（L2 外部装载缝设计 §3.2，K-19 刀1）。
//!
//! **暴露面铁律（复用 ipc-types 纪律）**：消息族封闭如下——MVP 词汇八变体
//! （hello/hello_ok/hello_err/tools_list/tools_list_ok/tool_invoke/tool_result/
//! shutdown）；新增消息类型=L2 装载缝设计修订（评审线一），实现时私加被
//! [`PluginProtoFrame`] 封闭枚举在类型层拒绝（未列变体无法构造帧）。
//!
//! 词汇演进预留（L2.1 才开闸，设计期只占位不实现——设计 §3.2 原文）：
//! `tools_changed`（热更新清单）/`cancel`（AbortSignal 贯穿）/`credentials_get`
//! （§4.2 凭证面）。本枚举**不含**这些变体——开闸=设计修订+枚举加变体（加法
//! 演进随 serde default 律，旧插件端未知新标签=拒绝=fail-loud 可见）。
//!
//! 全类型 serde+schemars 双派生=JSON Schema 唯一真源（与 ipc-types 同款）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 协议版本（握手比对；不兼容=hello_err fail-loud，设计 §3.2——PROTOCOL_VERSION
/// 模式与 ipc-types 同源）。
pub const PROTOCOL_VERSION: u32 = 1;

/// 宿主→插件握手帧（§3.2）：协议版本+能力集。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hello {
    /// 协议版本（[`PROTOCOL_VERSION`]）。
    pub protocol_version: u32,
    /// 宿主能力集（MVP=空集占位，加法演进钩子——L2.1 起按声明放行）。
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// 插件→宿主握手成功回执（版本回显）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloOk {
    /// 插件侧协议版本回显（与宿主一致方可继续）。
    pub protocol_version: u32,
    /// 插件自报名（诊断与人话面用；非鉴权字段）。
    #[serde(default)]
    pub plugin_name: String,
}

/// 插件→宿主握手拒绝（版本不符→装配 Failed，不静默降级）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloErr {
    /// 中文拒绝原因（人话直出，设计 §3.2）。
    pub reason: String,
}

/// 插件上报的工具清单条目（tools_list_ok 载荷；模型面只见三元组——
/// tools.v1 白名单纪律：timeout_ms 等零泄漏进模型面，设计 §3.3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolDescriptor {
    /// 工具名（代理注册到 ToolsRegistry 的注册名）。
    pub name: String,
    /// 人话描述（模型面）。
    pub description: String,
    /// 参数 JSON Schema（模型面）。
    pub parameters: String,
    /// 输出 JSON Schema（可选；加法演进律随 tools.v1）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
    /// 单次调用超时毫秒（缺省=宿主 manifest 的 request_timeout_ms 兜底）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// 并发安全标记（缺省 false=串行调度；代理调度面用，模型面不泄漏）。
    #[serde(default)]
    pub is_concurrency_safe: bool,
}

/// 宿主→插件：索取工具清单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolsList;

/// 插件→宿主：工具清单上报。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolsListOk {
    /// 工具清单（可为空=插件无工具；空清单合法非错误）。
    pub tools: Vec<ToolDescriptor>,
}

/// 宿主→插件：工具调用（invoke_id 关联 tool_result）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolInvoke {
    /// 调用关联 id（宿主生成；tool_result 回带——并发调用的关联键）。
    pub invoke_id: String,
    /// 工具名（须在 tools_list 上报过=白名单；未上报名=插件端拒绝）。
    pub name: String,
    /// 参数 JSON（模型面透传；schema 校验在插件端）。
    pub args: String,
}

/// 工具调用成功结果载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResultOk {
    /// 返回值 JSON（tools.v1 ToolOutcome::Ok 的 value 投影）。
    pub value: String,
}

/// 工具调用失败结果载荷（tools.v1 ToolError 三元组投影，错误码语义同源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResultErr {
    /// 错误码（tools.v1 错误码语义同源，如 "team/args"）。
    pub code: String,
    /// 人话错误消息。
    pub message: String,
    /// 修复提示（加法演进律随 tools.v1）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// 工具调用结果（outcome 二选一；tools.v1 ToolOutcome 投影）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    /// 成功（value JSON）。
    Ok(ToolResultOk),
    /// 失败（code/message/hint 三元组）。
    Err(ToolResultErr),
}

/// 插件→宿主：工具调用结果（invoke_id 关联）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResult {
    /// 调用关联 id（与 [`ToolInvoke::invoke_id`] 配对）。
    pub invoke_id: String,
    /// 结果（ok/err 二选一）。
    pub outcome: ToolOutcome,
}

/// 宿主→插件：卸载/停用通知（插件回执后宿主 kill 兜底——不回执也杀，
/// 不悬挂；设计 §3.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Shutdown;

/// 插件→宿主：会话消息投递请求（v1.1 加法演进，scheduler.v1 设计 §五）：
/// 到期触发器把注入消息交宿主投给会话（回合边界排队投递，与任务池信箱同律）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeliverMessage {
    /// 投递关联 id（插件铸造；DeliverAck 回带——等待回执的关联键）。
    pub deliver_id: String,
    /// 目标会话 id（None=当前会话/全局；宿主侧按会话路由）。
    #[serde(default)]
    pub session_id: Option<String>,
    /// 注入消息全文（用户角色；`[定时触发 <id>]` 前缀由宿主统一加，防伪）。
    pub text: String,
}

/// 宿主→插件：投递回执（ok=已入会话投递面；false+人话 detail=失败原因——
/// 插件侧按设计 §五 miss 策略处置：间隔型顺延一轮，At 型补送一次后销）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeliverAck {
    /// 与 [`DeliverMessage::deliver_id`] 配对。
    pub deliver_id: String,
    /// 投递结果。
    pub ok: bool,
    /// 人话详情（失败原因/成功路由面）。
    #[serde(default)]
    pub detail: String,
}

/// 插件协议封闭帧（§3.2 MVP 词汇；新增变体=设计修订评审线一）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginProtoFrame {
    /// 宿主→插件握手。
    Hello(Hello),
    /// 插件→宿主握手成功。
    HelloOk(HelloOk),
    /// 插件→宿主握手拒绝。
    HelloErr(HelloErr),
    /// 宿主→插件索取工具清单。
    ToolsList(ToolsList),
    /// 插件→宿主工具清单上报。
    ToolsListOk(ToolsListOk),
    /// 宿主→插件工具调用。
    ToolInvoke(ToolInvoke),
    /// 插件→宿主工具结果。
    ToolResult(ToolResult),
    /// 宿主→插件卸载通知。
    Shutdown(Shutdown),
    /// 插件→宿主会话消息投递请求（v1.1；旧宿主端=未知标签拒收 fail-loud 可见）。
    DeliverMessage(DeliverMessage),
    /// 宿主→插件投递回执（v1.1）。
    DeliverAck(DeliverAck),
}
