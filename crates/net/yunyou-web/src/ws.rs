//! WS 连接会话：入站 Submit 分派 + 出站广播泵。
//!
//! # 分派规则（与 REPL 同源）
//! - 空行忽略；`/quit` = 断开本连接（任务书裁决：不经命令缝——壳 /quit 的
//!   退出令牌会杀服务殃及其他连接，web 语义里杀服务是终端 Ctrl-C 的职权）；
//! - 其余 `/` 开头 = 斜杠命令：经命令缝服务端执行（同注册表同 handler 同
//!   D2 落盘，见 commands.rs；回合闸门内串行）；
//! - 普通文本 = 回合提交（TurnRunner；并发由闸门拒绝）。
//!
//! # 出站形态
//! 每连接一个广播订阅 → mpsc 队列 → select 与入站同循环驱动；滞后收
//! Lagged 人话告警后继续跟流（丢的是积压帧，对话继续）。

use crate::protocol::{ClientMessage, ServerMessage};
use crate::server::WebState;
use axum::extract::ws::{Message, WebSocket};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

/// 单连接生命周期：订阅 → hello → 双向泵 → 收场。
pub async fn handle_socket(mut socket: WebSocket, state: Arc<WebState>) {
    // 出站泵：广播订阅 → 每连接队列（连接关即泵退）
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let mut bcast_rx = state.broadcast.subscribe();
    let pump = tokio::spawn(async move {
        loop {
            match bcast_rx.recv().await {
                Ok(msg) => {
                    if outbound_tx.send(msg).is_err() {
                        break; // 连接已关（接收端 drop）
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let notice = ServerMessage::Error {
                        text: format!("（事件流积压，已丢弃 {n} 条；刷新页面可回放会话）"),
                    };
                    if outbound_tx.send(notice).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 连接问候 + 自动回放当前会话历史尾部（§4.13 会话恢复回放；完整无损在 JSONL）
    let hello = ServerMessage::Hello { model: state.model.clone() };
    if send(&mut socket, hello).await.is_err() {
        pump.abort();
        return;
    }
    let replay = crate::sessions::history_current(&state).await;
    if send(&mut socket, replay).await.is_err() {
        pump.abort();
        return;
    }

    loop {
        tokio::select! {
            biased;
            maybe_in = socket.recv() => {
                match maybe_in {
                    Some(Ok(Message::Text(data))) => {
                        if !handle_text(&mut socket, &state, &data).await {
                            break; // /quit 或写失败 → 断开
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // Ping/Pong 底层应答；Binary 一期不认
                    Some(Err(_)) => break,
                }
            }
            maybe_out = outbound_rx.recv() => {
                match maybe_out {
                    Some(msg) => { if send(&mut socket, msg).await.is_err() { break; } }
                    None => break, // 广播通道关（服务收场）
                }
            }
        }
    }
    // 防残余泵任务悬挂（select 正常路径泵已随 rx drop 退出，abort 幂等）
    pump.abort();
}

/// 入站文本分派；返回 false=要求断开连接。
async fn handle_text(socket: &mut WebSocket, state: &Arc<WebState>, data: &str) -> bool {
    let Ok(client) = serde_json::from_str::<ClientMessage>(data) else {
        let _ = send(
            socket,
            ServerMessage::Error {
                text: "消息格式不认识（需 {\"type\":\"submit\",\"text\":…}）".into(),
            },
        )
        .await;
        return true;
    };
    // 会话面控制词汇（P4）：列表与回放（None=当前会话重取，Some(id)=按名回放）
    match client {
        ClientMessage::ListSessions => {
            let _ = send(socket, crate::sessions::list(state)).await;
            true
        }
        ClientMessage::ReplaySession { id } => {
            let msg = match id {
                None => crate::sessions::history_current(state).await,
                Some(id) => crate::sessions::history_of(state, &id).await,
            };
            let _ = send(socket, msg).await;
            true
        }
        ClientMessage::Submit { text } => handle_submit(socket, state, text).await,
    }
}

/// submit 分派（消息/斜杠命令）；返回 false=要求断开连接。
async fn handle_submit(socket: &mut WebSocket, state: &Arc<WebState>, text: String) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    // 斜杠分派（strip_prefix 天然字符边界安全——对任意输入切片 [1..] 会在
    // 多字节首字符上 panic，真机测试抓过）
    if let Some(body) = trimmed.strip_prefix('/') {
        // token 解析与 command_exec 同规则：去斜杠后取首空白段
        let token = body.split_whitespace().next().unwrap_or("");
        if token == "quit" {
            let _ = send(
                socket,
                ServerMessage::Bye { text: "已断开连接（服务仍在运行；刷新页面可重连）".into() },
            )
            .await;
            return false;
        }
        // 命令缝服务端执行（P3）：与 REPL 同注册表同 handler 同 D2 落盘；
        // 闸门内串行（/new 重建装配不得与进行中回合并发）
        if let Some(msg) = crate::commands::execute(state, trimmed).await {
            let _ = send(socket, msg).await;
        }
        return true;
    }
    // 消息回合：spawn 跑（连接保持收流；并发提交由回合闸门拒绝）
    let turns = state.turns.clone();
    let sender = state.broadcast.clone();
    let text = trimmed.to_string();
    tokio::spawn(async move {
        if let Err(err) = turns.run(&text, sender.clone()).await {
            let _ = sender.send(ServerMessage::Error { text: err });
        }
    });
    true
}

/// 发一条 JSON 帧（序列化失败=协议 bug，降级为错误帧而非静默丢）。
async fn send(socket: &mut WebSocket, msg: ServerMessage) -> Result<(), axum::Error> {
    let json = serde_json::to_string(&msg)
        .unwrap_or_else(|err| format!("{{\"type\":\"error\",\"text\":\"协议序列化失败：{err}\"}}"));
    socket.send(Message::text(json)).await
}
