//! # yunyou-gateway —— 网关（K-12 阶段 A：纯逻辑+存储层，无网络）
//!
//! 网关落地方案 §9 阶段 A 范围：两段式令牌（[`token`]）+配对双式纯逻辑
//! （[`pairing`]：扫码/验证码，原子一次性消费+失败锁定）+gateway.db 存储
//! （[`store`]）+审计（[`audit`]，认证失败必留痕不可关闭）+scope 边界
//! （[`scope`]）。**无网络**：http/ws/ipc-client 归阶段 B（axum/tower 依赖
//! 随 B 批引入）。
//!
//! ## 依赖纪律（方案 §9.1）
//!
//! `yunyou-gateway → yunyou-ipc-types + yunyou-util + rusqlite + getrandom +
//! sha2（§3.1 SHA-256 存储哈希的方案内隐含件）`；**不依赖 yunyou 内核**
//! （deps-gate 黑名单，落库批接 deny 条目）。
//!
//! ## 安全纪律锚（方案 §10.1 逐条对应）
//!
//! 哈希存储（§3.1：SHA-256(token)，库泄露不泄可用令牌）｜原子一次性防重放
//! （§3.1 UPDATE 影响行数判定）｜失败锁定双维（§3.2：配对会话 5 次→锁 15
//! 分钟）｜设备数上限（默认 10）｜吊销立即生效（generation）｜审计必留痕
//! （§3.3）。测试逐条锁（模块内单测+集成随阶段 C）。

pub mod audit;
pub mod http;
pub mod ipc_client;
pub mod config;
pub mod middleware;
pub mod pairing;
pub mod scope;
pub mod store;
pub mod ws;
pub mod token;

pub use ipc_client::{AgentUnreachable, IpcClientConfig, IpcConnection};
pub use middleware::{AuthError, AuthMiddleware, AuthedDevice};
pub use pairing::{ConsumeError, NetworkDimension, PairingIssue, PairingMethod};
pub use scope::Scope;
pub use store::{GatewayStore, IpLockState};
pub use token::{b64url_encode, hash_plaintext, new_pairing_plaintext, new_session_plaintext, sha256_hex};
