//! lead 侧工具族（S4 规格命名：spawn_teammate/send_message/team_task_*）。
//!
//! 工具无 Context 无身份（ToolExec 面）——全部经 `Arc<dyn TeamService>` 代理，
//! 发送者身份由服务按 fiber 上下文派发（防伪造定案在工具面的落点：模型自报
//! 无效，工具实现也无从伪造）。成员侧的身份化覆写见 member_tools.rs（同构
//! 工具类型以 actor 参数区分：None=lead 侧经服务、Some(名)=成员自己身份）。
//!
//! output_schema 全覆盖（契约⑥ S4 收敛声明：新工具必须声明 canonical 值形状）。

use crate::roster::RosterEntry;
pub(crate) use crate::task_tools::{TaskCreateTool, TaskClaimTool, TaskSubmitTool, TaskReviewTool, TaskReassignTool, TaskKickTool};
use crate::task_types::{ContextMode, TeamService, TeammateSpec};
use yunyou::hub::{Address, MessageKind, OutgoingMessage, SentReceipt};
use yunyou_tools::contract::{ToolError, ToolExec, ToolOutcome};
use yunyou_tools::ToolDefinition;
use yunyou_util::BoxFut;
use serde_json::{json, Value};
use std::sync::Arc;

/// lead 侧工具清单（plugin.rs setup 注册进全局注册表；service=团队服务句柄）。
pub(crate) fn lead_tools(service: Arc<dyn TeamService>) -> Vec<Arc<dyn ToolDefinition>> {
    vec![
        Arc::new(SpawnTeammateTool { service: Arc::clone(&service) }),
        Arc::new(SendTool { service: Arc::clone(&service), actor: None, member_ctx: None }),
        Arc::new(TaskCreateTool { service: Arc::clone(&service) }),
        Arc::new(TaskReassignTool { service: Arc::clone(&service) }),
        Arc::new(TaskClaimTool { actor: None, service: Arc::clone(&service) }),
        Arc::new(TaskSubmitTool { actor: None, service: Arc::clone(&service) }),
        Arc::new(TaskReviewTool { service: Arc::clone(&service) }),
        Arc::new(TaskKickTool { service: Arc::clone(&service) }),
        Arc::new(TaskListTool { service: Arc::clone(&service) }),
        Arc::new(RosterTool { service }),
    ]
}

/// CordisError → 工具业务失败（fail-loud 不吞错：消息原文上面板与模型面）。
pub(crate) fn team_err(code: &'static str, err: yunyou::CordisError) -> ToolOutcome {
    ToolOutcome::Err(ToolError::new(code, err.message()))
}

/// 解析 kind 参数（默认 text；非法值 fail-loud 不静默改）。
fn parse_kind(value: &Value) -> Result<MessageKind, ToolOutcome> {
    match value.as_str().unwrap_or("text") {
        "text" => Ok(MessageKind::Text),
        "system" => Ok(MessageKind::System),
        "report" => Ok(MessageKind::Report),
        other => Err(ToolOutcome::Err(ToolError::new(
            "team/kind",
            format!("未知消息类别 {other:?}（可选 text/system/report）"),
        ))),
    }
}

/// 回执投影（send 类工具共用；真账三数+人话注记）。
fn receipt_json(receipt: &SentReceipt) -> Value {
    json!({
        "log_id": receipt.log_id,
        "recipients": receipt.recipients,
        "buffered": receipt.buffered,
        "note": if receipt.buffered > 0 {
            "部分收件人离线，消息已缓冲待补投"
        } else {
            "全部收件人已受理"
        },
    })
}

// ── ① AI 自拉人（五点规格①） ─────────────────────────────────

struct SpawnTeammateTool {
    service: Arc<dyn TeamService>,
}

impl ToolDefinition for SpawnTeammateTool {
    fn name(&self) -> &str {
        "spawn_teammate"
    }

    fn description(&self) -> &str {
        "拉起一个团队成员：独立 fiber 运行自己的会话与回合循环，先预注册信箱再上线\
         （拉起窗口内发给它的消息不丢，上线后首次消费取净）。name 全局唯一（重名拒绝）；\
         context=fork 时注入 lead 对话摘要（一期降级形态）。"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "成员名（全局唯一，如 code-reviewer）"},
                "description": {"type": "string", "description": "职责一句话（名册可见）"},
                "prompt": {"type": "string", "description": "成员人设与任务指令"},
                "context": {"type": "string", "enum": ["fresh", "fork"], "description": "上下文形态，默认 fresh"},
            },
            "required": ["name", "description", "prompt"],
        })
    }

    fn output_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "description": {"type": "string"},
                "note": {"type": "string"},
            },
            "required": ["name", "description", "note"],
        }))
    }

    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let Some(name) = args.get("name").and_then(Value::as_str).map(String::from) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 name"));
            };
            // 名字白名单前置闸（task-18）：成员名流向会话文件路径
            //（teammate-{name}.jsonl），穿越形态在 hub 层之前先给模型可操作的
            // 人话拒绝（与 hub validate_member_name 同一规则单一源，双闸防漂移）。
            if let Err(err) = yunyou::validate_member_name(&name) {
                return ToolOutcome::err_h(
                    "team/args",
                    format!("spawn_teammate 拒绝：{err}"),
                    "换一个合法成员名重试：只含小写字母/数字/中划线/下划线（kebab-case，如 code-reviewer），长度上限 64",
                );
            }
            let Some(description) = args.get("description").and_then(Value::as_str).map(String::from)
            else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 description"));
            };
            let Some(prompt) = args.get("prompt").and_then(Value::as_str).map(String::from) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 prompt"));
            };
            let context = match args.get("context").and_then(Value::as_str) {
                Some("fork") => ContextMode::Fork,
                _ => ContextMode::Fresh,
            };
            let spec = TeammateSpec { name, description, prompt, context };
            match service.spawn_teammate(spec).await {
                Ok(spawned) => ToolOutcome::Ok(json!({
                    "name": spawned.name,
                    "description": spawned.description,
                    "note": spawned.note,
                })),
                Err(err) => team_err("team/spawn", err),
            }
        })
    }
}

// ── ② 发消息（五点规格②成员互联；lead 身份版+成员身份化覆写同构） ──

/// 发消息工具：actor=None=lead 侧（经 TeamService，身份=团队 fiber "lead"）；
/// actor=Some(名)=成员覆写（身份=成员 fiber；schema 不含身份参数）。
pub(crate) struct SendTool {
    pub(crate) service: Arc<dyn TeamService>,
    pub actor: Option<String>,
    /// 成员身份通道（Some=成员覆写：发消息走成员自己 fiber 上下文；
    /// None=lead 版走服务=团队 fiber）。身份收在通道里不可自报。
    pub member_ctx: Option<yunyou::WeakContext>,
}

impl ToolDefinition for SendTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        match &self.actor {
            None => "给团队成员发消息（to=成员名）或全员广播（to=\"All\"，不含自己）。\
                     发送者身份由内核 hub 按调用上下文派发（你是 lead），无需也不能自报 from；\
                     收件人离线时消息自动缓冲补投。",
            Some(_) => "给其他成员或 lead 发消息（to=成员名或 \"lead\"）或全员广播（to=\"All\"）。\
                        发送者身份由内核 hub 派发（你是该成员），无需也不能自报 from。",
        }
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {"type": "string", "description": "收件成员名，或 \"all\"（大小写不敏感）广播在册全体（不含自己；零成员人话报错）。\"all\" 为保留广播地址，成员不得取名 all"},
                "body": {"type": "string", "description": "消息正文"},
                "kind": {"type": "string", "enum": ["text", "system", "report"], "description": "类别，默认 text"},
            },
            "required": ["to", "body"],
        })
    }

    fn output_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "log_id": {"type": "integer"},
                "recipients": {"type": "integer"},
                "buffered": {"type": "integer"},
                "note": {"type": "string"},
            },
            "required": ["log_id", "recipients", "buffered", "note"],
        }))
    }

    fn execute(&self, args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        let member_ctx = self.member_ctx.clone();
        Box::pin(async move {
            let Some(to) = args.get("to").and_then(Value::as_str).map(String::from) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 to"));
            };
            let Some(body) = args.get("body").and_then(Value::as_str).map(String::from) else {
                return ToolOutcome::Err(ToolError::new("team/args", "缺少 body"));
            };
            let kind = match parse_kind(args.get("kind").unwrap_or(&Value::Null)) {
                Ok(kind) => kind,
                Err(outcome) => return outcome,
            };
            // 广播地址解析（task-30①，lead 预裁=hub 寻址 All 承诺兑现）：
            // "all"（大小写不敏感）为保留广播地址；其余按成员名点对点（在册
            // 校验归 hub 层）。保留字声明进 schema description——成员不得取名
            // "all"（kebab 白名单未拦，注册拦截记 follow-up）。
            let address = if to.eq_ignore_ascii_case("all") {
                Address::All
            } else {
                Address::Member(to)
            };
            // 成员覆写=走成员自己的 fiber 上下文（member_tools 注册的实例）；
            // lead 版=走团队 fiber（"lead" 身份）——身份都收在上下文里不可自报
            if let Some(member_ctx) = &member_ctx {
                let Some(ctx) = member_ctx.upgrade() else {
                    return ToolOutcome::Err(ToolError::new(
                        "team/offline",
                        "成员 fiber 已离场，无法发送",
                    ));
                };
                return match ctx.hub_send(address, OutgoingMessage { kind, body }) {
                    Ok(receipt) => ToolOutcome::Ok(receipt_json(&receipt)),
                    Err(err) => team_err("team/send", err),
                };
            }
            match service.send_message(address, kind, body) {
                Ok(receipt) => ToolOutcome::Ok(receipt_json(&receipt)),
                Err(err) => team_err("team/send", err),
            }
        })
    }
}

// ── ④ 观测面（只读；lead 与成员共用） ───────────────────────

struct TaskListTool {
    service: Arc<dyn TeamService>,
}

impl ToolDefinition for TaskListTool {
    fn name(&self) -> &str {
        "team_task_list"
    }
    fn description(&self) -> &str {
        "列出共享任务板全部任务（号/标题/状态/负责人）。"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"tasks": {"type": "array", "items": {"type": "object"}}}, "required": ["tasks"]}))
    }
    fn execute(&self, _args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let tasks = service.task_list();
            ToolOutcome::Ok(json!({ "tasks": tasks }))
        })
    }
}

/// 名册观测（五点规格⑤）：谁在跑/谁空闲/谁的活。
struct RosterTool {
    service: Arc<dyn TeamService>,
}

impl ToolDefinition for RosterTool {
    fn name(&self) -> &str {
        "team_roster"
    }
    fn description(&self) -> &str {
        "列出团队名册：成员/职责/在线态/在忙态/认领任务——派工与围观的第一入口。"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn output_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"members": {"type": "array", "items": {"type": "object"}}}, "required": ["members"]}))
    }
    fn execute(&self, _args: Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let service = Arc::clone(&self.service);
        Box::pin(async move {
            let members: Vec<Value> = service.roster().iter().map(roster_entry_json).collect();
            ToolOutcome::Ok(json!({"members": members}))
        })
    }
}

/// 名册条目投影（观测工具与 spawn 返回共用；RosterEntry JSON 单源）。
pub(crate) fn roster_entry_json(entry: &RosterEntry) -> Value {
    json!({
        "name": entry.name,
        "description": entry.description,
        "online": entry.online,
        "busy": entry.busy,
        "task": entry.task,
    })
}
