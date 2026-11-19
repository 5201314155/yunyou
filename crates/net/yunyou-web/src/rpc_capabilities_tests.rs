//! rpc_capabilities 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

use crate::commands::tests::setup as cmd_setup;
use crate::rpc::rpc_unary;
use crate::rpc_team::rpc_team_tests::team_setup;
use crate::server::WebState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use std::sync::Arc;

/// 经真实 unary 通道发一封 client-request。
async fn call(state: &Arc<WebState>, method: &str) -> Value {
    let body = format!(
        r#"{{"type":"client-request","rpcId":"q1","method":"{method}","payload":{{}}}}"#
    );
    let resp = rpc_unary(
        State(state.clone()),
        axum::extract::RawQuery(None),
        HeaderMap::new(),
        axum::body::Bytes::from(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("体可读");
    serde_json::from_slice(&bytes).expect("合法 JSON")
}

/// 无团队插件装配：团队/凭据 unavailable 带 gate（诚实灰置），
/// session 恒可用、scheduler 可用带 durable gate、market 恒 gate。
#[tokio::test(flavor = "multi_thread")]
async fn 无插件装配的诚实灰置() {
    let (state, _) = cmd_setup().await;
    let value = call(&state, "capabilities/list").await["result"]["value"].clone();
    let items = value["items"].as_array().expect("items 在");
    if std::env::var("DBG").is_ok() { println!("CAPS1={}", serde_json::to_string_pretty(&value).unwrap()); }
    let find = |key: &str| items.iter().find(|i| i["key"] == key).expect("条目在").clone();
    // 恒可用与恒 gate 面
    assert_eq!(find("session")["available"], true);
    assert!(find("session")["gate"].is_null(), "可用面不带 gate");
    assert_eq!(find("market")["available"], false);
    assert!(find("market")["gate"].as_str().unwrap_or("").contains("S11"), "{value}");
    assert_eq!(find("skills")["available"], false, "下沉批未落=灰置");
    assert_eq!(find("scheduler")["available"], true);
    assert!(find("scheduler")["gate"].as_str().unwrap_or("").contains("durable"));
    // 团队/凭据：未启用→unavailable+gate
    assert_eq!(find("team")["available"], false, "{value}");
    assert!(find("team")["gate"].as_str().unwrap_or("").contains("未启用"));
    // commands：cmd_setup 装了 CommandsPlugin→可用
    assert_eq!(find("commands")["available"], true);
    // 红线：available=false 必带 gate（契约 §9.3 不出假开关）
    for item in items {
        if item["available"] == false {
            assert!(item["gate"].is_string(), "unavailable 必带 gate：{item}");
        }
    }
}

/// 有团队+凭据插件装配（team_setup 路径复用）：team/settings 翻绿。
#[tokio::test(flavor = "multi_thread")]
async fn 插件启用后翻绿() {
    let state = team_setup().await.0;
    let value = call(&state, "capabilities/list").await["result"]["value"].clone();
    let items = value["items"].as_array().expect("items 在");
    if std::env::var("DBG").is_ok() { println!("CAPS2={}", serde_json::to_string_pretty(&value).unwrap()); }
    let team = items.iter().find(|i| i["key"] == "team").expect("条目在");
    assert_eq!(team["available"], true, "团队插件在册：{value}");
    assert!(team["gate"].is_null(), "翻绿不带 gate：{team}");
}
