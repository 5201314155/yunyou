//! rpc_normalize 测试（外挂：#[path] 先例，源文件行数纪律）。

use super::*;

/// 纯函数锚点：三形态同效（点形/斜杠形/裸名）+异常形态不归一。
#[test]
fn 三形态同效与异常不归一() {
    // 裸名原样
    assert_eq!(normalize_method("team/roster"), "team/roster");
    assert_eq!(normalize_method("session/list"), "session/list");
    // 斜杠形别名
    assert_eq!(normalize_method("yunyou/team/roster"), "team/roster");
    assert_eq!(normalize_method("yunyou/tasks/list"), "tasks/list");
    assert_eq!(normalize_method("yunyou/schedule/delete"), "schedule/delete");
    // 点形（B2 终定）
    assert_eq!(normalize_method("yunyou/team.roster"), "team/roster");
    assert_eq!(normalize_method("yunyou/tasks.create"), "tasks/create");
    assert_eq!(normalize_method("yunyou/schedule.list"), "schedule/list");
    // 多级/异常形态不归一（fail-visible 兜底，不猜）
    assert_eq!(normalize_method("yunyou/team.roster.x"), "yunyou/team.roster.x");
    assert_eq!(normalize_method("yunyou/team/roster/x"), "yunyou/team/roster/x");
    // 非三域不归一（不扩面）
    assert_eq!(normalize_method("yunyou/session/list"), "yunyou/session/list");
    assert_eq!(normalize_method("yunyou/settings/write"), "yunyou/settings/write");
}

/// 尾段合法性：空尾/二级分隔拒绝。
#[test]
fn 尾段合法性() {
    assert!(tail_slash_ok("team/roster"));
    assert!(!tail_slash_ok("team/"));
    assert!(!tail_slash_ok("team/a/b"));
    assert!(!tail_slash_ok("team/a.b"));
}

// —— 端到端锚点（经真实 unary 通道：三形态同信封；归一纯函数面的行为面）——

use crate::commands::tests::setup as cmd_setup;
use crate::rpc::rpc_unary;
use crate::server::WebState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use std::sync::Arc;

async fn call(state: &Arc<WebState>, method: &str) -> Value {
    let body = format!(
        r#"{{"type":"client-request","rpcId":"n1","method":"{method}","payload":{{}}}}"#
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

    /// 端到端三形态同效（A 案）：三域两态同效（team/roster 与
    /// yunyou/team/roster 同响应）；非三域前缀不归一（session/yunyou-x 不动）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 前缀别名两态同效() {
        let state = cmd_setup().await.0;
        // 无团队插件环境：roster 返回「服务未就绪」人话信封——两态同信封即归一成立
        let bare = call(&state, "team/roster").await;
        let prefixed = call(&state, "yunyou/team/roster").await;
        assert_eq!(prefixed["result"], bare["result"], "前缀别名同效：{prefixed}");
        let bare = call(&state, "tasks/list").await;
        let prefixed = call(&state, "yunyou/tasks/list").await;
        assert_eq!(prefixed["result"], bare["result"], "tasks 域前缀同效：{prefixed}");
        let bare = call(&state, "schedule/list").await;
        let prefixed = call(&state, "yunyou/schedule/list").await;
        // nowMs 毫秒级漂移不进比对（比对 entries+ok——归一语义锚）
        assert_eq!(prefixed["result"]["ok"], bare["result"]["ok"], "schedule 域前缀同效：{prefixed}");
        assert_eq!(prefixed["result"]["value"]["entries"], bare["result"]["value"]["entries"]);
        // 点形（B2 wire 形定）：yunyou/<域>.<动作> 三态同效
        for method in ["yunyou/team.roster", "yunyou/tasks.list", "yunyou/schedule.list"] {
            let bare = call(&state, &method["yunyou/".len()..].replace('.', "/")).await;
            let dotted = call(&state, method).await;
            assert_eq!(
                dotted["result"]["error"], bare["result"]["error"],
                "点形同效（无插件环境=同款人话信封）：{method} → {dotted}"
            );
        }
        // 多级/异常形态不归一（fail-visible 兜底，不猜）
        let weird = call(&state, "yunyou/team.roster.x").await;
        assert_eq!(weird["result"]["error"]["code"], "gateway/not-implemented", "{weird}");
        // 非三域不归一：yunyou/session/list 未知（fail-visible，不扩面）
        let outside = call(&state, "yunyou/session/list").await;
        assert_eq!(outside["result"]["error"]["code"], "gateway/not-implemented", "{outside}");
    }
