//! team / tasks 域 unary RPC（W3 批，契约 web.v1 域-团队与任务.md）：
//! 名册/消息/踢人/拉人 + 任务板读写。Web 操作者身份恒=lead 视角（服务端
//! ensure_actor/身份派发强制）；写操作不持回合闸门（板非会话资源，并发由
//! TeamCore 板锁保证——lead 2026-11-19 裁决）；写成功即广播 tasks/changed
//! （websocket.md §4 增量提示，前端重拉 tasks/list，单真源不轮询）。
//!
//! # 错误映射（挂账清理批：结构化直透——TeamOpError 枚举，不再依赖 Display
//! 文案锚）
//! Board(SelfReview)→team/self-review｜Board(NotOwner)→team/not-owner｜
//! Board(其余)→team/state｜UnknownActor→team/unknown-member；details 字段化
//! （from/op/owner/actor/submitter/task_id），恒无 secret/栈。

use crate::contract::{self, code, err_body_details, err_envelope};
use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;
use yunyou::hub::{Address, MessageKind};
use yunyou::{ErrorKind, ServiceId};
use yunyou_team::{ContextMode, TeamService, TeammateSpec};

/// 服务取用（缺失=团队插件未启用，全域人话拒绝——不伪造空名册）。
fn service(state: &Arc<WebState>) -> Result<Arc<dyn TeamService>, Value> {
    state
        .turns
        .root()
        .get::<dyn TeamService>(ServiceId::of::<dyn TeamService>())
        .map_err(|_| {
            err_envelope(
                code::NOT_IMPLEMENTED,
                "团队服务未就绪（团队插件未启用），团队面不可用",
            )
        })
}

/// 内核错误 → 契约错误信封（映射规则见模块头）。
fn service_err(err: &yunyou::CordisError) -> Value {
    // 服务级错误（spawn/message/kick 等 hub 面）：kind 映射（HubUnknownMember→
    // team/unknown-member；其余原样透传 team/state 人话）
    let code_str = if *err.kind() == ErrorKind::HubUnknownMember {
        code::TEAM_UNKNOWN_MEMBER
    } else {
        code::TEAM_STATE
    };
    err_envelope(code_str, &err.to_string())
}

/// 任务板写操作结构化错误映射（挂账清理批：TeamOpError 直透，details 字段化）。
fn board_op_err(err: &yunyou_team::TeamOpError) -> Value {
    use yunyou_team::{BoardOpError, TeamOpError};
    let mut details = serde_json::Map::new();
    let code_str = match err {
        TeamOpError::Board(BoardOpError::SelfReview { task_id, submitter, actor }) => {
            details.insert("task_id".into(), json!(task_id));
            details.insert("submitter".into(), json!(submitter));
            details.insert("actor".into(), json!(actor));
            code::TEAM_SELF_REVIEW
        }
        TeamOpError::Board(BoardOpError::NotOwner { task_id, owner, actor }) => {
            details.insert("task_id".into(), json!(task_id));
            details.insert("owner".into(), json!(owner));
            details.insert("actor".into(), json!(actor));
            code::TEAM_NOT_OWNER
        }
        TeamOpError::Board(BoardOpError::InvalidState { task_id, from, op }) => {
            details.insert("task_id".into(), json!(task_id));
            details.insert("from".into(), json!(from));
            details.insert("op".into(), json!(op));
            code::TEAM_STATE
        }
        TeamOpError::Board(BoardOpError::UnknownTask(task_id)) => {
            details.insert("task_id".into(), json!(task_id));
            code::TEAM_STATE
        }
        TeamOpError::UnknownActor(name) => {
            details.insert("actor".into(), json!(name));
            code::TEAM_UNKNOWN_MEMBER
        }
    };
    // D3 修复（云尺 E2E 实测 details:{} 空）：err_envelope 丢 details——
    // details 字段化必经 err_body_details 组装
    json!({"ok": false, "error": err_body_details(code_str, &err.to_string(), details)})
}

/// 写操作成功后的 tasks/changed 事件（增量提示：taskId/state/owner?）。
fn notify_task_changed(state: &Arc<WebState>, task_id: u64, state_name: &str, owner: Option<String>) {
    let mut event = json!({"type": "tasks/changed", "taskId": task_id, "state": state_name});
    if let Some(owner) = owner {
        event["owner"] = Value::String(owner);
    }
    state
        .wire
        .broadcast_event(crate::bridge::StreamKind::Events, event);
}

/// 状态名（serde lowercase 冻结面）。
fn state_name(task: &yunyou_team::TeamTask) -> String {
    serde_json::to_value(task.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// team/roster（L1）：名册投影（online 真源=hub 合成已在 service 内）。
pub(crate) fn team_roster(state: &Arc<WebState>) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let items: Vec<Value> = service
        .roster()
        .iter()
        .map(|entry| {
            let mut row = json!({
                "name": entry.name,
                "description": entry.description,
                "online": entry.online,
                "busy": entry.busy,
            });
            if let Some(task) = entry.task {
                row["task"] = json!(task);
            }
            row
        })
        .collect();
    json!({"ok": true, "value": {"items": items}})
}

/// tasks/list（L1）：全量投影（契约 §2.2-2 注——比薄工具面宽，看板需要
/// createdBy/submittedBy/reviewer/capability/blockedBy）。
pub(crate) fn tasks_list(state: &Arc<WebState>) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let items: Vec<Value> = service
        .task_list_full()
        .iter()
        .map(|task| {
            let mut row = json!({
                "id": task.id,
                "title": task.title,
                "state": state_name(task),
                "createdBy": task.created_by,
                "capability": task.capability,
                "blockedBy": task.blocked_by,
            });
            if let Some(owner) = &task.owner {
                row["owner"] = Value::String(owner.clone());
            }
            if let Some(submitted_by) = &task.submitted_by {
                row["submittedBy"] = Value::String(submitted_by.clone());
            }
            if let Some(reviewer) = &task.reviewer {
                row["reviewer"] = Value::String(reviewer.clone());
            }
            row
        })
        .collect();
    json!({"ok": true, "value": {"items": items}})
}

/// team/spawn（L4，需显式授权——前端二次确认）：payload
/// {name!,description!,prompt!,context?} → {name,description,note}。
pub(crate) async fn team_spawn(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(name) = request.get("name").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "spawn 缺 name（成员可寻址名）");
    };
    let Some(description) = request.get("description").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "spawn 缺 description（职责一句话）");
    };
    let Some(prompt) = request.get("prompt").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "spawn 缺 prompt（人设与任务指令）");
    };
    let context = match request.get("context").and_then(Value::as_str) {
        None | Some("fresh") => ContextMode::Fresh,
        Some("fork") => ContextMode::Fork,
        Some(other) => {
            return err_envelope(
                code::BAD_REQUEST,
                &format!("spawn context 需 fresh|fork，得到 {other}"),
            )
        }
    };
    match service
        .spawn_teammate(TeammateSpec {
            name: name.to_string(),
            description: description.to_string(),
            prompt: prompt.to_string(),
            context,
        })
        .await
    {
        Ok(spawned) => {
            // roster-changed（A 线补齐，websocket.md §4）：W3 漏项——名册增量
            // （spawn 上线）推流，前端收到后重拉 team/roster（单真源不轮询）
            state.wire.broadcast_event(
                crate::bridge::StreamKind::Events,
                json!({"type": "team/roster-changed", "name": spawned.name,
                       "online": true, "busy": false}),
            );
            json!({"ok": true, "value": {
                "name": spawned.name,
                "description": spawned.description,
                "note": spawned.note,
            }})
        }
        Err(err) => service_err(&err),
    }
}

/// team/message（L2）：以 lead 身份发成员消息。kind 只收 chat（缺省）——
/// report 语义=成员完工自动汇报（谁终结谁发送，lead 代发会被身份派发拦截），
/// 人话拒绝不冒发（域细则 §1.3 与 hub 身份模型的诚实对齐，备案云枢）。
pub(crate) fn team_message(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(to) = request.get("to").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "message 缺 to（成员名或 all）");
    };
    let Some(body) = request.get("body").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "message 缺 body");
    };
    match request.get("kind").and_then(Value::as_str) {
        None | Some("chat") => {}
        Some("report") => {
            return err_envelope(
                code::TEAM_STATE,
                "report 语义=成员完工自动汇报（机制发送），lead 不可代发；发正文请用 chat",
            )
        }
        Some(other) => {
            return err_envelope(code::BAD_REQUEST, &format!("message kind 需 chat|report，得到 {other}"))
        }
    }
    let address = if to == "all" {
        Address::All
    } else {
        Address::Member(to.to_string())
    };
    match service.send_message(address, MessageKind::Text, body.to_string()) {
        Ok(receipt) => json!({"ok": true, "value": {
            "to": to, "accepted": true, "logId": receipt.log_id,
            "recipients": receipt.recipients, "buffered": receipt.buffered,
        }}),
        Err(err) => service_err(&err),
    }
}

/// team/kick（L3，前端二次确认）：dispose 成员 fiber（信箱保留）。
pub(crate) async fn team_kick(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(name) = request.get("name").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "kick 缺 name（成员名）");
    };
    match service.kick(name).await {
        Ok(()) => {
            // roster-changed（A 线补齐）：kick 离场增量推流
            state.wire.broadcast_event(
                crate::bridge::StreamKind::Events,
                json!({"type": "team/roster-changed", "name": name,
                       "online": false, "busy": false}),
            );
            json!({"ok": true, "value": {"kicked": true}})
        }
        Err(err) => service_err(&err),
    }
}

/// tasks/create（L2）：payload {title!,description!,capability?,blocked_by?}
/// → {taskId}；成功即 tasks/changed。
pub(crate) fn tasks_create(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(title) = request.get("title").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "create 缺 title");
    };
    let Some(description) = request.get("description").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "create 缺 description");
    };
    let capability = request.get("capability").and_then(Value::as_str).unwrap_or("");
    let blocked_by: Vec<u64> = request
        .get("blocked_by")
        .and_then(Value::as_array)
        .map(|array| array.iter().filter_map(|v| v.as_u64()).collect())
        .unwrap_or_default();
    match service.task_create_full(
        title.to_string(),
        description.to_string(),
        capability.to_string(),
        blocked_by,
    ) {
        Ok(task_id) => {
            notify_task_changed(state, task_id, "pending", None);
            json!({"ok": true, "value": {"taskId": task_id}})
        }
        Err(err) => service_err(&err),
    }
}

/// tasks/claim（L2）：payload {taskId!,owner?}——owner 缺省=lead 代派视角；
/// 非 pending→team/state。
pub(crate) fn tasks_claim(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(task_id) = request.get("taskId").and_then(Value::as_u64) else {
        return err_envelope(code::BAD_REQUEST, "claim 缺 taskId");
    };
    let owner = request
        .get("owner")
        .and_then(Value::as_str)
        .unwrap_or("lead")
        .to_string();
    match service.task_claim(task_id, &owner) {
        Ok(()) => {
            notify_task_changed(state, task_id, "claimed", Some(owner));
            json!({"ok": true, "value": {"claimed": true}})
        }
        Err(err) => board_op_err(&err),
    }
}

/// tasks/submit（L2）：payload {taskId!}——actor=lead（Web 视角）；仅 owner。
pub(crate) fn tasks_submit(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(task_id) = request.get("taskId").and_then(Value::as_u64) else {
        return err_envelope(code::BAD_REQUEST, "submit 缺 taskId");
    };
    match service.task_submit(task_id, "lead") {
        Ok(()) => {
            notify_task_changed(state, task_id, "submitted", None);
            json!({"ok": true, "value": {"submitted": true}})
        }
        Err(err) => board_op_err(&err),
    }
}

/// tasks/review（L3，前端二次确认）：payload {taskId!,approve!}——板层强制
/// 拒绝自审（lead 不可绕过）。
pub(crate) fn tasks_review(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(task_id) = request.get("taskId").and_then(Value::as_u64) else {
        return err_envelope(code::BAD_REQUEST, "review 缺 taskId");
    };
    let Some(approve) = request.get("approve").and_then(Value::as_bool) else {
        return err_envelope(code::BAD_REQUEST, "review 缺 approve（布尔裁决）");
    };
    match service.task_review(task_id, "lead", approve) {
        Ok(()) => {
            let state_name = if approve { "approved" } else { "rejected" };
            notify_task_changed(state, task_id, state_name, None);
            json!({"ok": true, "value": {"approved": approve}})
        }
        Err(err) => board_op_err(&err),
    }
}

/// tasks/reassign（L3，前端二次确认）：payload {taskId!,newOwner!}。
pub(crate) fn tasks_reassign(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let Some(task_id) = request.get("taskId").and_then(Value::as_u64) else {
        return err_envelope(code::BAD_REQUEST, "reassign 缺 taskId");
    };
    let Some(new_owner) = request.get("newOwner").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "reassign 缺 newOwner");
    };
    match service.task_reassign(task_id, new_owner) {
        Ok(()) => {
            notify_task_changed(state, task_id, "claimed", Some(new_owner.to_string()));
            json!({"ok": true, "value": {"reassigned": true}})
        }
        Err(err) => board_op_err(&err),
    }
}

#[cfg(test)]
#[path = "rpc_team_tests.rs"]
pub(crate) mod rpc_team_tests;
