//! commands 域 unary RPC（W2 批，契约 web.v1 §9.1/域-会话命令与技能.md §2）：
//! `commands/list`（字典序目录投影）+ `commands/execute`（命令缝执行桥）。
//!
//! # 语义分家
//! - 目录：CommandsService.list() 纯数据投影（无 handler），kind 二分
//!   action|expand 是前端行为分派唯一依据（契约 §2.1）；
//! - 执行：复用 [`crate::commands::execute`]（同注册表同 handler 同 D2 落盘，
//!   持回合闸门）；结算回真实会话，不只弹本地菜单（规范 §2.3）；
//! - `/quit`：契约=壳层拦截断本连接——unary 通道无连接可断，人话拒绝（不落盘）；
//! - 未知命令=信封内 `{ok:false,text}`（契约 §2.2 表），非 error 信封。

use crate::contract::{self, err_envelope};
use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;
use yunyou::{ServiceId};
use yunyou_commands::CommandsService;

/// commands/list：目录投影（字典序稳定；服务未就绪=装配异常，fail-visible）。
pub(crate) fn commands_list(state: &Arc<WebState>) -> Value {
    let Ok(commands) = state
        .turns
        .root()
        .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
    else {
        return err_envelope(
            contract::code::NOT_IMPLEMENTED,
            "commands 服务未就绪（装配异常），命令目录不可用",
        );
    };
    let items: Vec<Value> = commands
        .list()
        .into_iter()
        .map(|descriptor| {
            json!({
                "name": descriptor.name,
                "aliases": descriptor.aliases,
                "description": descriptor.description,
                "kind": descriptor.kind.as_str(),
                "accepts_args": descriptor.accepts_args,
            })
        })
        .collect();
    json!({"ok": true, "value": {"items": items}})
}

/// commands/execute（payload `{raw}`，raw 含前导斜杠）：命令缝执行桥。
/// async=执行持回合闸门须 await；结算形态=`{ok,text}`（契约 §2.2 未知命令
/// 也是信封内 ok:false）。
pub(crate) async fn commands_execute(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(raw) = request.get("raw").and_then(Value::as_str) else {
        return err_envelope(contract::code::BAD_REQUEST, "execute 缺 raw（需 \"/命令 [参数]\" 形态）");
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return err_envelope(contract::code::BAD_REQUEST, "raw 为空白（需以 / 开头的命令）");
    }
    if !raw.starts_with('/') {
        return err_envelope(contract::code::BAD_REQUEST, "raw 需以 / 开头（这是命令通道，消息请走 session/prompt）");
    }
    // /quit：unary 通道无连接可断（WS 壳层拦截语义），人话拒绝不落盘
    let token = raw[1..].split_whitespace().next().unwrap_or("");
    if token == "quit" {
        return json!({"ok": true, "value": {"ok": false, "text":
            "/quit 的语义是断开浏览器连接：请直接关闭页面或断开 WS；命令通道不支持"}});
    }
    // await 执行持回合闸门（命令 handler 毫秒级，/new 除外——闸门保证不与回合并发）
    crate::commands::execute(state, raw)
        .await
        .map(|msg| match msg {
        crate::protocol::ServerMessage::CommandOutput { text, ok } => {
            json!({"ok": true, "value": {"ok": ok, "text": text}})
        }
        crate::protocol::ServerMessage::Error { text } => {
            err_envelope(contract::code::BAD_REQUEST, &text)
        }
        other => err_envelope(
            contract::code::BAD_REQUEST,
            &format!("命令结算帧形态异常：{other:?}"),
        ),
    })
    .unwrap_or_else(|| json!({"ok": true, "value": {"ok": true, "text": ""}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tests::setup;
    use crate::rpc::rpc_unary;
    use axum::extract::State;
    use axum::http::StatusCode;

    /// 经真实 unary 通道发一封 client-request（走 rpc.rs 分派到本模块）。
    async fn call(state: &Arc<WebState>, method: &str, payload: &str) -> Value {
        let body = format!(
            r#"{{"type":"client-request","rpcId":"c1","method":"{method}","payload":{payload}}}"#
        );
        let resp = rpc_unary(State(state.clone()), axum::body::Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("体可读");
        serde_json::from_slice(&bytes).expect("合法 JSON")
    }

    /// 目录投影：注册表字典序、kind 二分、别名带出；/ping 在列。
    #[tokio::test(flavor = "multi_thread")]
    async fn 目录投影形状() {
        let (state, _) = setup().await;
        let value = call(&state, "commands/list", "{}").await["result"]["value"].clone();
        let items = value["items"].as_array().expect("items 在");
        let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "字典序稳定：{names:?}");
        let ping = items.iter().find(|i| i["name"] == "ping").expect("ping 在列");
        assert_eq!(ping["kind"], "action");
        assert_eq!(ping["aliases"][0], "乒", "别名带出");
        assert_eq!(ping["accepts_args"], false);
        let skill = items.iter().find(|i| i["name"] == "skill1").expect("skill1 在列");
        assert_eq!(skill["kind"], "expand");
    }

    /// 执行桥：动作类成功 ok/text、参数余段提示前缀在 text、未知命令
    /// 信封内 ok:false、/quit 人话拒绝、非斜杠 raw 400 级拒绝、D2 落盘配对。
    #[tokio::test(flavor = "multi_thread")]
    async fn 执行桥闭环() {
        let (state, session_file) = setup().await;
        // 动作类成功
        let value = call(&state, "commands/execute", r#"{"raw":"/ping"}"#).await["result"]["value"].clone();
        assert_eq!(value["ok"], true, "{value}");
        assert_eq!(value["text"], "pong");
        // 参数余段：提示前缀不阻断（契约 §2.2 表）
        let value = call(&state, "commands/execute", r#"{"raw":"/ping 多余"}"#).await["result"]["value"].clone();
        assert!(value["ok"] == true && value["text"].as_str().unwrap_or("").contains("未接受参数"), "{value}");
        // 未知命令：信封内 ok:false（非 error 信封）
        let value = call(&state, "commands/execute", r#"{"raw":"/nope"}"#).await["result"]["value"].clone();
        assert_eq!(value["ok"], false, "{value}");
        assert!(value["text"].as_str().unwrap_or("").contains("未知命令 /nope"));
        // /quit：unary 人话拒绝（契约=壳层拦截；此处不落盘不杀服务）
        let value = call(&state, "commands/execute", r#"{"raw":"/quit"}"#).await["result"]["value"].clone();
        assert_eq!(value["ok"], false, "{value}");
        assert!(value["text"].as_str().unwrap_or("").contains("断开浏览器连接"));
        // 展开类：原文上屏+不代发注记
        let value = call(&state, "commands/execute", r#"{"raw":"/skill1"}"#).await["result"]["value"].clone();
        let text = value["text"].as_str().unwrap_or("");
        assert!(value["ok"] == true && text.contains("# 技能正文") && text.contains("不代发"), "{text}");
        // 载荷缺陷：缺 raw / 空白 / 非斜杠 → error 信封 bad-request
        for payload in ["{}", r#"{"raw":"  "}"#, r#"{"raw":"你好"}"#] {
            let resp = call(&state, "commands/execute", payload).await;
            assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request", "{payload}: {resp}");
        }
        // D2：run/done 配对落盘（ping×2+skill1×1；未知与 /quit 不计）
        let lines = std::fs::read_to_string(&session_file).unwrap_or_default();
        assert_eq!(lines.matches("\"type\":\"command-run\"").count(), 3, "run 3 行：{lines}");
        assert_eq!(lines.matches("\"type\":\"command-done\"").count(), 3, "done 3 行：{lines}");
    }
}
