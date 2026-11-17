//! v2 上游前端适配网关（任务40 适配层）：`/api/remote.mux` 多路复用流通道。
//!
//! # 线协议（api/gateway stream-protocol，JSON 文本帧）
//! 上行：`{type:'open',streamId,endpoint,payload}` / `{type:'item',streamId,value}`
//! / `{type:'end',streamId}` / `{type:'cancel',streamId}`；
//! 下行：`{type:'item',streamId,value}` / `{type:'end',streamId}`
//! / `{type:'error',streamId,error:{code,message,details}}`。
//!
//! # 本片范围（适配层第二片「对话事件桥」）
//! 握手+帧分派+**follow live**：`session/follow` 打开即回快照（重放环+
//! assistantStream 基线）并订阅 wire 桥——回合事件实时推流；`session/control`
//! 空投影基线、`$events` ready（waterfall 远期）；未识别端点 fail-visible。
//! unary RPC 面拆至 [`crate::rpc`]（通道/RPC 职责分家，行数纪律）。

use crate::server::WebState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use serde_json::{json, Value};
use std::sync::Arc;

/// mux 升级入口（鉴权中间件先行；对端 client 连接点）。
pub(crate) async fn mux_upgrade(
    State(state): State<Arc<WebState>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| handle_mux(socket, state))
}

/// 单连接生命周期：上行帧分派 ∥ wire 桥下行推流（select 双路——follow 流
/// 的 live 事件与上行 open 并发到达，不互堵）。
async fn handle_mux(mut socket: WebSocket, state: Arc<WebState>) {
    let mut follows: Vec<(Value, crate::bridge::StreamKind)> = Vec::new();
    let mut bridge = state.wire.subscribe();
    loop {
        tokio::select! {
            uplink = socket.recv() => {
                let Some(Ok(Message::Text(data))) = uplink else {
                    break; // 关闭/非文本/错误 → 连接收场
                };
                let Ok(uplink) = serde_json::from_str::<Value>(&data) else {
                    let _ = send(
                        &mut socket,
                        error_frame(Value::String("?".into()), "gateway/bad-frame", "帧不是合法 JSON 对象"),
                    )
                    .await;
                    continue;
                };
                match uplink.get("type").and_then(Value::as_str) {
                    Some("open") => dispatch_open(&mut socket, state.clone(), &uplink, &mut follows).await,
                    // 本片无上行数据流消费方：item/end/cancel 即刻忽略
                    Some("item") | Some("end") | Some("cancel") => {}
                    _ => {
                        let id = uplink.get("streamId").cloned().unwrap_or(Value::Null);
                        let _ = send(&mut socket, error_frame(id, "gateway/bad-frame", "上行帧 type 不认识")).await;
                    }
                }
            }
            frame = bridge.recv() => {
                match frame {
                    Ok((kind, value)) => {
                        for (stream_id, _) in follows.iter().filter(|(_, k)| *k == kind) {
                            let item = json!({"type": "item", "streamId": stream_id, "value": value.as_ref()});
                            if send(&mut socket, item).await.is_err() {
                                break; // 写失败=连接已死，收场（select 下轮 recv 也必败）
                            }
                        }
                    }
                    // 桥广播积压：跳号帧直接丢弃（客户端按 revision 铁律自会重连对齐）
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        println!("☁ 云悠 web mux：follow 下行积压丢 {n} 帧（客户端将重连对齐）");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// open 分派：按 endpoint 应答。端点名实测为斜杠形（client 线上值），点形兜底。
/// follow 打开即入推流名单（快照先行，live 随后）。
async fn dispatch_open(
    socket: &mut WebSocket,
    state: Arc<WebState>,
    uplink: &Value,
    follows: &mut Vec<(Value, crate::bridge::StreamKind)>,
) {
    let stream_id = uplink.get("streamId").cloned().unwrap_or(Value::Null);
    let endpoint = uplink.get("endpoint").and_then(Value::as_str).unwrap_or("");
    let payload = uplink.get("payload").cloned().unwrap_or(json!({}));
    match endpoint {
        // 全局会话控制流：基线=单会话投影块（title+sessionListMetadata；
        // 批⑬投影面——侧栏会话列表真名），live 投影帧经 Control 流别推流
        "session/control" | "session.control" => {
            let _ = send(
                socket,
                json!({"type": "item", "streamId": stream_id,
                       "value": state.wire.control_baseline()}),
            )
            .await;
            follows.push((stream_id, crate::bridge::StreamKind::Control));
        }
        // 单会话事件日志流：快照（重放环+流基线）→ 订阅 wire 桥 live 推流
        // （线约定 payload={args:{request:{address…}}}，兼容直接形态）
        "session/follow" | "session.follow" => {
            let request = payload
                .get("args")
                .and_then(|args| args.get("request"))
                .unwrap_or(&payload);
            let session_id = request
                .get("address")
                .and_then(|address| address.get("sessionId"))
                .and_then(Value::as_str);
            let _ = send(
                socket,
                json!({"type": "item", "streamId": stream_id,
                       "value": state.wire.snapshot(session_id)}),
            )
            .await;
            follows.push((stream_id, crate::bridge::StreamKind::Session));
        }
        // 工作区流：基线=单工作区（服务进程 cwd——单会话服务的诚实投影；
        // 对端 UI 点选工作区→session/create→prompt 桥全链路的触发源）。
        // sessionIds 带当前会话（composer chip 名 resolution 路径③依赖：
        // sessionWorkspace=items.find(w.sessionIds.includes(sessionId))）；
        // 元数据登记给首铸 upsert（create 后免刷新激活 composer）
        "workspace/follow" | "workspace.follow" => {
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "/".into());
            let title = std::path::Path::new(&cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| cwd.clone());
            state.wire.register_workspace(&cwd, &title);
            let now = crate::wire::now_ms();
            let session_ids = state.wire.session_ids();
            let _ = send(
                socket,
                json!({"type": "item", "streamId": stream_id, "value": {
                    "type": "baseline",
                    "value": {"items": [{
                        "workspaceId": state.wire.workspace_id(),
                        "path": cwd, "title": title,
                        "sessionIds": session_ids, "createdAt": now, "updatedAt": now,
                    }], "archivedSessionIds": [], "pinnedSessionIds": []}
                }}),
            )
            .await;
            follows.push((stream_id, crate::bridge::StreamKind::Workspace));
        }
        // 客户端事件下行流：ready 帧（waterfall 桥接远期——本片只建通道）
        "$events" => {
            let _ = send(
                socket,
                json!({"type": "item", "streamId": stream_id, "value": {
                    "type": "ready",
                    "clientId": uuid::Uuid::new_v4().simple().to_string(),
                    "host": {"home": state.session_dir.display().to_string()}
                }}),
            )
            .await;
        }
        other => {
            let _ = send(
                socket,
                error_frame(
                    stream_id.clone(),
                    "gateway/not-implemented",
                    &format!("端点 {other} 未实现（适配层后续片）"),
                ),
            )
            .await;
            let _ = send(socket, json!({"type": "end", "streamId": stream_id})).await;
        }
    }
}

/// 错误帧（gateway 统一三字段错误体）。
fn error_frame(stream_id: Value, code: &str, message: &str) -> Value {
    json!({"type": "error", "streamId": stream_id,
           "error": {"code": code, "message": message, "details": {}}})
}

/// 发一帧（序列化失败=协议 bug，降级错误帧不静默——ws.rs 同纪律）。
async fn send(socket: &mut WebSocket, frame: Value) -> Result<(), axum::Error> {
    let text = frame.to_string();
    socket.send(Message::text(text)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ServerMessage;
    use crate::server::tests::test_ws_connect;
    use crate::server::WebState;
    use crate::turns::TurnRunner;
    use crate::bridge::WireBridge;
    use futures_util::{SinkExt, StreamExt};
    use std::path::PathBuf;
    use tokio::sync::broadcast;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yunyou::Context;

    /// 测试态：与 server.rs 测试同构（无装配 root，走失败路径面）。
    fn state() -> Arc<WebState> {
        let (sender, _) = broadcast::channel(16);
        Arc::new(WebState {
            token: "t".into(),
            model: "test-model".into(),
            turns: Arc::new(TurnRunner::new(Arc::new(Context::new), "test-model".into())),
            broadcast: sender,
            session_dir: PathBuf::from("/tmp/yunyou-gw-test-sessions"),
            wire: Arc::new(WireBridge::new("test-model".into())),
        })
    }

    /// mux 握手与端点应答：control 基线（斜杠端点名）/未知端点 error+end。
    #[tokio::test(flavor = "multi_thread")]
    async fn mux握手与端点应答() {
        let mut ws = test_ws_connect(state()).await;
        // open session/control → baseline 空投影
        ws.send(WsMessage::text(
            r#"{"type":"open","streamId":"s1","endpoint":"session/control","payload":{}}"#,
        ))
        .await
        .expect("open 可写");
        let first = ws.next().await.expect("应有帧").expect("帧可读");
        let WsMessage::Text(data) = first else { panic!("应文本帧：{first:?}") };
        assert!(data.contains(r#""type":"baseline""#) && data.contains("projections"), "{data}");
        // open 未知端点 → error + end（fail-visible）
        ws.send(WsMessage::text(
            r#"{"type":"open","streamId":"s2","endpoint":"nope/x","payload":{}}"#,
        ))
        .await
        .expect("open2 可写");
        let mut saw_error = false;
        let mut saw_end = false;
        for _ in 0..4 {
            match ws.next().await {
                Some(Ok(WsMessage::Text(data))) => {
                    if data.contains("gateway/not-implemented") {
                        saw_error = true;
                    }
                    if data.contains(r#""type":"end""#) && data.contains("s2") {
                        saw_end = true;
                        break;
                    }
                }
                Some(Ok(_)) => {}
                _ => break,
            }
        }
        assert!(saw_error && saw_end, "未知端点应 error+end");
        let _ = ws.close(None).await;
    }

    /// follow live 桥端到端：打开 follow → 快照（重放环+流基线在场）→
    /// 广播 ServerMessage → item 帧实时推流（streamId 包裹+对端词汇转换）。
    #[tokio::test(flavor = "multi_thread")]
    async fn follow快照与live推流() {
        let state = state();
        let mut ws = test_ws_connect(state.clone()).await;
        ws.send(WsMessage::text(
            r#"{"type":"open","streamId":"f1","endpoint":"session/follow","payload":{"address":{"kind":"session","sessionId":"s-9"}}}"#,
        ))
        .await
        .expect("follow open 可写");
        // 快照先行：assistantStream 基线必带（transport.ts 强制），header.id=address
        let snap = ws.next().await.expect("应有快照").expect("快照可读");
        let WsMessage::Text(snap) = snap else { panic!("快照应文本：{snap:?}") };
        assert!(snap.contains(r#""type":"snapshot""#), "{snap}");
        assert!(snap.contains(r#""assistantStream""#) && snap.contains(r#""revision""#), "{snap}");
        assert!(snap.contains(r#""id":"s-9""#), "header.id 取 address.sessionId：{snap}");
        // 广播用户消息 → wire 桥转 user/message 事件 → item 帧推流
        state
            .broadcast
            .send(ServerMessage::UserMessage { text: "你好".into() })
            .expect("广播收件人在场（wire 泵订阅）");
        let mut saw_event = false;
        for _ in 0..4 {
            match ws.next().await {
                Some(Ok(WsMessage::Text(data))) => {
                    if data.contains(r#""streamId":"f1""#)
                        && data.contains(r#""type":"user/message""#)
                        && data.contains(r#""surfaceOp":"append""#)
                    {
                        saw_event = true;
                        break;
                    }
                }
                Some(Ok(_)) => {}
                _ => break,
            }
        }
        assert!(saw_event, "live 事件应经 follow 流推达：广播后未收 user/message");
        let _ = ws.close(None).await;
    }
}
