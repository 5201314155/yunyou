//! rpc_credentials 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

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
        r#"{{"type":"client-request","rpcId":"k1","method":"{method}","payload":{payload}}}"#
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

/// 装配：真凭据插件（CredentialsPlugin provide CredentialService；config_store
/// 指向 temp yunyou.toml）。
async fn creds_setup(config_content: &str) -> Arc<WebState> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yunyou-web-creds-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let config_path = dir.join("yunyou.toml");
    std::fs::write(&config_path, config_content).expect("预置配置");
    let (state, session_file) = cmd_setup().await;
    let mut state = state;
    Arc::get_mut(&mut state)
        .expect("单引用可变")
        .config_store = Arc::new(ConfigStore::new(config_path));
    let root = state.turns.root();
    let handle = root.plugin(
        yunyou_credentials::CredentialsPlugin,
        serde_json::json!({"store_path": dir.join("credentials.toml").display().to_string()}),
    );
    let mut active = false;
    for _ in 0..400 {
        if handle.state() == yunyou::FiberState::Active {
            active = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(active, "CredentialsPlugin 应在限时内装载");
    let _ = session_file;
    state
}

/// list：未配置=set=false source 缺席；配置明文残存=source "file"（file 值**零回显**）。
#[tokio::test(flavor = "multi_thread")]
async fn 槽位目录与来源三值() {
    let plain = "[llm]\napi_key = \"sk-明文key\"\n";
    let state = creds_setup(plain).await;
    let value = call(&state, "credentials/list", "{}").await["result"]["value"].clone();
    let items = value["items"].as_array().expect("items 在");
    let slot = items.iter().find(|i| i["path"][0] == "llm").expect("llm.api_key 槽在");
    assert_eq!(slot["set"], true, "明文残存=已配置事实：{value}");
    assert_eq!(slot["source"], "file", "明文残存=file 来源（裁决②）");
    // 红线：目录序列化串零明文
    let serialized = serde_json::to_string(&value).expect("序列化");
    assert!(!serialized.contains("sk-明文key"), "目录零明文红线：{serialized}");
    // set 后：source 转 store（值入托管+配置改引用），配置文件零明文
    let value = call(&state, "credentials/set",
        r#"{"args":{"request":{"path":["llm","api_key"],"value":"sk-new-key"}}}"#)
        .await["result"]["value"].clone();
    assert_eq!(value["set"], true);
    let value = call(&state, "credentials/list", "{}").await["result"]["value"].clone();
    let slot = value["items"].as_array().unwrap()[0].clone();
    assert_eq!(slot["source"], "store", "托管后=store 来源：{slot}");
    let config_text = std::fs::read_to_string(state.config_store.path()).unwrap();
    assert!(config_text.contains("$YUNYOU_LLM_API_KEY"), "定点改写为引用：{config_text}");
    assert!(!config_text.contains("sk-"), "配置文件零明文：{config_text}");
    // store 文件 600 权限（unix）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let store_path = state.config_store.path().with_file_name("credentials.toml");
        let mode = std::fs::metadata(&store_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "托管存储 600：{mode:o}");
    }
}

/// set/delete 闭环：幂等重复 set 不炸；delete 后 set=false（unset 缺失 no-op）；
/// 响应零明文（set 的值永不回显）；载荷缺陷与未知槽位拒绝。
#[tokio::test(flavor = "multi_thread")]
async fn 写删闭环与幂等() {
    let state = creds_setup("[llm]\n").await;
    // set
    let resp = call(&state, "credentials/set",
        r#"{"args":{"request":{"path":["llm","api_key"],"value":"sk-abc"}}}"#).await;
    assert_eq!(resp["result"]["value"]["set"], true, "{resp}");
    // 幂等：重复 set 同值不炸
    let resp = call(&state, "credentials/set",
        r#"{"args":{"request":{"path":["llm","api_key"],"value":"sk-abc"}}}"#).await;
    assert_eq!(resp["result"]["value"]["set"], true);
    // set 响应零明文
    let serialized = serde_json::to_string(&resp).expect("序列化");
    assert!(!serialized.contains("sk-abc"), "set 响应零明文红线：{serialized}");
    // delete
    let resp = call(&state, "credentials/delete",
        r#"{"args":{"request":{"path":["llm","api_key"]}}}"#).await;
    assert_eq!(resp["result"]["value"]["deleted"], true, "{resp}");
    // delete 再删=no-op（unset 缺失语义）
    let resp = call(&state, "credentials/delete",
        r#"{"args":{"request":{"path":["llm","api_key"]}}}"#).await;
    assert_eq!(resp["result"]["value"]["deleted"], true, "缺失 no-op：{resp}");
    // 拒绝面：空值/未知槽位/缺 value
    for payload in [
        r#"{"args":{"request":{"path":["llm","api_key"],"value":"  "}}}"#,
        r#"{"args":{"request":{"path":["other","key"],"value":"x"}}}"#,
        r#"{"args":{"request":{"path":["llm","api_key"]}}}"#,
    ] {
        let resp = call(&state, "credentials/set", payload).await;
        assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request", "{payload}: {resp}");
    }
}
