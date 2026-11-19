//! 任务板操作工具族（五点规格④+主 agent 踢人）：create/claim/complete/
//! reassign 四操作+kick 离场。actor 语义：None=lead 侧（owner 显式传参/改派
//! lead 专属），Some(名)=成员身份化覆写（member_tools.rs 注册时烘焙）。
//!
//! output_schema 全覆盖（契约⑥ S4 收敛声明）。

use crate::task_types::TeamService;
use yunyou_tools::contract::{ToolError, ToolExec, ToolOutcome};
use yunyou_tools::ToolDefinition;
use yunyou_util::BoxFut;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) struct TaskCreateTool {
    pub(crate) service: Arc<dyn TeamService>,
}

impl ToolDefinition for TaskCreateTool {
    fn name(&self) -> &str {
        "team_task_create"
    }
    fn description(&self) -> &str {
        "在共享任务板创建任务（created_by=lead）。成员用 team_task_claim 认领。\
         自动通知：成员提交送审时系统自动通知你审查（无需成员另行发消息）；\
         前置任务被 approve 时系统自动通知被阻塞任务的负责人解锁。"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "任务标题"},
                "description": {"type": "string", "description": "详情与验收标准"},
                "capability": {"type": "string", "description": "capability 标签（自动领取匹配用，可空）"},
                "blocked_by": {"type": "array", "items": {"type": "integer"}, "description": "阻塞本任务的前置任务号列表"},
            },
            "required": ["title", "description"],
        })
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"task_id": {"type": "integer"}}, "required": ["task_id"]}))
    }
    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let Some(title) = args.get("title").and_then(Value::as_str).map(String::from) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 title"));
            };
            let Some(description) = args.get("description").and_then(Value::as_str).map(String::from)
            else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 description"));
            };
            let capability = args.get("capability").and_then(Value::as_str).unwrap_or("").to_string();
            let blocked_by: Vec<u64> = args
                .get("blocked_by")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(|v| v.as_u64()).collect())
                .unwrap_or_default();
            match service.task_create_full(title, description, capability, blocked_by) {
                Ok(task_id) => ToolOutcome::Ok(json!({"task_id": task_id})),
                Err(err) => ToolOutcome::Err(ToolError::new("team/task", err.to_string())),
            }
        })
    }
}

/// 认领工具：actor=None=lead 侧（owner 显式传参）；Some(名)=成员覆写（自己认领）。
pub(crate) struct TaskClaimTool {
    pub actor: Option<String>,
    pub service: Arc<dyn TeamService>,
}

impl ToolDefinition for TaskClaimTool {
    fn name(&self) -> &str {
        "team_task_claim"
    }
    fn description(&self) -> &str {
        match &self.actor {
            None => "认领任务板上的任务到指定成员名下（owner 落真实成员名，pending→claimed）。",
            Some(_) => "认领一个任务到自己名下（pending→claimed；非 pending 不可认领）。\
                        自动通知：该任务的前置任务被 approve 时，你会自动收到「前置已通过、任务解锁」系统通知（无需轮询任务板）。",
        }
    }
    fn parameters(&self) -> Value {
        if self.actor.is_some() {
            json!({"type": "object", "properties": {"task_id": {"type": "integer", "description": "任务板号"}}, "required": ["task_id"]})
        } else {
            json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "integer", "description": "任务板号"},
                    "owner": {"type": "string", "description": "认领成员名（须在册）"},
                },
                "required": ["task_id", "owner"],
            })
        }
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"claimed": {"type": "boolean"}}, "required": ["claimed"]}))
    }
    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let actor_override = self.actor.clone();
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let Some(task_id) = args.get("task_id").and_then(Value::as_u64) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 task_id"));
            };
            let actor = match &actor_override {
                Some(name) => name.clone(),
                None => match args.get("owner").and_then(Value::as_str).map(String::from) {
                    Some(owner) => owner,
                    None => return ToolOutcome::Err(ToolError::new("team/args", "缺少 owner")),
                },
            };
            match service.task_claim(task_id, &actor) {
                Ok(()) => ToolOutcome::Ok(json!({"claimed": true})),
                Err(err) => ToolOutcome::Err(ToolError::new("team/task", err.to_string())),
            }
        })
    }
}

/// 提交成果工具：actor 语义同认领（submit 仅 owner 本人；claimed→submitted）。
pub(crate) struct TaskSubmitTool {
    pub actor: Option<String>,
    pub service: Arc<dyn TeamService>,
}

impl ToolDefinition for TaskSubmitTool {
    fn name(&self) -> &str {
        "team_task_submit"
    }
    fn description(&self) -> &str {
        match &self.actor {
            None => "把认领的任务提交送审（仅任务负责人本人；claimed→submitted，待审查人裁决）。\
                     自动通知：提交后系统自动通知 lead 审查（自动送审，无需再发消息）。",
            Some(_) => "把自己名下已完成的任务提交送审（claimed→submitted；审查人裁决前状态不变）。\
                        自动通知：提交后系统自动通知 lead 审查（自动送审，无需再发消息）。",
        }
    }
    fn parameters(&self) -> Value {
        if self.actor.is_some() {
            json!({"type": "object", "properties": {"task_id": {"type": "integer", "description": "任务板号"}}, "required": ["task_id"]})
        } else {
            json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "integer", "description": "任务板号"},
                    "owner": {"type": "string", "description": "操作者成员名（须与任务负责人一致）"},
                },
                "required": ["task_id", "owner"],
            })
        }
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"submitted": {"type": "boolean"}}, "required": ["submitted"]}))
    }
    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let actor_override = self.actor.clone();
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let Some(task_id) = args.get("task_id").and_then(Value::as_u64) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 task_id"));
            };
            let actor = match &actor_override {
                Some(name) => name.clone(),
                None => match args.get("owner").and_then(Value::as_str).map(String::from) {
                    Some(owner) => owner,
                    None => return ToolOutcome::Err(ToolError::new("team/args", "缺少 owner")),
                },
            };
            match service.task_submit(task_id, &actor) {
                Ok(()) => ToolOutcome::Ok(json!({"submitted": true})),
                Err(err) => ToolOutcome::Err(ToolError::new("team/task", err.to_string())),
            }
        })
    }
}

/// 审查裁决工具（lead 专属，写审分离 K-8：审查人=提交人时系统拒绝）。
pub(crate) struct TaskReviewTool {
    pub service: Arc<dyn TeamService>,
}

impl ToolDefinition for TaskReviewTool {
    fn name(&self) -> &str {
        "team_task_review"
    }
    fn description(&self) -> &str {
        "审查裁决成员提交的任务：approve=通过（终态）/reject=驳回（终态）。\
         写审分离系统级强制：审查人=提交人（含 lead 自审自己的活）一律被系统拒绝。\
         自动通知：approve 后系统自动通知被阻塞任务的 owner「前置已通过」（owner 下一回合即可推进，无需你人工转达）；reject 不触发解锁通知。"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "integer", "description": "任务板号"},
                "decision": {"type": "string", "enum": ["approve", "reject"], "description": "裁决"},
            },
            "required": ["task_id", "decision"],
        })
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"state": {"type": "string"}}, "required": ["state"]}))
    }
    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let Some(task_id) = args.get("task_id").and_then(Value::as_u64) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 task_id"));
            };
            let approve = match args.get("decision").and_then(Value::as_str) {
                Some("approve") => true,
                Some("reject") => false,
                _ => return ToolOutcome::Err(ToolError::new("team/args", "decision 须为 approve 或 reject")),
            };
            // 审查人=lead（工具 lead 专属；板层另有 actor≠提交人的系统级守卫）
            match service.task_review(task_id, "lead", approve) {
                Ok(()) => ToolOutcome::Ok(json!({"state": if approve { "approved" } else { "rejected" }})),
                Err(err) => ToolOutcome::Err(ToolError::new("team/task", err.to_string())),
            }
        })
    }
}

pub(crate) struct TaskReassignTool {
    pub(crate) service: Arc<dyn TeamService>,
}

impl ToolDefinition for TaskReassignTool {
    fn name(&self) -> &str {
        "team_task_reassign"
    }
    fn description(&self) -> &str {
        "改派任务到新负责人（改派权=派工权，lead 专属；成员请走消息面向 lead 提请）。"
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "integer", "description": "任务板号"},
                "new_owner": {"type": "string", "description": "新负责人成员名（须在册）"},
            },
            "required": ["task_id", "new_owner"],
        })
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"reassigned": {"type": "boolean"}}, "required": ["reassigned"]}))
    }
    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let Some(task_id) = args.get("task_id").and_then(Value::as_u64) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 task_id"));
            };
            let Some(new_owner) = args.get("new_owner").and_then(Value::as_str).map(String::from)
            else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 new_owner"));
            };
            match service.task_reassign(task_id, &new_owner) {
                Ok(()) => ToolOutcome::Ok(json!({"reassigned": true})),
                Err(err) => ToolOutcome::Err(ToolError::new("team/task", err.to_string())),
            }
        })
    }
}

/// 踢人（主 agent 管理/踢人，业主架构；信箱保留 Pending 可重拉续投——hub 降级
/// 语义直接复用，成员 fiber dispose 即离场）。
pub(crate) struct TaskKickTool {
    pub(crate) service: Arc<dyn TeamService>,
}

impl ToolDefinition for TaskKickTool {
    fn name(&self) -> &str {
        "team_kick"
    }
    fn description(&self) -> &str {
        "让一个成员离场（dispose 其 fiber）：信箱与未取净消息保留（Pending），         同名重新 spawn 即续投。lead 专属管理动作。"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {"name": {"type": "string", "description": "成员名"}}, "required": ["name"]})
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"kicked": {"type": "boolean"}}, "required": ["kicked"]}))
    }
    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let Some(name) = args.get("name").and_then(Value::as_str).map(String::from) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 name"));
            };
            match service.kick(&name).await {
                Ok(()) => ToolOutcome::Ok(json!({"kicked": true})),
                Err(err) => crate::tools::team_err("team/kick", err),
            }
        })
    }
}

