//! 插件协议类型单源（K-19 刀1，L2 外部装载缝设计 §3.2/§3.3）。
//!
//! **crate 定位**：宿主↔插件子进程双端共用防漂移；无层属性零业务依赖
//! （依赖上界 serde/schemars+yunyou-util，与 ipc-types 同款禁依赖内核）。
//!
//! **纪律复用声明（复用做法不复用类型）**：封闭枚举+tag type（snake_case）+
//! JSON Lines（`\n` 分隔）+版本握手（不兼容=hello_err fail-loud）+未知 type
//! 标签=解码拒绝（暴露面铁律的类型化执行）。不复用 [`yunyou-ipc-types`] 的
//! GatewayIpcFrame：域不同（网关↔agent vs 宿主↔插件）、消费者不同、演进律
//! 独立（混入=两域暴露面互相牵连，设计 §3.3 原文）。
//!
//! **消费面**：宿主侧=Runner（刀2，云愈线）；插件侧=任意语言协议端
//! （Python/Node 脚本按 JSON Lines 语义实现）。

pub mod codec;
pub mod messages;

pub use codec::{decode_frame, encode_frame, FRAME_SEPARATOR};
pub use messages::{
    DeliverAck, DeliverMessage, Hello, HelloErr, HelloOk, PluginProtoFrame, Shutdown,
    ToolDescriptor, ToolInvoke, ToolOutcome, ToolResult, ToolResultErr, ToolResultOk, ToolsList,
    ToolsListOk, PROTOCOL_VERSION,
};
