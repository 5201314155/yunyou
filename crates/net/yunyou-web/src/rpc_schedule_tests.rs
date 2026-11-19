//! rpc_schedule 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

use crate::commands::tests::setup as cmd_setup;
use crate::rpc::rpc_unary;
use crate::server::WebState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use std::sync::Arc;

/// 经真实 unary 通道发一封 client-request。
async fn call(state: &Arc<WebState>, method: &str, payload: &str) -> Value {
    let body = format!(
        r#"{{"type":"client-request","rpcId":"s1","method":"{method}","payload":{payload}}}"#
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

/// create：合法规格受理（id 形态/nextFireMs/nowMs/note 诚实）+ 规格违例三态
/// （三选一零或二/at 过去/every_minutes<5 → schedule/spec）+ 缺 message。
#[tokio::test(flavor = "multi_thread")]
async fn 创建_受理与规格违例() {
    let (state, _) = cmd_setup().await;
    // 合法 interval（假时钟 now=1_000_000）
    let value = call(&state, "schedule/create",
        r#"{"args":{"request":{"every_minutes":10,"message":"该喝水了"}}}"#)
        .await["result"]["value"].clone();
    assert!(value["id"].as_str().unwrap_or("").starts_with("sched-"), "{value}");
    assert_eq!(value["nextFireMs"].as_u64(), Some(1_600_000), "锚点+间隔：{value}");
    assert_eq!(value["durable"], false);
    assert!(value["note"].as_str().unwrap_or("").contains("会话内有效"));
    // durable=true：受理但诚实 note（不跨重启）
    let value = call(&state, "schedule/create",
        r#"{"args":{"request":{"every_minutes":30,"message":"巡检","durable":true}}}"#)
        .await["result"]["value"].clone();
    assert_eq!(value["durable"], true);
    assert!(value["note"].as_str().unwrap_or("").contains("不跨重启"), "{value}");
    // list：两条在册（spec serde 形态/nextFireMs/nowMs 在场）
    let value = call(&state, "schedule/list", "{}").await["result"]["value"].clone();
    let entries = value["entries"].as_array().expect("entries 在");
    assert_eq!(entries.len(), 2, "{value}");
    assert_eq!(entries[0]["spec"]["Interval"]["every_ms"], 600_000);
    assert!(value["nowMs"].is_u64());
    // 违例：三选一零/二、at 过去、every_minutes<5、缺 message
    for (payload, why) in [
        (r#"{"args":{"request":{"message":"x"}}}"#, "零选"),
        (r#"{"args":{"request":{"at_ms":2000000,"every_minutes":10,"message":"x"}}}"#, "双选"),
        (r#"{"args":{"request":{"at_ms":1,"message":"x"}}}"#, "at 过去"),
        (r#"{"args":{"request":{"every_minutes":4,"message":"x"}}}"#, "低于下限"),
        (r#"{"args":{"request":{"every_minutes":10}}}"#, "缺 message"),
    ] {
        let resp = call(&state, "schedule/create", payload).await;
        let code = resp["result"]["error"]["code"].as_str().unwrap_or("");
        assert!(
            code == "schedule/spec" || code == "gateway/bad-request",
            "{why}: {resp}"
        );
    }
    // delete：命中→deleted；unknown→schedule/unknown-id+details.entries 现有清单
    let id = entries[0]["id"].as_str().expect("id 在").to_string();
    let value = call(&state, "schedule/delete", &format!(r#"{{"args":{{"request":{{"id":"{id}"}}}}}}"#))
        .await["result"]["value"].clone();
    assert_eq!(value["deleted"], id);
    let resp = call(&state, "schedule/delete", &format!(r#"{{"args":{{"request":{{"id":"{id}"}}}}}}"#)).await;
    assert_eq!(resp["result"]["error"]["code"], "schedule/unknown-id", "{resp}");
    let remaining = resp["result"]["error"]["details"]["entries"].as_array().expect("entries 在");
    assert_eq!(remaining.len(), 1, "删一条剩一条：{resp}");
    // 缺 id → bad-request
    let resp = call(&state, "schedule/delete", "{}").await;
    assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request");
}

/// At 型未来时刻受理+终态摘除（tick 驱动假时钟——注入触发器未挂，仅登记面）。
#[tokio::test(flavor = "multi_thread")]
async fn at型登记与终态摘除() {
    let (state, _) = cmd_setup().await;
    let value = call(&state, "schedule/create",
        r#"{"args":{"request":{"at_ms":1500000,"message":"提醒"}}}"#)
        .await["result"]["value"].clone();
    assert_eq!(value["nextFireMs"].as_u64(), Some(1_500_000));
    // tick（now=假时钟 1_000_000 < at 1_500_000）：未到期，仍在册
    state.scheduler.tick();
    let value = call(&state, "schedule/list", "{}").await["result"]["value"].clone();
    assert_eq!(value["entries"].as_array().map(Vec::len), Some(1), "未到期仍在册：{value}");
}
