//! WS 会话桥（网关落地方案 §8 WS /ws/chat；阶段 B 尾段）。
//!
//! 职责：App WebSocket ↔ agent IPC 的对话桥——App 侧用户消息经鉴权后转
//! ChatSend 投 IPC；agent 侧 ChatEvent 回注 WS（scope=chat 的令牌专属，
//! §2.4 两道闸）。server 侧 WS 升级的传输编排（axum::extract::ws）随阶段 C
//! 真机联调定型——本文件落桥接纯逻辑与事件路由（可测部分先固化）。

use yunyou_ipc_types::{ChatEvent, ChatSend, GatewayIpcFrame, SessionOpened};

/// WS 桥会话（一个 App 设备连接一条；device↔session 映射 §2.5）。
#[derive(Debug, Clone)]
pub struct WsBridge {
    /// 会话 id（session.open 惰性创建所得）。
    pub session_id: String,
    /// 设备 id（scope 校验记录）。
    pub device_id: String,
}

/// 用户消息→IPC 帧（桥接下行方向；scope=chat 已在 WS 升级时验讫）。
pub fn user_message_to_frame(bridge: &WsBridge, content: &str) -> GatewayIpcFrame {
    GatewayIpcFrame::ChatSend(ChatSend {
        session_id: bridge.session_id.clone(),
        content: content.to_string(),
    })
}

/// agent 事件→WS 文本（桥接上行方向；payload 原文透传=会话域 schema 单源，
/// 网关不解释事件内容——§2.3「网关看不到存储原始内容」的边界内透传投影）。
pub fn event_to_ws_text(frame: &GatewayIpcFrame) -> Option<String> {
    match frame {
        GatewayIpcFrame::ChatEvent(event) => Some(serde_json::to_string(event).unwrap_or_default()),
        GatewayIpcFrame::SessionOpened(SessionOpened { session_id }) => {
            // 会话开启回执转控制消息（App 侧绑定 session_id 用）
            Some(serde_json::json!({
                "type": "session.opened",
                "session_id": session_id,
            })
            .to_string())
        }
        _ => None, // 非会话事件不进 WS（ping/pong 在 IPC 层自闭环）
    }
}

/// WS 桥的 session 归属校验（§2.5 隔离：device 只收自己 session 的事件——
/// ChatEvent 的 session_id 与桥会话不符=其他设备事件，静默丢弃并计数）。
pub fn event_belongs_to(bridge: &WsBridge, event: &ChatEvent) -> bool {
    event.session_id == bridge.session_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge() -> WsBridge {
        WsBridge { session_id: "sess-1".into(), device_id: "dev-1".into() }
    }

    /// 下行桥接：用户消息→ChatSend 帧（session_id=桥会话）。
    #[test]
    fn 下行桥接帧() {
        let frame = user_message_to_frame(&bridge(), "你好");
        match frame {
            GatewayIpcFrame::ChatSend(send) => {
                assert_eq!((send.session_id.as_str(), send.content.as_str()), ("sess-1", "你好"));
            }
            other => panic!("应为 ChatSend: {other:?}"),
        }
    }

    /// 上行桥接：ChatEvent→WS 文本透传；session 不符=丢弃（设备隔离 §2.5）。
    #[test]
    fn 上行桥接与隔离() {
        let mine = ChatEvent { session_id: "sess-1".into(), seq: 1, payload: "增量".into() };
        let others = ChatEvent { session_id: "sess-其他设备".into(), seq: 2, payload: "偷看".into() };
        assert!(event_belongs_to(&bridge(), &mine), "本会话事件应放行");
        assert!(!event_belongs_to(&bridge(), &others), "他设备事件必须丢弃");
        let text = event_to_ws_text(&GatewayIpcFrame::ChatEvent(mine)).expect("本会话事件应转 WS 文本");
        assert!(text.contains("增量"));
    }

    /// 控制消息：session.opened 回执转 App 侧绑定消息。
    #[test]
    fn 控制消息回执() {
        let frame = GatewayIpcFrame::SessionOpened(SessionOpened { session_id: "s9".into() });
        let text = event_to_ws_text(&frame).expect("opened 回执应转控制消息");
        assert!(text.contains("session.opened") && text.contains("s9"), "{text}");
        // 非会话帧不进 WS
        assert!(event_to_ws_text(&GatewayIpcFrame::Ping(Ping)).is_none());
        use yunyou_ipc_types::Ping;
    }
}
