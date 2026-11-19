//! rpc_settings 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

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
        r#"{{"type":"client-request","rpcId":"w1","method":"{method}","payload":{payload}}}"#
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

/// 装配：config_store 指向 temp yunyou.toml（预置内容）。
async fn settings_setup(config_content: &str) -> Arc<WebState> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yunyou-web-set-{}-{n}", std::process::id()));
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

/// sections：九段+plugins 全列带 revision；form：llm 段投影含 secret 密码框标注
/// +secrets sidecar+**value/default 双空**；form 序列化串零明文（红线）。
#[tokio::test(flavor = "multi_thread")]
async fn 段清单与表单投影secret红线() {
    let state = settings_setup("[llm]\napi_key = \"sk-明文key\"\ntimeout_secs = 30\n").await;
    // sections
    let value = call(&state, "settings/sections", "{}").await["result"]["value"].clone();
    let items = value["items"].as_array().expect("items 在");
    assert!(items.len() >= 10, "九段+plugins：{value}");
    assert!(items.iter().all(|i| i["revision"].is_u64()));
    // form：llm 段
    let value = call(&state, "settings/form",
        r#"{"args":{"request":{"section":"llm"}}}"#)
        .await["result"]["value"].clone();
    let fields = value["fields"].as_array().expect("fields 在");
    if std::env::var("DBG").is_ok() { println!("FORM={}", serde_json::to_string_pretty(&value).unwrap()); }
    let api_key = fields.iter().find(|f| f["path"][0] == "api_key").expect("api_key 字段在");
    assert_eq!(api_key["secret"], true, "密码框标注：{api_key}");
    assert_eq!(api_key["value"], Value::Null, "secret value 恒 null");
    assert_eq!(api_key["default"], Value::Null, "secret default 恒 null");
    assert_eq!(api_key["required"], false, "secret required 恒 false");
    let secrets = value["secrets"].as_array().expect("secrets 在");
    assert!(secrets.iter().any(|s| s["path"][0] == "api_key" && s["set"] == true), "{value}");
    // timeout_secs 非密字段：值回显正常
    let timeout = fields.iter().find(|f| f["path"][0] == "timeout_secs").expect("字段在");
    assert_eq!(timeout["value"], 30, "非密字段值回显：{timeout}");
    assert_eq!(timeout["secret"], false);
    // 红线：form 投影序列化串零明文
    let serialized = serde_json::to_string(&value).expect("序列化");
    assert!(!serialized.contains("sk-明文key"), "form 零明文红线：{serialized}");
    // 未知段拒绝
    let resp = call(&state, "settings/form", r#"{"args":{"request":{"section":"nope"}}}"#).await;
    assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request");
}

/// write：revision 冲突=config/revision-conflict details.current；深度校验失败=
/// config/validation details.keys；secret 路径恒拒（credentials/write-only 指路）；
/// 合法写=revision+1+applied 需重启；响应零明文红线。
#[tokio::test(flavor = "multi_thread")]
async fn 修订写冲突校验与secret拒写() {
    let state = settings_setup("[llm]\ntimeout_secs = 30\n").await;
    let revision = state.config_store.revision();
    // 合法写
    let value = call(&state, "settings/write",
        &format!(r#"{{"args":{{"request":{{"section":"llm","revision":{revision},"value":{{"timeout_secs":60}}}}}}}}"#))
        .await["result"]["value"].clone();
    assert_eq!(value["revision"], revision + 1, "{value}");
    assert_eq!(value["restartRequired"], true, "applied 恒需重启（裁决①）：{value}");
    assert_eq!(value["applied"][0], "需重启");
    // 冲突：旧 revision 再写
    let resp = call(&state, "settings/write",
        &format!(r#"{{"args":{{"request":{{"section":"llm","revision":{revision},"value":{{"timeout_secs":90}}}}}}}}"#))
        .await;
    assert_eq!(resp["result"]["error"]["code"], "config/revision-conflict", "{resp}");
    assert_eq!(resp["result"]["error"]["details"]["current"], revision + 1);
    // 深度校验失败：未知键
    let resp = call(&state, "settings/write",
        &format!(r#"{{"args":{{"request":{{"section":"llm","revision":{},"value":{{"nope":1}}}}}}}}"#, revision + 1))
        .await;
    assert_eq!(resp["result"]["error"]["code"], "config/validation", "{resp}");
    assert!(resp["result"]["error"]["details"]["keys"].as_array().expect("keys 在")
        [0].as_str().unwrap_or("").contains("llm.nope"), "{resp}");
    // secret 路径恒拒：指路 credentials/set（裁决），details.path 定位
    let resp = call(&state, "settings/write",
        &format!(r#"{{"args":{{"request":{{"section":"llm","revision":{},"value":{{"api_key":"sk-x"}}}}}}}}"#, revision + 1))
        .await;
    assert_eq!(resp["result"]["error"]["code"], "credentials/write-only", "{resp}");
    assert!(resp["result"]["error"]["message"].as_str().unwrap_or("").contains("credentials/set"));
    // 载荷缺陷：缺 revision/缺 value/未知段 → bad-request
    for payload in [
        r#"{"args":{"request":{"section":"llm","value":{}}}}"#,
        r#"{"args":{"request":{"section":"llm","revision":99}}}"#,
        r#"{"args":{"request":{"section":"nope","revision":99,"value":{}}}}"#,
    ] {
        let resp = call(&state, "settings/write", payload).await;
        assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request", "{payload}: {resp}");
    }
}
