//! session 域提交/恢复桥（多会话步 2 自 rpc.rs 拆出，行数纪律）：
//! `session/prompt`（提交桥：闸门受理→后台回合，echo 对账经 wire 桥回流）+
//! `session/open`（会话恢复：切活跃+JSONL 尾部灌入重放环）。
//! 方法分派与信封成形在 rpc.rs；契约解包 `request_of` 在 contract.rs。

use crate::contract::{self, code, err_envelope};
use crate::protocol::ServerMessage;
use crate::server::WebState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::sync::Arc;
use yunyou_session::SessionStore;

/// typert 线约定解包（contract 统一实现）。
use contract::request_of;

/// 提交桥（session/prompt，施工图 §3.3）：取 content 首个 text → 回合闸门
/// 即刻受理（占住=agent-busy 人话拒绝）→ 后台跑回合，事件经 wire 桥回流。
/// requestId 先登记（user/message source echo 对账——客户端乐观 echo 退场凭据）。
pub(crate) async fn rpc_prompt(state: &Arc<WebState>, body: &Value) -> Value {
    let payload = body.get("payload").cloned().unwrap_or(json!({}));
    let request = request_of(&payload);
    let request_id = request.get("requestId").and_then(Value::as_str).unwrap_or("");
    let mode = request.get("mode").and_then(Value::as_str).unwrap_or("");
    let text = request.get("content").and_then(Value::as_array).and_then(|parts| {
        parts.iter().find_map(|part| {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                part.get("text").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            }
        })
    });
    if request_id.is_empty() {
        return err_envelope(code::PROMPT_INVALID, "prompt 缺 requestId（对端 echo 对账必需）");
    }
    if !matches!(mode, "queue" | "steer") {
        return err_envelope(code::PROMPT_INVALID, "prompt mode 需 queue|steer");
    }
    let Some(text) = text else {
        return err_envelope(
            code::PROMPT_INVALID,
            "prompt content 需至少一个 text 部分（空数组续跑语义未桥接）",
        );
    };
    if text.trim().is_empty() {
        return err_envelope(code::PROMPT_INVALID, "prompt 正文为空白");
    }
    // 回合闸门：即刻探测——占住=对端 agent-busy（人话文案与 ws 层同源）
    let Some(guard) = state.turns.try_lock_gate() else {
        return err_envelope(
            code::AGENT_BUSY,
            "当前会话正在对话中，等本回合落定再发新消息",
        );
    };
    state.wire.set_request_id(Some(request_id.to_string()));
    let runner = state.turns.clone();
    let sender = state.broadcast.clone();
    tokio::spawn(async move {
        // 受理后失败（agentLoop 缺失/回合失败）：人话进事件流——wire 桥转
        // system/message；回合已开则一并闭合（turn/end error）。
        // 用户停止（cancel 令牌贯穿，agent-loop Err loop/cancelled）：转
        // 人话落款，不露内核错误码。
        if let Err(err) = runner.run_locked(&text, sender.clone()).await {
            let text = if err.contains("loop/cancelled") {
                "已停止本回合（用户取消）".to_string()
            } else {
                err
            };
            let _ = sender.send(ServerMessage::Error { text });
        }
        drop(guard); // 守卫持至回合落定（含 Err），闸门语义与 ws 层等价
    });
    json!({"ok": true, "value": {"accepted": true}})
}

/// 会话恢复桥（session/open，payload `{sessionId}`）：校验 id 与 JSONL 存在 →
/// 锁外读尾部（HISTORY_CAP 封顶）→ 锁内切活跃+灌入。跳过数如实进响应。
/// session/list（多会话步 4）：内存行（wire 聚合，活跃行 agentAvailable=true）
/// ∪ 磁盘扫描行（agentAvailable=false 诚实——open 后才可 prompt）。扫描行
/// 标题=derive_title 首个 user-message（无则 title 缺席不虚构），排序=内存行
/// 先、扫描行按 mtime 倒序（会话中心先见活跃，历史随后）。
pub(crate) fn session_list(state: &Arc<WebState>) -> Value {
    let mut items = state.wire.session_summaries();
    let known: Vec<String> = items
        .iter()
        .filter_map(|row| row["sessionId"].as_str().map(str::to_string))
        .collect();
    let mut scanned = crate::sessions::scan_rows(&state.session_dir);
    scanned.sort_by_key(|row| std::cmp::Reverse(row.modified_ms));
    for row in scanned {
        if known.contains(&row.id) {
            continue; // 内存表已有（活跃/已开）行优先，不重复出列
        }
        let mut values = serde_json::json!({"sessionListMetadata": {
            "blank": row.blank, "lastPromptAt": serde_json::Value::Null,
        }});
        if let Some(title) = &row.title {
            values["title"] = serde_json::Value::String(title.clone());
        }
        items.push(serde_json::json!({
            "sessionId": row.id,
            "agentAvailable": false,
            "updatedAt": row.modified_ms,
            "running": false,
            "blank": row.blank,
            "projections": {"kind": "sequenced", "asOfSeq": -1, "values": values},
        }));
    }
    serde_json::json!({"ok": true, "value": {"items": items}})
}

pub(crate) async fn rpc_open(state: &Arc<WebState>, body: &Value) -> Response {
    let payload = body.get("payload").cloned().unwrap_or(json!({}));
    let request = request_of(&payload);
    let Some(session_id) = request.get("sessionId").and_then(Value::as_str) else {
        return rpc_response(
            body.get("rpcId").cloned().unwrap_or(Value::Null),
            err_envelope(contract::code::BAD_REQUEST, "open 缺 sessionId"),
            StatusCode::OK,
        );
    };
    if !crate::sessions::valid_id(session_id) {
        return rpc_response(
            body.get("rpcId").cloned().unwrap_or(Value::Null),
            err_envelope(contract::code::BAD_REQUEST, &format!("会话名不合法：{session_id}")),
            StatusCode::OK,
        );
    }
    let path = state.session_dir.join(format!("{session_id}.jsonl"));
    if !path.is_file() {
        return rpc_response(
            body.get("rpcId").cloned().unwrap_or(Value::Null),
            err_envelope(
                contract::code::BAD_REQUEST,
                &format!("会话文件不存在：{session_id}.jsonl"),
            ),
            StatusCode::OK,
        );
    }
    // 锁外读（IO 不持 wire 锁）：尾部 HISTORY_CAP 封顶——完整历史在 JSONL
    let events = match yunyou_session::JsonlStore::new(path).load_all().await {
        Ok(mut events) => {
            if events.len() > crate::wire_frames::HISTORY_CAP {
                events.split_off(events.len() - crate::wire_frames::HISTORY_CAP)
            } else {
                events
            }
        }
        Err(err) => {
            return rpc_response(
                body.get("rpcId").cloned().unwrap_or(Value::Null),
                err_envelope(
                    contract::code::BAD_REQUEST,
                    &format!("会话 {session_id} 读取失败：{err}"),
                ),
                StatusCode::OK,
            )
        }
    };
    let (ingested, skipped) = state.wire.open_session(session_id, &events);
    rpc_response(
        body.get("rpcId").cloned().unwrap_or(Value::Null),
        json!({"ok": true, "value": {
            "sessionId": session_id,
            "ingested": ingested,
            "skipped": skipped,
            "skippedNote": if skipped > 0 {
                format!("跳过 {skipped} 条未桥接词汇事件（完整历史以会话文件为准）")
            } else { String::new() },
        }}),
        StatusCode::OK,
    )
}

/// server-response 信封统一形态（application/json；json feature 未启用，
/// 字符串体+头手工成形）。
pub(crate) fn rpc_response(rpc_id: Value, result: Value, status: StatusCode) -> Response {
    let envelope = json!({"type": "server-response", "rpcId": rpc_id, "result": result});
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        envelope.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use crate::commands::tests::setup as cmd_setup;
    use axum::extract::State;

    /// session/list 多行合成（步 4）：内存活跃行 agentAvailable=true ∪ 磁盘
    /// 扫描行 agentAvailable=false（去重、mtime 倒序、标题 derive_title 不虚构、
    /// 无用户消息=blank）；workspace 全量 ids 同步含扫描会话。
    #[tokio::test(flavor = "multi_thread")]
    async fn 会话列表多行合成() {
        let (state, _session_file) = cmd_setup().await;
        // 磁盘造两个历史会话（有用户消息/无用户消息各一）+ 非法名文件
        let dir = state.session_dir.clone();
        std::fs::create_dir_all(&dir).expect("sessions 目录");
        std::fs::write(
            dir.join("hist-a.jsonl"),
            format!("{}\n", serde_json::json!({"type":"user-message","text":"历史甲问\n第二行"})),
        )
        .unwrap();
        std::fs::write(
            dir.join("hist-b.jsonl"),
            format!("{}\n", serde_json::json!({"type":"command-run","command_id":"c"})),
        )
        .unwrap();
        std::fs::write(dir.join("..evil.jsonl"), "").unwrap();
        // 铸活跃会话（内存行）
        let create = crate::rpc::rpc_unary(
            State(state.clone()),
            axum::extract::RawQuery(None),
            HeaderMap::new(),
            axum::body::Bytes::from(
                r#"{"type":"client-request","rpcId":"c","method":"session/create","payload":{"args":{"request":{"sessionId":"live-1"}}}}"#,
            ),
        )
        .await;
        let _ = axum::body::to_bytes(create.into_body(), usize::MAX).await.expect("体可读");
        // 多行合成：live-1（true）+ hist-a（false，有标题）+ hist-b（false blank）
        let value = session_list(&state);
        let items = value["value"]["items"].as_array().expect("items 在");
        assert_eq!(items.len(), 3, "三行：{value}");
        let live = items.iter().find(|i| i["sessionId"] == "live-1").expect("活跃行在");
        assert_eq!(live["agentAvailable"], true);
        let a = items.iter().find(|i| i["sessionId"] == "hist-a").expect("hist-a 在");
        assert_eq!(a["agentAvailable"], false, "扫描行不可直接 prompt");
        assert_eq!(a["blank"], false);
        assert_eq!(a["projections"]["values"]["title"], "历史甲问", "短标题不带省略号：{}", a);
        let b = items.iter().find(|i| i["sessionId"] == "hist-b").expect("hist-b 在");
        assert_eq!(b["blank"], true, "无用户消息=blank");
        assert!(b["projections"]["values"].get("title").is_none(), "无标题不虚构");
        // 非法名不入列；workspace 全量 ids 含扫描会话
        let ids = crate::sessions::all_ids(state.wire.session_ids(), &state.session_dir);
        assert!(ids.contains(&"hist-a".to_string()) && ids.contains(&"live-1".to_string()), "{ids:?}");
        assert!(!ids.iter().any(|id| id.contains("evil")), "非法名不入：{ids:?}");
    }
}
