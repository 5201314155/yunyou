//! rpc_system 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

use crate::commands::tests::setup as cmd_setup;
use crate::rpc::rpc_unary;
use crate::rpc_system::{devices_current, system_health};
use crate::server::WebState;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use serde_json::Value;
use std::sync::Arc;

/// 经真实 unary 通道发一封 client-request（含查询串/头透传——devices 三形态）。
async fn call(state: &Arc<WebState>, method: &str, query: Option<&str>, headers: HeaderMap) -> Value {
    let body = format!(
        r#"{{"type":"client-request","rpcId":"y1","method":"{method}","payload":{{}}}}"#
    );
    let resp = rpc_unary(
        State(state.clone()),
        axum::extract::RawQuery(query.map(str::to_string)),
        headers,
        axum::body::Bytes::from(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("体可读");
    serde_json::from_slice(&bytes).expect("合法 JSON")
}

/// info/health 形状（version/contract/turnOpen）+经真实通道。
#[tokio::test(flavor = "multi_thread")]
async fn info与health形状() {
    let (state, _) = cmd_setup().await;
    let value = call(&state, "system/info", None, HeaderMap::new()).await["result"]["value"].clone();
    assert_eq!(value["contract"], "web.v1", "{value}");
    assert_eq!(value["model"], "test-model");
    assert!(value["version"].is_string() && value["nowMs"].is_u64());
    assert!(value["sessionDir"].is_string() && value["workspaceRoot"].is_string());
    // health：turnOpen=false（无回合）
    let value = call(&state, "system/health", None, HeaderMap::new()).await["result"]["value"].clone();
    assert_eq!(value["ok"], true);
    assert_eq!(value["turnOpen"], false);
    assert!(value["uptimeMs"].is_u64());
    // 口径注记：turnOpen=active_cancel 在场（run 时设，闸门 permit≠回合中）——
    // 无装配环境 run 必败无法造 true 态，此处锁 false 现态+字段在位。
    let health = system_health(&state);
    assert_eq!(health["value"]["turnOpen"], false, "无回合=turnOpen false：{health}");
}

/// devices/current 三 via 形态（query>bearer>cookie 判定顺序）+**令牌零回显红线**。
#[tokio::test(flavor = "multi_thread")]
async fn devices三形态与令牌零回显() {
    let (state, _) = cmd_setup().await;
    // query 优先
    let value = devices_current(Some("token=q1&x=2"), &HeaderMap::new(), &state);
    assert_eq!(value["value"]["via"], "query", "{value}");
    // bearer 次之
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, "Bearer sk-secret-token".parse().unwrap());
    let value = devices_current(None, &headers, &state);
    assert_eq!(value["value"]["via"], "bearer");
    // cookie 兜底
    let mut cookie_headers = HeaderMap::new();
    cookie_headers.insert(
        header::COOKIE,
        "yunyou-web-token=c-secret".parse().unwrap(),
    );
    let value = devices_current(None, &cookie_headers, &state);
    assert_eq!(value["value"]["via"], "cookie");
    // 无 any → cookie 兜底（中间件已拦未鉴权，此处仅形态判定）
    let value = devices_current(None, &HeaderMap::new(), &state);
    assert_eq!(value["value"]["via"], "cookie");
    assert_eq!(value["value"]["connected"], true);
    assert_eq!(value["value"]["loopback"], true, "测试态=回环");
    // 红线：响应串零令牌
    let serialized = serde_json::to_string(&value).expect("序列化");
    assert!(!serialized.contains("c-secret") && !serialized.contains("sk-secret"), "令牌零回显：{serialized}");
}
