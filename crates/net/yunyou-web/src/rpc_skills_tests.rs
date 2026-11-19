//! rpc_skills 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

use crate::commands::tests::setup as cmd_setup;
use crate::rpc::rpc_unary;
use crate::server::WebState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use std::path::PathBuf;
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

/// 造 temp 技能场（.yunyou 槽：good 正常+noheading 无标题）。
fn skill_dir() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yunyou-web-skill-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let slot = dir.join(".yunyou").join("skills");
    std::fs::create_dir_all(&slot).expect("槽目录");
    std::fs::write(slot.join("good.md"), "# 好技能\n正文第一行").expect("写技能");
    std::fs::write(slot.join("noheading.md"), "没有标题").expect("写技能");
    dir
}

/// list：project 双槽目录（source 恒 "project" 诚实——云枢注记）+ 无标题跳过；
/// get：正文回显+缺席/非法 id 拒绝（不伪造）。cwd 切换=进程级状态，current_thread
/// 单线程 runtime 内串行安全（测试进程并发场互不串扰：各测独立 temp 目录）。
#[tokio::test(flavor = "current_thread")]
async fn 目录与正文与缺席拒绝() {
    let dir = skill_dir();
    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&dir).expect("切 cwd");
    // cwd 敏感面在单线程 runtime 内完成（cmd_setup 装配+三 RPC 调用）
    let state = cmd_setup().await.0;
    let value = call(&state, "skills/list", "{}").await;
    let items = value["result"]["value"]["items"].as_array().expect("items 在");
    assert_eq!(items.len(), 1, "无标题跳过：{value}");
    assert_eq!(items[0]["id"], "good");
    assert_eq!(items[0]["source"], "project", "source 恒 project（云枢注记）");
    // get：正文
    let value = call(&state, "skills/get", r#"{"args":{"request":{"id":"good"}}}"#).await;
    assert_eq!(value["result"]["value"]["description"], "好技能", "{value}");
    assert_eq!(value["result"]["value"]["body"], "# 好技能\n正文第一行");
    // 缺席/非法 id/缺 id → bad-request（不伪造）
    for payload in [
        r#"{"args":{"request":{"id":"nope"}}}"#,
        r#"{"args":{"request":{"id":"Bad"}}}"#,
        r#"{"args":{"request":{}}}"#,
    ] {
        let resp = call(&state, "skills/get", payload).await;
        assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request", "{payload}: {resp}");
    }
    std::env::set_current_dir(prev).expect("还原 cwd");
    let _ = std::fs::remove_dir_all(&dir);
}
