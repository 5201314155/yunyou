//! # yunyou-web —— `yunyou web` 浏览器启动形态（云幕席位，业主令任务书）
//!
//! ## 形态定位（主板论边界）
//! v1 web 面是 cli 的**并列前端**：同一装配（run.rs 的 §2.5 序列原样）、同一
//! 会话（SessionStore 唯一事实源）、同一 agent-loop（`dyn AgentLoop` 服务）。
//! 本 crate 只是把「渲染端」从终端换成浏览器：TurnObserver 事件流经 WS 推给
//! 页面逐字渲染，输入经 WS 走同一 agent-loop——不改装配序列、不加内核机制。
//!
//! ## 内核 UiEntry page{} 缝的衔接注记（一期拒装，二期插件化，不实现）
//! 官方 DeepSeek Harness 的 web 端在 manifest 的 UiEntry `page{}` 缝挂页面，
//! 由宿主按 manifest 装载；yunyou 内核已预留该缝（`yunyou::manifest_ui::UiEntry`，
//! 一期冻结不消费）。二期 web 面插件化的落点：把本 crate 的静态资产路由与 WS
//! 端点迁到 UiEntry 声明的页面目录，由内核按 manifest 装载——本 crate 的
//! [`server`] 路由表保持函数粒度无状态，即届时的可搬运缝位。
//!
//! ## 依赖纪律
//! - 后端仅 axum（HTTP+WS 一体，tokio 系，lead 依赖预批）；前端零 node 构建
//!   链——vanilla JS/CSS 按职责拆文件，`include_str!` 嵌入二进制；
//! - 样式值由 build.rs 从 docs/设计/tokens.json 生成（与 yunyou-cli 的 palette
//!   同机制；按 adapters.web 命名规则映射 CSS 变量），Web 端不私设色值/符号；
//! - 安全（网关 D7）：默认 127.0.0.1 + 随机 token，页面与 WS 握手同过鉴权，
//!   无匿名通道；LAN 暴露需显式 --addr 且 token 恒开。
//!
//! ## 依赖方向（防循环）
//! yunyou-cli（接线侧）→ yunyou-web → {yunyou, yunyou-agent-loop, yunyou-commands,
//! yunyou-session}；**yunyou-web 不依赖 yunyou-cli**——当前会话 root 经
//! [`WebParams::root_getter`] 回调注入（run.rs 从会话槽取），/new 换槽后自动取新装配。

mod bridge;
mod commands;
mod config_store;
mod rpc_credentials;
mod rpc_settings;
mod rpc_plugins;
mod rpc_system;
mod contract;
mod dist;
mod error;
mod gateway;
mod instance;
mod observer;
mod protocol;
mod rpc;
mod rpc_normalize;
mod rpc_session;
mod rpc_capabilities;
mod rpc_commands;
mod rpc_schedule;
mod rpc_skills;
mod rpc_team;
mod schedule_fire;
mod server;
mod sessions;
mod token;
mod turns;
mod wire;
mod wire_frames;
mod wire_session;
mod ws;

pub use error::WebError;
pub use instance::{serve, start_instance, WebInstance, WebParams};
