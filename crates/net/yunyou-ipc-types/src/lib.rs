//! # yunyou-ipc-types —— 网关↔大 agent IPC 消息类型单源（K-12 阶段 A）
//!
//! 网关落地方案 §2.3/§9.1：协议契约 crate（类比 yunyou-llm 的 Service
//! Definition），**无层属性零业务依赖**——yunyou-cli（agent 侧 ipc-server）
//! 与 yunyou-gateway（网关侧 ipc-client）共同依赖，双端类型单源防漂移。
//!
//! ## 内容
//!
//! - [`messages`]：消息族封闭枚举（暴露面铁律的类型化执行——私加消息类型
//!   =编译失败，方案 §2.3「不允许实现时私加」的落地形态）；
//! - [`paths`]：套接字/凭证路径约定与权限常量（§2.1/§2.2，双端共用防漂移）；
//! - 帧格式约定：JSON Lines（每行一消息 `\n` 分隔，方案 §2.1——可调试、
//!   与 JSON Schema 纪律同源、一期吞吐要求低）。
//!
//! ## 依赖纪律
//!
//! 本 crate → serde/schemars（类型派生）+yunyou-util（零业务依赖上界，
//! 方案 §9.1 依赖规则原文）；**禁止依赖 yunyou 内核**（无层属性）。

pub mod messages;

pub use messages::{
    ChatEvent, ChatSend, GatewayIpcFrame, Hello, HelloErr, HelloOk, PanelCommandSubmit,
    PanelTasksRead, Ping, Pong, ResumeInfo, SessionOpen, SessionOpened, PROTOCOL_VERSION,
};

/// 帧分隔符（JSON Lines 约定，方案 §2.1）。
pub const FRAME_SEPARATOR: u8 = b'\n';

/// JSON Lines 编码：帧序列化为单行（不含分隔符；调用方追加 [`FRAME_SEPARATOR`]）。
///
/// 序列化失败在类型层不可能（全字段 String/u64/Option——serde  Total 编码），
/// 仍保留 Result 形态与 serde 语义对齐（fail-loud 不吞错）。
pub fn encode_frame(frame: &GatewayIpcFrame) -> serde_json::Result<String> {
    serde_json::to_string(frame)
}

/// JSON Lines 解码：单行（不含分隔符）反序列化为帧。
///
/// 未知 `type` 标签=Err（协议版本/实现私加的类型在此拒绝——封闭枚举的
/// 反序列化端同样执行暴露面铁律）。
pub fn decode_frame(line: &str) -> serde_json::Result<GatewayIpcFrame> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 帧往返：全变体 serde 往返无损（二期 schema 生成的最低承诺）。
    #[test]
    fn 帧往返无损() {
        let frames = vec![
            GatewayIpcFrame::Hello(Hello {
                secret_hash: "ab12".into(),
                protocol_version: PROTOCOL_VERSION,
                resume: Some(ResumeInfo { session_id: "s1".into(), last_seq: 7 }),
            }),
            GatewayIpcFrame::HelloOk(HelloOk { protocol_version: PROTOCOL_VERSION }),
            GatewayIpcFrame::HelloErr(HelloErr { reason: "认证失败".into() }),
            GatewayIpcFrame::Ping(Ping),
            GatewayIpcFrame::Pong(Pong),
            GatewayIpcFrame::SessionOpen(SessionOpen {
                device_id: "d1".into(),
                device_name: "业主手机".into(),
            }),
            GatewayIpcFrame::SessionOpened(SessionOpened { session_id: "s2".into() }),
            GatewayIpcFrame::ChatSend(ChatSend { session_id: "s2".into(), content: "你好".into() }),
            GatewayIpcFrame::ChatEvent(ChatEvent {
                session_id: "s2".into(),
                seq: 42,
                payload: r#"{"delta":"文"}"#.into(),
            }),
            GatewayIpcFrame::PanelTasksRead(PanelTasksRead),
            GatewayIpcFrame::PanelCommandSubmit(PanelCommandSubmit {
                command: "review".into(),
                arguments: "{}".into(),
            }),
        ];
        for frame in frames {
            let line = encode_frame(&frame).expect("编码应成功");
            assert!(!line.contains('\n'), "单行帧不含分隔符");
            assert_eq!(decode_frame(&line).expect("解码应成功"), frame, "往返无损: {line}");
        }
    }

    /// 暴露面铁律（类型化执行）：未知 type 标签=解码拒绝（实现私加的消息类型
    /// 在双端都过不了封闭枚举——方案 §2.3 铁律的 serde 执行）。
    #[test]
    fn 未知消息类型解码拒绝() {
        let line = r#"{"type":"panel.tasks.write","payload":"越权直写"}"#;
        assert!(decode_frame(line).is_err(), "未列消息类型必须拒绝");
    }

    /// tagged 枚举的线上形态：type 标签=snake_case 变体名（跨端契约可读）。
    #[test]
    fn 线上形态带类型标签() {
        let line = encode_frame(&GatewayIpcFrame::Ping(Ping)).expect("编码应成功");
        assert!(line.contains(r#""type":"ping""#), "{line}");
    }
}
