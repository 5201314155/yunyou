//! v2 上游前端适配网关的 unary RPC 面（任务40 适配层）：`POST /api/<method>`。
//!
//! # 信封（client-request/server-response，形状实测自 client/connection/rpc.ts）
//! 请求=`{type:'client-request',rpcId,method,payload}`（payload=请求对象本体）；
//! 响应=`{type:'server-response',rpcId,result:{ok:true,value}|{ok:false,error}}`。
//!
//! # 本片方法面（typert.host.js 实测全集的已接子集）
//! `session/list`（空 items）、`session/create`（铸造/复用会话 id）、
//! `session/page`（空页——loadOlder 兜底）、`session/prompt`（提交桥：闸门
//! 受理→后台回合，echo 对账经 wire 桥回流）。其余方法显式 not-implemented
//! （fail-visible，不伪造值——解析错比假值可诊断）。

use crate::protocol::ServerMessage;
use crate::server::WebState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::sync::Arc;

/// unary RPC（POST /api/<method>）：client-request 信封 → server-response 信封。
pub(crate) async fn rpc_unary(
    State(state): State<Arc<WebState>>,
    body: axum::body::Bytes,
) -> Response {
    // json feature 未启用（依赖预批收敛清单）：Bytes+serde 手解，形态等价
    let parsed: Result<Value, _> = serde_json::from_slice(&body);
    let Ok(body) = parsed else {
        return rpc_response(
            Value::Null,
            json!({"ok": false, "error": err_body("gateway/bad-request", "请求体不是合法 JSON")}),
            StatusCode::BAD_REQUEST,
        );
    };
    if body.get("type").and_then(Value::as_str) != Some("client-request") {
        return rpc_response(
            Value::Null,
            json!({"ok": false, "error": err_body("gateway/bad-request", "非 client-request 信封")}),
            StatusCode::BAD_REQUEST,
        );
    }
    let rpc_id = body.get("rpcId").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        // 侧栏会话列表（批⑬投影面）：单会话真名行（title 投影键——
        // manager.ts:755 消费路径）；未铸会话=空表
        "session/list" => {
            json!({"ok": true, "value": {"items": state.wire.session_summaries()}})
        }
        // 停止按钮（SessionCancelRequest={sessionId}，types.ts:366）：取消
        // 进行中回合；空闲=人话拒绝（fail-visible，不静默装成功）
        "session/cancel" => {
            if state.turns.cancel_active() {
                json!({"ok": true, "value": {"accepted": true}})
            } else {
                err_envelope("session/no-active-turn", "当前没有正在执行的回合，无需停止")
            }
        }
        // 模型触达面（ModelCatalog，types.ts:135）：单模型单供应商的诚实
        // 目录——default=服务配置模型，无失败面（failures 空=无目录加载失败）
        "session/modelCatalog" | "modelCatalog" => json!({"ok": true, "value": {
            "default": {"provider": "yunyou", "model": state.model},
            "routableProviders": ["yunyou"],
            "groups": [{"id": "yunyou", "name": "云悠", "models": [{
                "id": state.model, "name": state.model,
            }]}],
            "failures": [],
        }}),
        "session/create" => {
            let payload = body.get("payload").cloned().unwrap_or(json!({}));
            let requested = request_of(&payload).get("sessionId").and_then(Value::as_str);
            json!({"ok": true, "value": {"sessionId": state.wire.create_session(requested)}})
        }
        // loadOlder 向上翻页兜底：空页（本进程会话流不超快照重放环）
        "session/page" => json!({"ok": true, "value": {"records": [], "hasMore": false}}),
        "session/prompt" => rpc_prompt(&state, &body),
        _ => json!({"ok": false, "error": err_body(
            "gateway/not-implemented",
            &format!("RPC 方法 {method} 未实现（适配层后续片）")
        )}),
    };
    rpc_response(rpc_id, result, StatusCode::OK)
}

/// 提交桥（session/prompt，施工图 §3.3）：取 content 首个 text → 回合闸门
/// 即刻受理（占住=agent-busy 人话拒绝）→ 后台跑回合，事件经 wire 桥回流。
/// requestId 先登记（user/message source echo 对账——客户端乐观 echo 退场凭据）。
fn rpc_prompt(state: &Arc<WebState>, body: &Value) -> Value {
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
        return err_envelope("gateway/bad-request", "prompt 缺 requestId（对端 echo 对账必需）");
    }
    if !matches!(mode, "queue" | "steer") {
        return err_envelope("gateway/bad-request", "prompt mode 需 queue|steer");
    }
    let Some(text) = text else {
        return err_envelope(
            "gateway/bad-request",
            "prompt content 需至少一个 text 部分（空数组续跑语义未桥接）",
        );
    };
    if text.trim().is_empty() {
        return err_envelope("gateway/bad-request", "prompt 正文为空白");
    }
    // 回合闸门：即刻探测——占住=对端 agent-busy（人话文案与 ws 层同源）
    let Some(guard) = state.turns.try_lock_gate() else {
        return err_envelope(
            "session/agent-busy",
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

/// typert 线约定解包：payload=`{args:{<参数名>:<值>}}`（FaceModel 参数名
/// 'request' 为单参方法惯例；无参方法 args={}）。兼容直接 payload 形态
/// （批⑤ 探针/直调测试用）。
fn request_of(payload: &Value) -> &Value {
    payload
        .get("args")
        .and_then(|args| args.get("request"))
        .unwrap_or(payload)
}

/// 错误 result 信封（信封内 ok:false——fail-visible 不伪造值）。
fn err_envelope(code: &str, message: &str) -> Value {
    json!({"ok": false, "error": err_body(code, message)})
}

/// 错误体（gateway 统一三字段）。
fn err_body(code: &str, message: &str) -> Value {
    json!({"code": code, "message": message, "details": {}})
}

/// server-response 信封统一形态（application/json；json feature 未启用，
/// 字符串体+头手工成形）。
fn rpc_response(rpc_id: Value, result: Value, status: StatusCode) -> Response {
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
    use crate::server::WebState;
    use crate::turns::TurnRunner;
    use crate::bridge::WireBridge;
    use std::path::PathBuf;
    use tokio::sync::broadcast;
    use yunyou::Context;

    /// 测试态：与 server.rs/gateway.rs 测试同构（无装配 root，走失败路径面）。
    fn state() -> Arc<WebState> {
        let (sender, _) = broadcast::channel(16);
        Arc::new(WebState {
            token: "t".into(),
            model: "test-model".into(),
            turns: Arc::new(TurnRunner::new(Arc::new(Context::new), "test-model".into())),
            broadcast: sender,
            session_dir: PathBuf::from("/tmp/yunyou-rpc-test-sessions"),
            wire: Arc::new(WireBridge::new("test-model".into())),
        })
    }

    /// 发一封 client-request 并解响应信封（真机 HTTP 走 server.rs 鉴权测试，
    /// 此处直调处理器专注方法分派）。
    async fn call(state: &Arc<WebState>, method: &str, payload: &str) -> Value {
        let body = format!(
            r#"{{"type":"client-request","rpcId":"r1","method":"{method}","payload":{payload}}}"#
        );
        let resp = rpc_unary(State(state.clone()), axum::body::Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::OK, "{method} 应走信封（200）");
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("响应体可读");
        serde_json::from_slice(&bytes).expect("响应是合法 JSON")
    }

    /// 方法分派：list 空 items / create 铸造并登记 / page 空页 / 未知方法
    /// not-implemented / 坏信封 400。
    #[tokio::test(flavor = "multi_thread")]
    async fn 方法分派与坏信封() {
        let state = state();
        let list = call(&state, "session/list", "{}").await;
        assert_eq!(list["result"]["value"]["items"].as_array().map(Vec::len), Some(0));
        let create = call(&state, "session/create", r#"{"sessionId":"s-1"}"#).await;
        assert_eq!(create["result"]["value"]["sessionId"], "s-1", "复用客户端指定 id");
        let page = call(&state, "session/page", r#"{"address":{"sessionId":"s-1"}}"#).await;
        assert_eq!(page["result"]["value"]["hasMore"], false);
        let unknown = call(&state, "nope/x", "{}").await;
        assert_eq!(unknown["result"]["ok"], false, "未知方法 fail-visible");
        assert!(unknown["result"]["error"]["code"] == "gateway/not-implemented");
        // 坏信封：400
        let bad = rpc_unary(State(state.clone()), axum::body::Bytes::from(r#"{"type":"nope"}"#))
            .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST, "非 client-request 应 400");
    }

    /// 提交桥：合法载荷受理（accepted+echo 登记+后台回合失败人话回流）；
    /// 载荷缺陷四态拒绝；闸门占住=agent-busy。
    #[tokio::test(flavor = "multi_thread")]
    async fn 提交桥受理与拒绝() {
        // 闸门占用态先建（防局部 state 绑定遮蔽 state() 函数）
        let busy_state = state();
        let _permit = busy_state.turns.try_lock_gate().expect("闸门空闲可取");
        let state = state();
        let mut rx = state.broadcast.subscribe();
        let payload = r#"{"requestId":"req-7","sessionId":"s-1","mode":"queue",
            "content":[{"type":"text","text":"你好"}]}"#;
        let ok = call(&state, "session/prompt", payload).await;
        assert_eq!(ok["result"]["value"]["accepted"], true, "合法载荷应受理：{ok}");
        // 无装配 root → 后台回合失败 → Error 人话进广播（wire 桥转 system/message）
        let err = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("失败应回流（2s 内）")
            .expect("广播在");
        assert!(matches!(err, ServerMessage::Error { ref text } if text.contains("agentLoop")), "{err:?}");
        // requestId 已登记（echo 对账凭据；user/message 消费面在 wire.rs 测试锁定）
        assert_eq!(state.wire.request_id().as_deref(), Some("req-7"), "echo 登记在场");
        // 载荷缺陷：mode 非法 / 无 text 部分 / 正文空白 → 信封内 ok:false
        for (payload, why) in [
            (r#"{"requestId":"r","sessionId":"s","mode":"other","content":[{"type":"text","text":"x"}]}"#, "mode"),
            (r#"{"requestId":"r","sessionId":"s","mode":"queue","content":[]}"#, "无 text"),
            (r#"{"requestId":"r","sessionId":"s","mode":"queue","content":[{"type":"text","text":"  "}]}"#, "空白"),
        ] {
            let reject = call(&state, "session/prompt", payload).await;
            assert_eq!(reject["result"]["ok"], false, "{why} 应拒绝：{reject}");
            assert_eq!(reject["result"]["error"]["code"], "gateway/bad-request");
        }
        // 闸门占住：对端 agent-busy（人话文案与 ws 层同源）
        let busy = call(
            &busy_state,
            "session/prompt",
            r#"{"requestId":"r2","sessionId":"s","mode":"queue","content":[{"type":"text","text":"hi"}]}"#,
        )
        .await;
        assert_eq!(busy["result"]["error"]["code"], "session/agent-busy", "占住应 agent-busy：{busy}");
        assert!(
            busy["result"]["error"]["message"].as_str().unwrap_or("").contains("正在对话中"),
            "人话文案：{busy}"
        );
    }

    /// 触达面（批⑬）：list 真名行（未铸=空）/cancel 空闲人话拒绝/
    /// modelCatalog 形状（default/groups/failures 空面）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 触达面_列表与停止与模型目录() {
        let state = state();
        // 未铸会话：list 空表（不伪造行）
        let list = call(&state, "session/list", "{}").await;
        assert_eq!(list["result"]["value"]["items"].as_array().map(Vec::len), Some(0));
        // cancel：无进行中回合=人话拒绝（fail-visible，不装成功）
        let cancel = call(&state, "session/cancel", r#"{"sessionId":"s"}"#).await;
        assert_eq!(cancel["result"]["ok"], false, "空闲 cancel 应拒绝：{cancel}");
        assert_eq!(cancel["result"]["error"]["code"], "session/no-active-turn");
        // modelCatalog：单供应商诚实目录
        let catalog = call(&state, "session/modelCatalog", "{}").await;
        let value = &catalog["result"]["value"];
        assert_eq!(value["default"], serde_json::json!({"provider": "yunyou", "model": "test-model"}));
        assert_eq!(value["groups"][0]["name"], "云悠");
        assert_eq!(value["failures"].as_array().map(Vec::len), Some(0));
        // 铸会话后：list 行带投影块（title 尚未派生=键缺席）
        let _ = call(&state, "session/create", r#"{"sessionId":"s-9"}"#).await;
        let list = call(&state, "session/list", "{}").await;
        let items = list["result"]["value"]["items"].as_array().expect("items 在");
        assert_eq!(items.len(), 1, "单会话单行：{list}");
        assert_eq!(items[0]["sessionId"], "s-9");
        assert_eq!(items[0]["blank"], true, "无回合=blank");
        assert_eq!(
            items[0]["projections"]["values"]["sessionListMetadata"]["blank"], true,
            "投影块在列"
        );
    }
}
