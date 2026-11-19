//! rpc_plugins 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

use crate::commands::tests::setup as cmd_setup;
use crate::config_store::ConfigStore;
use crate::rpc::rpc_unary;
use crate::server::WebState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use std::sync::Arc;

/// 经真实 unary 通道发一封 client-request。
async fn call(state: &Arc<WebState>, method: &str, payload: &str) -> Value {
    let body = format!(
        r#"{{"type":"client-request","rpcId":"p1","method":"{method}","payload":{payload}}}"#
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

/// 装配：config_store 指向 temp yunyou.toml（预置 [plugins] 段）。
async fn plugins_setup(config_content: &str) -> Arc<WebState> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yunyou-web-plug-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let config_path = dir.join("yunyou.toml");
    std::fs::write(&config_path, config_content).expect("预置配置");
    let (state, _) = cmd_setup().await;
    let mut state = state;
    Arc::get_mut(&mut state).expect("单引用可变").config_store =
        Arc::new(ConfigStore::new(config_path));
    state
}

/// plugins/list：config [plugins] 条目诚实投影（enable 回显）；enable 写入落
/// 配置+config/reloaded 事件广播；config 读回段值。
#[tokio::test(flavor = "multi_thread")]
async fn 列表与启停与配置读写() {
    let state = plugins_setup("[plugins]\n[plugins.yunyou-team]\nenable = true\n").await;
    let mut events = state.wire.subscribe();
    // list：config 条目在列（bundled 无磁盘 manifest=值缺席不伪造）
    let value = call(&state, "plugins/list", "{}").await["result"]["value"].clone();
    let items = value["items"].as_array().expect("items 在");
    let team = items.iter().find(|i| i["id"] == "yunyou-team").expect("条目在");
    assert_eq!(team["enabled"], true, "{value}");
    // enable 写入：revision 锁 + config/reloaded 事件
    let revision = state.config_store.revision();
    let value = call(&state, "plugins/enable",
        &serde_json::json!({"args":{"request":{"pluginId":"yunyou-team","enable":false,"revision":revision}}}).to_string())
        .await["result"]["value"].clone();
    assert_eq!(value["enable"], false, "{value}");
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
        .await.expect("事件在").expect("流在");
    assert_eq!(event.0, crate::bridge::StreamKind::Events);
    assert_eq!(event.2["type"], "config/reloaded");
    assert_eq!(event.2["revision"], revision + 1);
    // 读回：enable=false 落盘
    let value = call(&state, "plugins/list", "{}").await["result"]["value"].clone();
    let team = value["items"].as_array().unwrap().iter()
        .find(|i| i["id"] == "yunyou-team").expect("条目在");
    assert_eq!(team["enabled"], false, "写后读回：{team}");
    // 冲突：旧 revision → config/revision-conflict
    let resp = call(&state, "plugins/enable",
        &serde_json::json!({"args":{"request":{"pluginId":"yunyou-team","enable":true,"revision":revision}}}).to_string())
        .await;
    assert_eq!(resp["result"]["error"]["code"], "config/revision-conflict", "{resp}");
    // config 读：段值回显
    let value = call(&state, "plugins/config",
        r#"{"args":{"request":{"pluginId":"yunyou-team"}}}"#)
        .await["result"]["value"].clone();
    assert_eq!(value["config"]["enable"], false, "{value}");
    // config 写：深度校验（[plugins] deny_unknown_fields——非法键拒绝）
    let resp = call(&state, "plugins/config",
        &format!(r#"{{"args":{{"request":{{"pluginId":"yunyou-team","revision":{},"config":{{"nope":1}}}}}}}}"#, revision + 1))
        .await;
    assert_eq!(resp["result"]["error"]["code"], "config/validation", "{resp}");
    // 载荷缺陷：缺 pluginId/缺 enable/缺 revision
    for payload in [
        r#"{"args":{"request":{"enable":true,"revision":1}}}"#,
        r#"{"args":{"request":{"pluginId":"x","revision":1}}}"#,
        r#"{"args":{"request":{"pluginId":"x","enable":true}}}"#,
    ] {
        let resp = call(&state, "plugins/enable", payload).await;
        assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request", "{payload}: {resp}");
    }
}

/// plugins/tools（方案 A）：诚实空 items+status.errors 人话指路（归属投影挂后续批）。
#[tokio::test(flavor = "multi_thread")]
async fn tools诚实面() {
    let (state, _) = cmd_setup().await;
    let value = call(&state, "plugins/tools",
        r#"{"args":{"request":{"pluginId":"yunyou-team"}}}"#)
        .await["result"]["value"].clone();
    assert_eq!(value["items"].as_array().map(Vec::len), Some(0), "{value}");
    let errors = value["status"]["errors"].as_array().expect("errors 在");
    assert!(errors[0].as_str().unwrap_or("").contains("归属面未建"), "{value}");
    assert!(errors[0].as_str().unwrap_or("").contains("--verbose"), "人话指路（裁决③口径）");
}
