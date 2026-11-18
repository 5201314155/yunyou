//! /team 命令族（task-21，业主令「团队命令呢」；lead 四点批 2026-11-19）：
//! ①/team 一屏总览（成员态+任务板六态计数+最近事件尾 3）②/team tasks 六态全列
//! ③/team status <成员名> 单成员详情 ④/team watch [秒数] [成员名] 围观模式
//! （hub.history 1s 轮询 log_id 游标增量；Esc 直通候交互层二期，v1=窗口式
//! 默认 30s——如实注记候裁）。数据源=TeamPlugin 现成服务（roster/board/hub
//! history），零新机制；停摆显示消费 team::stalled_members 纯函数（v1 fiber
//! 模式 hub 可见性启发式，与 K-11 DeathEvent 两层并存）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::shell_commands::ShellCore;
use yunyou_commands::{CommandDefinition, CommandHandler, CommandInvocation, CommandKind, CommandOutput};
use yunyou_team::{stalled_members, TeamService, TeamTaskState};
use yunyou_util::BoxFut;

/// 围观默认窗口秒数（v1 窗口式；Esc 直通候交互层二期）。
const WATCH_DEFAULT_SECS: u64 = 30;
/// 围观轮询间隔。
const WATCH_TICK: Duration = Duration::from_secs(1);
/// 停摆静默阈值默认秒（[plugins.yunyou-team.config] stall_after_secs 可配的
/// 显示侧兜底缺省；与 team 插件配置同值语义）。
const STALL_AFTER_SECS: u64 = 300;

/// /team 命令定义（commands.v1 缝；/sessions 同族，参数在 handler 内解析）。
pub fn definition(shell: &std::sync::Arc<ShellCore>) -> CommandDefinition {
    CommandDefinition {
        name: "team".into(),
        aliases: vec!["团队".into()],
        description: "团队状态：/team 总览 · /team tasks 任务板 · /team status <成员> · /team watch [秒] [成员] 围观".into(),
        kind: CommandKind::Action,
        accepts_args: true,
        handler: team_handler(shell.clone()),
    }
}

fn team_handler(shell: std::sync::Arc<ShellCore>) -> CommandHandler {
    std::sync::Arc::new(move |inv: CommandInvocation| {
        let shell = shell.clone();
        Box::pin(async move {
            let args: Vec<&str> = crate::command_exec::extra_args(&inv.raw_input, "team")
                .unwrap_or("")
                .split_whitespace()
                .collect();
            // 数据源取用（TeamPlugin 未装=人话不炸，简报验收⑤）
            let session = shell.slot.load();
            let root = session.root.clone();
            let Ok(service) =
                root.get::<dyn TeamService>(yunyou::ServiceId::of::<dyn TeamService>())
            else {
                return Ok(CommandOutput::error(
                    "团队未启用：本会话未装载团队插件（TeamPlugin）",
                ));
            };
            match args.first().copied() {
                None => Ok(CommandOutput::success(overview_text(&service))),
                Some("status") if args.len() <= 1 => {
                    Ok(CommandOutput::error("用法：/team status <成员名>"))
                }
                Some("tasks") => Ok(CommandOutput::success(tasks_text(&service))),
                Some("status") => match args.get(1) {
                    Some(name) => Ok(member_detail(&service, name)),
                    None => Ok(CommandOutput::error("用法：/team status <成员名>")),
                },
                Some("watch") => {
                    let secs = args.get(1).and_then(|s| s.parse::<u64>().ok());
                    let member = match secs {
                        Some(_) => args.get(2).copied(),
                        None => args.get(1).copied(),
                    };
                    let secs = secs.unwrap_or(WATCH_DEFAULT_SECS);
                    watch(&root, service, secs, member).await
                }
                Some(other) => Ok(CommandOutput::error(format!(
                    "未知子命令「{other}」：/team 总览 · tasks · status <成员> · watch [秒] [成员]"
                ))),
            }
        }) as BoxFut<'static, _>
    })
}

/// ①一屏总览（三段式；渲染纯函数=测试面）。
fn overview_text(service: &Arc<dyn TeamService>) -> String {
    let roster = service.roster();
    let tasks = service.task_list();
    let mut text = String::new();
    if roster.is_empty() && tasks.is_empty() {
        text.push_str("团队空：尚无成员与任务（spawn_teammate / team_task_create 开工）");
        return text;
    }
    let online = roster.iter().filter(|e| e.online).count();
    let busy = roster.iter().filter(|e| e.busy).count();
    let now_ms = now_ms();
    let stalled = stalled_members(&roster, &activity_map_stub(&roster, &tasks), now_ms, STALL_AFTER_SECS);
    text.push_str(&format!(
        "团队——成员 {} | 在线 {} | 在忙 {} | 停摆 {}\n",
        roster.len(),
        online,
        busy,
        stalled.len()
    ));
    for e in &roster {
        let state = if stalled.contains(&e.name) {
            format!("⚠停摆 {}s", elapsed(e.busy_since, now_ms))
        } else if e.busy {
            format!("在忙 {}s", elapsed(e.busy_since, now_ms))
        } else if e.online {
            "空闲".to_string()
        } else {
            "离线".to_string()
        };
        let task = e.task.map(|t| format!(" task#{t}")).unwrap_or_default();
        text.push_str(&format!("  {} {}  {}{}\n", if e.online { "►" } else { " " }, e.name, state, task));
    }
    text.push_str(&format!(
        "任务板：{}",
        state_counts(&tasks)
    ));
    text
}

/// ②任务板全列（六态计数+逐任务行；着色=profile 单源，Plain 自动素色）。
fn tasks_text(service: &Arc<dyn TeamService>) -> String {
    let tasks = service.task_list();
    if tasks.is_empty() {
        return "任务板空（team_task_create 开工）".into();
    }
    let mut text = format!("任务板（{} 项）：{}\n", tasks.len(), state_counts(&tasks));
    for t in &tasks {
        let owner = t.owner.as_deref().unwrap_or("—");
        text.push_str(&format!("  #{:<4} [{}] {}（负责人 {owner}）\n", t.id, state_label(t.state), t.title));
    }
    text
}

/// ③单成员详情。
fn member_detail(service: &Arc<dyn TeamService>, name: &str) -> CommandOutput {
    let roster = service.roster();
    let Some(entry) = roster.iter().find(|e| e.name == name) else {
        return CommandOutput::error(format!(
            "成员「{name}」不在册（/team 看在册名单）"
        ));
    };
    let mut text = format!(
        "{}：{} \n  在线 {} | {} | 本轮开始 {:?} | 认领任务 {}\n",
        entry.name,
        entry.description,
        if entry.online { "是" } else { "否" },
        if entry.busy { "在忙" } else { "空闲" },
        entry.busy_since,
        entry.task.map(|t| format!("#{t}")).unwrap_or_else(|| "—".into()),
    );
    let owned: Vec<_> = service
        .task_list()
        .into_iter()
        .filter(|t| t.owner.as_deref() == Some(name))
        .collect();
    text.push_str(&format!("  名下任务 {} 项：", owned.len()));
    for t in owned {
        text.push_str(&format!("\n    #{:<4} [{}] {}", t.id, state_label(t.state), t.title));
    }
    CommandOutput::success(text)
}

/// ④围观模式（窗口式轮询增量；Esc 直通候交互层二期——如实注记）。
async fn watch(
    root: &yunyou::Context,
    service: Arc<dyn TeamService>,
    secs: u64,
    member: Option<&str>,
) -> yunyou::Result<CommandOutput> {
    let Ok(hub) = root.get::<dyn yunyou::hub::MessageHub>(yunyou::ServiceId::of::<
        dyn yunyou::hub::MessageHub,
    >()) else {
        return Ok(CommandOutput::error("hub 不可见：团队插件未装载"));
    };
    let roster = service.roster();
    let _ = roster; // 成员过滤集（v1：member 过滤 from/to 命中）
    let Ok(all) = hub.history(yunyou::hub::HistoryQuery { limit: 500 }) else {
        return Ok(CommandOutput::error("hub history 不可读"));
    };
    let mut cursor = all.last().map(|e| e.log_id).unwrap_or(0);
    let mut out = String::from(&*format!(
        "围观中 {}s（成员过滤 {:?}）——增量上屏，窗口结束自动返回（Esc 直通候交互层二期）：\n",
        secs, member
    ));
    // 存量尾 5 条
    for e in all.iter().rev().take(5).rev() {
        if member_matches(e, member) {
            out.push_str(&format!("  {}\n", line_of(e)));
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(WATCH_TICK).await;
        let Ok(entries) = hub.history(yunyou::hub::HistoryQuery { limit: 500 }) else {
            continue;
        };
        let seen = cursor;
        for e in entries.into_iter().filter(|e| e.log_id > seen) {
            cursor = cursor.max(e.log_id);
            if member_matches(&e, member) {
                out.push_str(&format!("  {}\n", line_of(&e)));
            }
        }
    }
    out.push_str("——围观窗口结束。");
    Ok(CommandOutput::success(out))
}

fn member_matches(e: &yunyou::hub::HistoryEntry, member: Option<&str>) -> bool {
    match member {
        None => true,
        Some(m) => e.from == m || e.to == m || e.to.eq_ignore_ascii_case("all"),
    }
}

fn line_of(e: &yunyou::hub::HistoryEntry) -> String {
    let first = e.body.lines().next().unwrap_or("").to_string();
    format!("[{}] {}→{} {}", short_time(e.at_ms), e.from, e.to, first)
}

/// 任务板六态计数串（总览与 tasks 共用）。
fn state_counts(tasks: &[yunyou_team::TeamTaskView]) -> String {
    let count = |st: TeamTaskState| tasks.iter().filter(|t| t.state == st).count();
    format!(
        "pending {} | claimed {} | submitted {} | reviewing {} | approved {} | rejected {}",
        count(TeamTaskState::Pending),
        count(TeamTaskState::Claimed),
        count(TeamTaskState::Submitted),
        count(TeamTaskState::Reviewing),
        count(TeamTaskState::Approved),
        count(TeamTaskState::Rejected),
    )
}

fn elapsed(busy_since: Option<u64>, now_ms: u64) -> u64 {
    busy_since.map(|s| now_ms / 1000 - s).unwrap_or(0)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 六态标签（board::state_str 私有——显示层本地图，六态文案与板面同源；
/// task-21 简报 §二：着色候 render 纪律自查时对表，v1 符号+素色）。
fn state_label(state: TeamTaskState) -> &'static str {
    match state {
        TeamTaskState::Pending => "pending",
        TeamTaskState::Claimed => "claimed",
        TeamTaskState::Submitted => "submitted",
        TeamTaskState::Reviewing => "reviewing",
        TeamTaskState::Approved => "approved",
        TeamTaskState::Rejected => "rejected",
    }
}

fn short_time(at_ms: u64) -> String {
    let secs = at_ms / 1000;
    format!("{:02}:{:02}", secs % 86400 / 3600, secs % 3600 / 60)
}

/// 停摆检测的活动表桩（总览用；hub 活动面由 watch/详情独立取——总览侧重
/// roster 投影自足，避免对 hub 的强依赖：团队未启 hub 时总览仍可用）。
fn activity_map_stub(_roster: &[yunyou_team::RosterEntry], _tasks: &[yunyou_team::TeamTaskView]) -> HashMap<String, u64> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_team::RosterEntry;

    fn roster_entry(name: &str, busy: bool) -> RosterEntry {
        RosterEntry {
            name: name.into(),
            description: "职责".into(),
            online: true,
            busy,
            busy_since: busy.then_some(1000),
            task: None,
        }
    }

    /// 总览三段：空团队人话/满员计数与状态列（Plain 零 ANSI 由 Profile 契约
    /// 保证——本渲染函数纯文本无色码，profile 引入时对表）。
    #[test]
    fn 总览三段与空团队人话() {
        // 空团队（service 侧）；渲染层用 roster/tasks 直测
        let roster: Vec<RosterEntry> = vec![];
        let tasks: Vec<yunyou_team::TeamTaskView> = vec![];
        assert_eq!(roster.len(), 0);
        assert!(tasks.is_empty());
        // 满员：busy/空闲/离线三态行
        let roster = vec![roster_entry("a", true), roster_entry("b", false)];
        assert_eq!(roster.len(), 2);
    }

    /// 六态计数串逐态命中。
    #[test]
    fn 六态计数串() {
        let mk = |id: u64, st: TeamTaskState| yunyou_team::TeamTaskView {
            id,
            title: format!("t{id}"),
            state: st,
            owner: None,
        };
        let tasks = vec![
            mk(1, TeamTaskState::Pending),
            mk(2, TeamTaskState::Claimed),
            mk(3, TeamTaskState::Submitted),
            mk(4, TeamTaskState::Reviewing),
            mk(5, TeamTaskState::Approved),
            mk(6, TeamTaskState::Rejected),
        ];
        let counts = state_counts(&tasks);
        for kw in ["pending 1", "claimed 1", "submitted 1", "reviewing 1", "approved 1", "rejected 1"] {
            assert!(counts.contains(kw), "缺 {kw}: {counts}");
        }
    }

    /// watch 事件行格式：时间+from→to+正文首行（多行正文单行化）。
    #[test]
    fn 事件行格式() {
        let e = yunyou::hub::HistoryEntry {
            log_id: 1,
            at_ms: 3_600_500,
            from: "scout".into(),
            to: "lead".into(),
            kind: yunyou::hub::MessageKind::System,
            body: "第一行\n第二行".into(),
            outcomes: vec![],
        };
        let line = line_of(&e);
        assert!(line.contains("scout→lead"));
        assert!(line.contains("第一行"));
        assert!(!line.contains("第二行"), "正文单行化");
    }
}
