//! system / devices 域 unary RPC（W6 批，契约 web.v1 域-配置凭据与插件.md §6
//! 与 规格.md §11）：`system/info`（版本/契约/模型/工作区根/会话目录）、
//! `system/health`（探活）、`devices/current`（当前连接鉴权形态）。
//!
//! # 红线
//! devices/current 只描述**当前这条连接**：via 三值判定复用 token::from_request
//! 顺序（query>bearer>cookie）；**令牌零回显**（secret 红线同规）、无设备清单。

use crate::server::WebState;
use axum::http::{header, HeaderMap};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

/// 进程启动时刻（uptimeMs 锚；OnceLock=首问即记，跨实例语义诚实）。
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn started() -> Instant {
    *STARTED.get_or_init(Instant::now)
}

/// system/info（L1）：关于页数据源（版本/许可走静态资产，不入接口——契约 §11）。
pub(crate) fn system_info(state: &Arc<WebState>) -> Value {
    let workspace_root = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/".to_string());
    json!({"ok": true, "value": {
        "version": env!("CARGO_PKG_VERSION"),
        "contract": "web.v1",
        "model": state.model,
        "workspaceRoot": workspace_root,
        "sessionDir": state.session_dir.display().to_string(),
        "startedAt": started_at_ms(),
        "nowMs": crate::wire_frames::now_ms(),
    }})
}

/// system/health（L1）：探活（uptime/turnOpen——TurnRunner 只读口）。
pub(crate) fn system_health(state: &Arc<WebState>) -> Value {
    json!({"ok": true, "value": {
        "ok": true,
        "uptimeMs": started().elapsed().as_millis() as u64,
        "turnOpen": state.turns.turn_open(),
    }})
}

/// devices/current（L1，payload 无参）：当前连接鉴权形态。
/// via 判定顺序与 token::from_request 同源（query>bearer>cookie）；
/// connected 恒 true（中间件已拦未鉴权请求——到不了这里）。
pub(crate) fn devices_current(query: Option<&str>, headers: &HeaderMap, state: &Arc<WebState>) -> Value {
    let via = if query_token_present(query) {
        "query"
    } else if bearer_present(headers) {
        "bearer"
    } else {
        "cookie"
    };
    json!({"ok": true, "value": {
        "connected": true,
        "via": via,
        "loopback": state.loopback,
    }})
}

/// 查询串是否带非空 token（与 token.rs 解析同序）。
fn query_token_present(query: Option<&str>) -> bool {
    crate::token::from_request(query, &HeaderMap::new()).is_some()
}

/// Bearer 头是否在（存在即算，值不读出——令牌零回显）。
fn bearer_present(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|text| text.starts_with("Bearer "))
}

/// 启动时刻（毫秒 epoch；OnceLock 首问锚定）。
fn started_at_ms() -> u64 {
    let elapsed = started().elapsed().as_millis() as u64;
    crate::wire_frames::now_ms().saturating_sub(elapsed)
}

// 分派在 rpc.rs（信封成形单源）；本文件三方法均返回 Value。

#[cfg(test)]
#[path = "rpc_system_tests.rs"]
mod rpc_system_tests;
