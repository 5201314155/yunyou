//! scheduler.v1 S3 三工具样板（池 task-14，设计 docs/设计/scheduler.v1-定时触发缝设计-20261118.md §四）。
//!
//! 模型交互面三纪律（设计原文）：①工具描述注明「创建前先从环境获知当前时间」；
//! ②回显即确认（结果含下一次触发时刻+注入消息全文）；③durable 语义照抄 claude——
//! 「仅当用户明确要求跨重启保留时才置 true」（模型擅自置 true=审批链可拦，
//! 审批词汇开闸前=提示词纪律+参数描述强制）。
//!
//! 到期投递：真会话投递缝=proto v1.1 开闸项（设计 §五）；当前到期动作=stderr
//! 人话（宿主 drain 透传主板日志留痕），S4 durable 持久化随投递缝批。

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use yunyou_plugin_sdk::scheduler::{ScheduleSpec, Scheduler, SchedulerTrigger, ScheduleEntry};
use yunyou_plugin_sdk::{tool, Plugin, ToolFailure};

/// schedule_create 参数（三选一必填恰一；设计 §四）。
#[derive(Deserialize, JsonSchema)]
struct CreateArgs {
    /// 一次性：ISO8601 或 epoch 毫秒（与 every_minutes/cron 三选一，必填恰一）
    #[serde(default)]
    at_ms: Option<u64>,
    /// 循环间隔分钟（≥5；与 at_ms/cron 三选一，必填恰一）
    #[serde(default)]
    every_minutes: Option<u64>,
    /// cron 表达式（5 段标准子集：分 时 日 月 周；与 at_ms/every_minutes 三选一，必填恰一）
    #[serde(default)]
    cron: Option<String>,
    /// 到期注入的消息全文（必填；用户会看到这句话）
    message: String,
    /// 仅当用户明确要求跨重启保留时才置 true（缺省 false=会话内有效）
    #[serde(default)]
    durable: bool,
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn main() -> std::process::ExitCode {
    let sched = Arc::new(Scheduler::new(epoch_ms));
    // 到期执行体（v1=stderr 留痕占位；真会话投递=proto v1.1 开闸后换注入）
    sched.set_trigger(Arc::new(StdoutTrigger));
    sched.spawn_thread();

    let sched_create = Arc::clone(&sched);
    let sched_list = Arc::clone(&sched);
    let sched_delete = Arc::clone(&sched);

    Plugin::new("scheduler", "0.1.0")
        .category("workflows")
        .description("定时任务：创建/查看/删除（模型自主选择时间）")
        .tool(tool(
            "schedule_create",
            "创建定时任务：到点把 message 注入会话提醒用户。创建前先从环境获知当前时间（结果回显 now 辅助换算「1 小时后」等相对时间）。durable 仅当用户明确要求跨重启保留时才置 true。",
            move |args: CreateArgs| {
                let spec = match (&args.at_ms, &args.every_minutes, &args.cron) {
                    (Some(at), None, None) => ScheduleSpec::At { at_ms: *at },
                    (None, Some(mins), None) => {
                        if *mins < 5 {
                            return Err(ToolFailure::with_hint(
                                "tool/invalid-args",
                                format!("间隔 {mins} 分钟低于下限 5 分钟（防过密循环打爆会话）"),
                                "把 every_minutes 提到 ≥5",
                            ));
                        }
                        ScheduleSpec::Interval { every_ms: mins * 60_000 }
                    }
                    (None, None, Some(expr)) => ScheduleSpec::Cron { expr: expr.clone() },
                    _ => {
                        return Err(ToolFailure::with_hint(
                            "tool/invalid-args",
                            "at_ms / every_minutes / cron 三选一，必填恰一",
                            "只填其中一个字段，其余两个省略",
                        ));
                    }
                };
                if args.durable {
                    // durable 纪律（设计 §四③）：模型擅自置 true 的防线=参数描述+
                    // 此处回显警示（审批联动候审批词汇开闸）
                    eprintln!("[scheduler] durable=true 创建请求（须用户明说；审批联动候开闸）");
                }
                match sched_create.register_schedule(None, spec, args.message.clone()) {
                    Ok((id, next)) => Ok(json!({
                        "id": id,
                        "next_fire_ms": next,
                        "now_ms": epoch_ms(),
                        "message": args.message,
                        "durable": args.durable,
                        "note": "已注册（回显即确认：请向用户复述下一次触发时刻与注入消息全文）",
                    })),
                    Err(err) => Err(ToolFailure::new("tool/schedule", err.message)),
                }
            },
        ))
        .tool(tool(
            "schedule_list",
            "列出在规定时任务（id/规格/下次触发时刻）",
            move |_args: EmptyArgs| {
                let rows: Vec<serde_json::Value> = sched_list
                    .list()
                    .into_iter()
                    .map(|(e, next)| {
                        json!({ "id": e.id, "spec": e.spec, "next_fire_ms": next, "message": e.message })
                    })
                    .collect();
                Ok(json!({ "entries": rows, "now_ms": epoch_ms() }))
            },
        ))
        .tool(tool(
            "schedule_delete",
            "按 id 删除定时任务（不存在=报错并列出现有清单）",
            move |args: DeleteArgs| match sched_delete.unregister_schedule(&args.id) {
                Ok(()) => Ok(json!({ "deleted": args.id })),
                Err(err) => {
                    let existing: Vec<String> =
                        sched_delete.list().into_iter().map(|(e, _)| e.id).collect();
                    Err(ToolFailure::with_hint("tool/schedule", err.message, format!("现有：{existing:?}")))
                }
            },
        ))
        .serve()
}

/// 空参数（list 用；schemars 派生空 object schema）。
#[derive(Deserialize, JsonSchema)]
struct EmptyArgs {}

/// schedule_delete 参数。
#[derive(Deserialize, JsonSchema)]
struct DeleteArgs {
    /// 要删除的调度 id（schedule_list 返回的 id）
    id: String,
}

/// v1 到期执行体：stderr 留痕（宿主 drain 透传主板日志）；真投递候缝。
struct StdoutTrigger;

impl SchedulerTrigger for StdoutTrigger {
    fn name(&self) -> &str {
        "stderr-echo"
    }

    fn on_fire(&self, entry: &ScheduleEntry, fired_at_ms: u64) -> Result<(), String> {
        let text = entry
            .message
            .replace("{id}", &entry.id)
            .replace("{fired_at}", &fired_at_ms.to_string());
        eprintln!("[scheduler][定时触发 {}] {text}", entry.id);
        Ok(())
    }
}
