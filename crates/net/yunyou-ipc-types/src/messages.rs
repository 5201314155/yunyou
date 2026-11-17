//! IPC 消息族单源（网关落地方案 §2.3，业主 D1 裁决一期范围）。
//!
//! **暴露面铁律（业主硬约束③）**：消息族封闭如下——不存在任务图直写、状态
//! 流转、花名册管理消息；新增消息类型=网关落地方案修订（评审线一），实现时
//! 私加被 [`GatewayIpcFrame`] 的封闭枚举在类型层拒绝（未列变体无法构造帧）。
//!
//! 全类型 serde+schemars 双派生=JSON Schema 2020-12 生成唯一真源（方案 §8：
//! docs/接口/schema/ 四件套纪律）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 协议版本（握手比对；不兼容=hello_err，方案 §2.2）。
pub const PROTOCOL_VERSION: u32 = 1;

/// IPC 凭证与套接字路径约定（网关落地方案 §2.1/§2.2；相对 yunyou-home）。
pub mod paths {
    /// agent 侧 IPC 套接字（`run/` 目录 0700、socket 0600；已存在先安全删除防残留劫持）。
    pub const GATEWAY_SOCKET: &str = "run/gateway.sock";
    /// agent 启动生成的 32B 随机 secret（0600；每次重启轮换，网关重连重读）。
    pub const GATEWAY_IPC_SECRET: &str = "run/gateway-ipc-secret";
    /// `run/` 目录权限（八进制）。
    pub const RUN_DIR_MODE: u32 = 0o700;
    /// socket 与 secret 文件权限（八进制）。
    pub const SECRET_FILE_MODE: u32 = 0o600;
    /// 连续认证失败上限（超过=agent 删 socket 告警 fail-loud，防暴力试 secret）。
    pub const AUTH_FAIL_LIMIT: u32 = 3;
    /// 心跳间隔（毫秒；连续 3 次超时断开）。
    pub const PING_INTERVAL_MS: u64 = 30_000;
    /// 心跳超时上限倍数。
    pub const PING_TIMEOUT_TICKS: u32 = 3;
}

/// 网关→agent 握手帧（方案 §2.2）：secret 的 SHA-256（不传明文）+协议版本；
/// `resume` 携带断线重连信息（§2.6：seq 回放补发）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hello {
    /// IPC secret 的 SHA-256 十六进制（明文不过线）。
    pub secret_hash: String,
    /// 协议版本（[`PROTOCOL_VERSION`]）。
    pub protocol_version: u32,
    /// 断线重连信息（首连=None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeInfo>,
}

/// 重连补发请求（§2.6）：从 `last_seq` 之后按序回放该会话事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResumeInfo {
    /// 请求续聊的会话（device↔session 映射，方案 §2.5）。
    pub session_id: String,
    /// 已消费的最后事件序号（回放 `seq > last_seq`）。
    pub last_seq: u64,
}

/// agent→网关 握手确认。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloOk {
    /// 协议版本回显。
    pub protocol_version: u32,
}

/// agent→网关 握手拒绝（附中文原因；连续 3 次失败 agent 删 socket fail-loud）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloErr {
    /// 拒绝原因（人话，审计同步留痕）。
    pub reason: String,
}

/// 心跳（双向，30s 间隔；连续 3 次超时断开）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Ping;

/// 心跳应答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Pong;

/// 会话开启（网关→agent）：device↔session 惰性创建（方案 §2.5——同设备重连
/// 映射同一 session 续聊；多设备互相隔离）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionOpen {
    /// 设备唯一标识（配对时签发）。
    pub device_id: String,
    /// 设备人读名（审计与面板观察用）。
    pub device_name: String,
}

/// 会话开启应答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionOpened {
    /// 惰性创建（或复用）的会话 id。
    pub session_id: String,
}

/// 用户消息投递（网关→agent）：App 侧用户输入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatSend {
    /// 目标会话（scope=chat 的设备只能投自己的会话，方案 §2.4 两道闸）。
    pub session_id: String,
    /// 用户输入原文。
    pub content: String,
}

/// 会话事件（agent→网关）：流式增量/工具事件等，`seq` 单调递增源自会话
/// JSONL 持久化序号（断连回放的排序键，§2.6）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatEvent {
    /// 会话 id。
    pub session_id: String,
    /// 单调递增事件序号（回放补发键）。
    pub seq: u64,
    /// 事件 JSON（会话事件存储的透传投影；结构归会话域 schema）。
    pub payload: String,
}

/// 面板任务只读投影请求（scope=panel:read；agent 侧编排层生成投影，
/// 网关看不到存储原始内容——方案 §2.3 铁律）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PanelTasksRead;

/// 面板命令入队请求（scope=panel:write；等价 admin 入队路径，不直写任务图）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PanelCommandSubmit {
    /// 命令名（与命令注册缝词汇一致）。
    pub command: String,
    /// 命令参数（JSON 文本）。
    pub arguments: String,
}

/// **IPC 帧封闭枚举**（暴露面铁律的类型化执行）：网关↔agent 之间只存在
/// 上述消息——封闭性由类型系统强制，私加消息类型=编译失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayIpcFrame {
    /// 握手请求。
    Hello(Hello),
    /// 握手确认。
    HelloOk(HelloOk),
    /// 握手拒绝。
    HelloErr(HelloErr),
    /// 心跳。
    Ping(Ping),
    /// 心跳应答。
    Pong(Pong),
    /// 会话开启请求。
    SessionOpen(SessionOpen),
    /// 会话开启应答。
    SessionOpened(SessionOpened),
    /// 用户消息投递。
    ChatSend(ChatSend),
    /// 会话事件。
    ChatEvent(ChatEvent),
    /// 面板任务只读请求。
    PanelTasksRead(PanelTasksRead),
    /// 面板命令入队。
    PanelCommandSubmit(PanelCommandSubmit),
}
