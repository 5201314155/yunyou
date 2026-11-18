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
    // follow 订阅：streamId + 流别 + 会话过滤器（session/follow 带
    // address.sessionId——步 3 定向路由；None=不过滤）。
    let mut follows: Vec<(Value, crate::bridge::StreamKind, Option<String>)> = Vec::new();
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
                    Ok((kind, tag, value)) => {
                        // 定向路由（裁决①）：Session 帧只达「未过滤」或「标签匹配」
                        // 的 follow；标签 None=活跃会话未铸（单会话现态唯一产出源，
                        // 凡 session follow 皆收——多会话后各自定向，见模块头）。
                        for (stream_id, _, _) in follows.iter().filter(|(_, k, filter)| {
                            *k == kind
                                && match (tag.as_deref(), filter.as_deref()) {
                                    (Some(t), Some(f)) if *k == crate::bridge::StreamKind::Session => t == f,
                                    _ => true,
                                }
                        }) {
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
    follows: &mut Vec<(Value, crate::bridge::StreamKind, Option<String>)>,
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
            follows.push((stream_id, crate::bridge::StreamKind::Control, None));
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
            // 步 3 定向：记 address.sessionId 过滤器（None=未过滤）
            let filter = request
                .get("address")
                .and_then(|address| address.get("sessionId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            follows.push((stream_id, crate::bridge::StreamKind::Session, filter));
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
            let now = crate::wire_frames::now_ms();
            // 步 4：sessionIds=内存表 ∪ 磁盘扫描（workspace 基线多会话同步）
            let session_ids = crate::sessions::all_ids(state.wire.session_ids(), &state.session_dir);
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
            follows.push((stream_id, crate::bridge::StreamKind::Workspace, None));
        }
        // 客户端事件下行流（$events）：ready 帧先行；live 事件经 Events 流别推流
        // （websocket.md §4：tasks/changed 等，增量提示，前端重拉投影）
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
            follows.push((stream_id, crate::bridge::StreamKind::Events, None));
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
            scheduler: Arc::new(yunyou_plugin_sdk::scheduler::Scheduler::new(|| 1_000_000)),
            config_store: Arc::new(crate::config_store::ConfigStore::new(std::path::PathBuf::from("/tmp/yunyou-test-yunyou.toml"))),
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

    /// 定向路由（步 3，裁决①）：铸会话 s-1 后，addressed s-1 的 follow 收
    /// live 帧、addressed s-2 的 follow 不收（各自定向）；未铸标签=凡 follow
    /// 皆收（单会话现态诚实规则，见 handle_mux 注记）。
    #[tokio::test(flavor = "multi_thread")]
    async fn live帧按sessionId定向() {
        let state = state();
        // 铸会话 s-1（活跃标签=s-1）
        let _ = call_create(&state, "s-1").await;
        let mut ws_hit = test_ws_connect(state.clone()).await;
        let mut ws_miss = test_ws_connect(state.clone()).await;
        for (ws, sid) in [(&mut ws_hit, "s-1"), (&mut ws_miss, "s-2")] {
            ws.send(WsMessage::text(format!(
                r#"{{"type":"open","streamId":"f-{sid}","endpoint":"session/follow","payload":{{"args":{{"request":{{"address":{{"kind":"session","sessionId":"{sid}"}}}}}}}}}}"#
            )))
            .await
            .expect("open 可写");
            // 吃掉快照帧
            let _ = ws.next().await.expect("快照在").expect("快照可读");
        }
        // 广播用户消息 → 标签 s-1
        state
            .broadcast
            .send(ServerMessage::UserMessage { text: "定向测试".into() })
            .expect("广播收件人在场");
        // s-1 follow 应达
        let mut hit = false;
        for _ in 0..4 {
            match ws_hit.next().await {
                Some(Ok(WsMessage::Text(data))) => {
                    if data.contains(r#""streamId":"f-s-1""#) && data.contains("user/message") {
                        hit = true;
                        break;
                    }
                }
                Some(Ok(_)) => {}
                _ => break,
            }
        }
        assert!(hit, "addressed s-1 的 follow 应收 live 帧");
        // s-2 follow 不得收（超时短窗探测：500ms 内无任何 user/message 帧即通过）
        let mut leaked = false;
        for _ in 0..2 {
            match tokio::time::timeout(std::time::Duration::from_millis(500), ws_miss.next()).await {
                Ok(Some(Ok(WsMessage::Text(data)))) => {
                    if data.contains("user/message") {
                        leaked = true;
                    }
                }
                Ok(Some(Ok(_))) => {}
                _ => break,
            }
        }
        assert!(!leaked, "addressed s-2 的 follow 不得收 s-1 的 live 帧");
        let _ = ws_hit.close(None).await;
        let _ = ws_miss.close(None).await;
    }

    /// session/create 经 unary 铸会话（定向测试辅助；真 HTTP 走 server 测试）。
    async fn call_create(state: &Arc<WebState>, session_id: &str) -> Value {
        use axum::extract::State;
        let body = format!(
            r#"{{"type":"client-request","rpcId":"c","method":"session/create","payload":{{"args":{{"request":{{"sessionId":"{session_id}"}}}}}}}}"#
        );
        let resp = crate::rpc::rpc_unary(State(state.clone()), axum::body::Bytes::from(body)).await;
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("体可读");
        serde_json::from_slice(&bytes).expect("合法 JSON")
    }
}
