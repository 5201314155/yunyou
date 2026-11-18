//! schedule 域 unary RPC（W4 批，契约 web.v1 域-配置凭据与插件.md §3）：
//! `schedule/create`（三选一必填恰一；nextFireMs 回显即确认）、`schedule/list`、
//! `schedule/delete`（id 定点删）。宿主调度器=WebState.scheduler（SDK 缝
//! v1 内存注册表）；到期注入=回合闸门内的正常回合（schedule_fire.rs）。
//!
//! # durable 诚实纪律（契约 L5）
//! SDK v1 恒内存注册表（durable 持久化=S4 未闭环）：durable=true 请求受理但
//! 留痕 stderr + 响应 note 如实「会话内有效，不跨重启」；不给跨重启开关。

use crate::contract::{self, code, err_body_details, err_envelope};
use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;
use yunyou_plugin_sdk::scheduler::ScheduleSpec;

/// 间隔下限（域细则 §3-2：防过密循环打爆会话）。
const MIN_EVERY_MINUTES: u64 = 5;

/// schedule/create（L2，回显即确认）：三选一必填恰一；nowMs 由调度器时钟。
pub(crate) fn schedule_create(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(message) = request.get("message").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "create 缺 message（到期注入文本，必填）");
    };
    let at = request.get("at_ms").and_then(Value::as_u64);
    let every_minutes = request.get("every_minutes").and_then(Value::as_u64);
    let cron = request.get("cron").and_then(Value::as_str);
    let chosen = [at.is_some(), every_minutes.is_some(), cron.is_some()]
        .iter()
        .filter(|chosen| **chosen)
        .count();
    if chosen != 1 {
        return err_envelope(
            code::SCHEDULE_SPEC,
            "触发规格三选一必填恰一：at_ms / every_minutes / cron",
        );
    }
    let spec = if let Some(at_ms) = at {
        ScheduleSpec::At { at_ms }
    } else if let Some(minutes) = every_minutes {
        if minutes < MIN_EVERY_MINUTES {
            return err_envelope(
                code::SCHEDULE_SPEC,
                &format!("every_minutes {minutes} 低于下限 {MIN_EVERY_MINUTES}（防过密循环打爆会话）"),
            );
        }
        ScheduleSpec::Interval { every_ms: minutes * 60_000 }
    } else {
        ScheduleSpec::Cron { expr: cron.unwrap_or_default().to_string() }
    };
    let durable = request.get("durable").and_then(Value::as_bool).unwrap_or(false);
    if durable {
        // 契约 L5：durable 一律按会话内有效投影；留痕=日志可查
        eprintln!(
            "[schedule] durable=true 请求受理：持久化未闭环（S4），按会话内有效处理——id 待回显"
        );
    }
    // 会话绑定：v1 Web 单活跃会话，session_id 取活跃（未铸=None=进程内全局）
    let session_id = state.wire.session_ids().into_iter().next();
    match state.scheduler.register_schedule(session_id, spec, message.to_string()) {
        Ok((id, next_fire)) => {
            let note = if durable {
                "durable 已受理：持久化未闭环，本条按会话内有效（进程退出即失，不跨重启）".to_string()
            } else {
                "会话内有效（进程退出即失）".to_string()
            };
            json!({"ok": true, "value": {
                "id": id,
                "nextFireMs": next_fire,
                "nowMs": crate::wire_frames::now_ms(),
                "message": message,
                "durable": durable,
                "note": note,
            }})
        }
        Err(err) => err_envelope(code::SCHEDULE_SPEC, &err.message),
    }
}

/// schedule/list（L1）：在规定时列表（id/规格/下次触发/消息）+ nowMs。
pub(crate) fn schedule_list(state: &Arc<WebState>) -> Value {
    let entries: Vec<Value> = state
        .scheduler
        .list()
        .iter()
        .map(|(entry, next)| {
            json!({
                "id": entry.id,
                "spec": serde_json::to_value(&entry.spec).unwrap_or(Value::Null),
                "nextFireMs": next,
                "message": entry.message,
            })
        })
        .collect();
    json!({"ok": true, "value": {"entries": entries, "nowMs": crate::wire_frames::now_ms()}})
}

/// schedule/delete（L3，前端二次确认）：不存在=schedule/unknown-id +
/// details.entries 现有清单（域细则 §3 表）。
pub(crate) fn schedule_delete(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(id) = request.get("id").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "delete 缺 id（schedule/list 查看在册）");
    };
    match state.scheduler.unregister_schedule(id) {
        Ok(()) => json!({"ok": true, "value": {"deleted": id}}),
        Err(_) => {
            let entries: Vec<Value> = state
                .scheduler
                .list()
                .iter()
                .map(|(entry, _)| Value::String(entry.id.clone()))
                .collect();
            let mut details = serde_json::Map::new();
            details.insert("entries".into(), Value::Array(entries));
            json!({"ok": false, "error": err_body_details(
                code::SCHEDULE_UNKNOWN_ID,
                &format!("调度 {id} 不存在（schedule_list 查看在册清单）"),
                details,
            )})
        }
    }
}

#[cfg(test)]
#[path = "rpc_schedule_tests.rs"]
mod rpc_schedule_tests;
