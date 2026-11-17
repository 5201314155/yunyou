//! /sessions 会话列表与切换命令 + 会话回放渲染（task-36④，命令缝 D1）。
//!
//! 职责三分：
//! ①列表——扫 sessions 目录 mtime 倒序（`*.lock` 排除；`task-*` 显示但标
//!   [task] 且拒绝切换=单任务槽语义保护，task-36②）；
//! ②切换——走 `ShellCore::rebuild_session_to` 参数化缝（D3：teardown→重装配
//!   →换槽→壳命令重注册→清屏重印横幅→回放）；
//! ③回放——尾部 10 条 user/assistant 消息（tool/command/reasoning 不回放=
//!   已批口径；200 字单行化截断，不引宽表避开 task-33 display_width 挂账域）；
//! ④恢复扫描——`pick_latest_session`：启动期最近活跃会话选取（排除 task-*，
//!   --task 独立槽不参与恢复）。
//!
//! 落点纪律（task-19① 收编后）：回放符号与色彩走 render 域单源——`profile.symbol`
//! 降级体系（❯/☁ 有 ASCII 映射，Plain 契约闭环）+ `profile.color(SGR_TEXT_SECONDARY_256)`
//! （tokens 生成常量，废除裸 \x1b[90m）；不再自持色码开关，交互态判定由 Profile
//! 档位自身承担（NoColor/Plain 自动素色）；interactive 参数已随第二批删除
//! （调用点 run.rs/shell_commands.rs 同批同步）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::render::terminal::{ascii, Profile, SYM_BRAND, SYM_PROMPT};
use crate::shell_commands::ShellCore;
use yunyou_commands::{CommandDefinition, CommandHandler, CommandInvocation, CommandKind, CommandOutput};
use yunyou_session::SessionEvent;
use yunyou_util::BoxFut;

/// 回放条数上限（lead 已批 N=10）。
const REPLAY_LIMIT: usize = 10;
/// 回放单条正文截断字符数。
const REPLAY_TRUNCATE: usize = 200;

/// 会话目录条目（列表行数据）。
#[derive(Debug, Clone)]
pub(crate) struct SessionEntry {
    /// 会话文件完整路径。
    pub path: PathBuf,
    /// 文件名（展示用）。
    pub name: String,
    /// 最后修改时刻（mtime 倒序键）。
    pub modified: SystemTime,
    /// 是否单任务会话（`task-` 前缀；展示标 [task] 且拒绝切换）。
    pub is_task: bool,
}

/// /sessions 命令定义（走 register 缝；kind=Action，参数在 handler 内解析）。
pub fn definition(shell: &std::sync::Arc<ShellCore>) -> CommandDefinition {
    CommandDefinition {
        name: "sessions".into(),
        aliases: vec!["会话".into()],
        description: "列会话（无参）或切换（/sessions <序号>）".into(),
        kind: CommandKind::Action,
        accepts_args: true, // task-57：/sessions <序号> 参数语义（带参 Action）
        handler: sessions_handler(shell.clone()),
    }
}

/// /sessions 参数段提取（task-17① 纯函数）：token=斜杠后首空白段（主名/
/// 中文别名均适用），余段即参数——修复前 strip_prefix("/sessions") 硬编码
/// 主名，/会话 <序号> 的参数被静默丢弃走列表分支（挑刺 P1-6 实测）。
/// 提取与 command_exec::extra_args 同源复用。
fn sessions_arg(raw: &str) -> &str {
    let body = raw.strip_prefix('/').unwrap_or(raw);
    let token = body.split_whitespace().next().unwrap_or("");
    crate::command_exec::extra_args(raw, token).unwrap_or("").trim()
}

fn sessions_handler(shell: std::sync::Arc<ShellCore>) -> CommandHandler {
    std::sync::Arc::new(move |inv: CommandInvocation| {
        let shell = shell.clone();
        Box::pin(async move {
            let arg = sessions_arg(&inv.raw_input);
            if arg.is_empty() {
                return Ok(CommandOutput::success(list_text(&shell)));
            }
            match arg.parse::<usize>() {
                Ok(n) => switch_to(&shell, n).await,
                Err(_) => Ok(CommandOutput::error(
                    "用法：/sessions（列表）或 /sessions <序号>（切换到列表中的会话）",
                )),
            }
        }) as BoxFut<'static, _>
    })
}

/// 扫描会话目录（mtime 倒序；`*.jsonl` 且排除 `*.lock`；task- 条目带 is_task 标）。
pub(crate) fn list_sessions(session_dir: &Path) -> Vec<SessionEntry> {
    let Ok(rd) = std::fs::read_dir(session_dir) else {
        return Vec::new();
    };
    let mut out: Vec<SessionEntry> = rd
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "jsonl").unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let modified = e.metadata().ok()?.modified().ok()?;
            let is_task = name.starts_with("task-");
            Some(SessionEntry { path: e.path(), name, modified, is_task })
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.modified));
    out
}

/// 启动期最近活跃会话选取（task-36④恢复）：排除 task-* 单任务槽，取 mtime 最新。
pub(crate) fn pick_latest_session(home: &Path) -> Option<PathBuf> {
    resume_candidates(home).into_iter().next().map(|e| e.path)
}

/// 恢复候选（task-9）：常规会话（排除 task- 槽与点开头隐藏件）mtime 倒序——
/// `--continue` 取首条；`--resume` 交互选择展示本序。
pub(crate) fn resume_candidates(home: &Path) -> Vec<SessionEntry> {
    list_sessions(&home.join("sessions"))
        .into_iter()
        // task-59d：恢复扫描跳点开头（.hidden.jsonl 隐藏惯例；列表显示照旧）
        .filter(|e| !e.name.starts_with('.'))
        .filter(|e| !e.is_task)
        .collect()
}

/// `--resume <目标>` 解析（纯函数，task-9）：接受 会话文件名（含/不含 .jsonl）、
/// 完整路径、或列表序号（`--resume 2` = resume_candidates 第 2 条）——
/// 序号语义与 /sessions <序号> 同源，用户零新概念。
pub(crate) fn resolve_resume_target(
    target: &str,
    home: &Path,
) -> Result<PathBuf, String> {
    let candidates = resume_candidates(home);
    // ① 纯数字=列表序号（1 起）
    if let Ok(n) = target.parse::<usize>() {
        return candidates
            .get(n.saturating_sub(1))
            .map(|e| e.path.clone())
            .ok_or_else(|| format!("序号 {n} 不在历史会话列表中（候选 {n0} 条；yunyou --resume 查看全部)", n0 = candidates.len()));
    }
    // ② 完整路径（存在即收）
    let as_path = PathBuf::from(target);
    if as_path.is_file() {
        return Ok(as_path);
    }
    // ③ 文件名/名字干（含或不含 .jsonl）在候选里找
    let wanted = target.strip_suffix(".jsonl").unwrap_or(target);
    candidates
        .into_iter()
        .find(|e| {
            e.name == target
                || e.name.strip_suffix(".jsonl").is_some_and(|stem| stem == wanted)
        })
        .map(|e| e.path)
        .ok_or_else(|| format!("找不到会话「{target}」（yunyou --resume 交互选择，或 /sessions 查看现有会话）"))
}

/// `--resume` 交互选择（task-9，对齐 claude -r 无参形态）：列候选→读 stdin
/// 序号→回路径；Ctrl-D/EOF/空输入=放弃恢复（None=走全新会话）。
pub(crate) fn pick_interactively(home: &Path) -> Option<PathBuf> {
    let candidates = resume_candidates(home);
    if candidates.is_empty() {
        eprintln!("尚无历史会话（对话开始后自动落盘）——本次启动全新会话。");
        return None;
    }
    println!("选择要恢复的会话（回车放弃，启动全新会话）：");
    for (i, e) in candidates.iter().enumerate() {
        println!(
            "  {}  {}  {}  {}",
            i + 1,
            e.name,
            humanize_age(e.modified),
            humanize_size(&e.path)
        );
    }
    print!("序号: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        return None; // EOF（Ctrl-D）
    }
    match parse_pick(line.trim(), candidates.len()) {
        Some(idx) => Some(candidates[idx].path.clone()),
        None => None,
    }
}

/// 交互序号解析（纯函数）：空输入=放弃；1..=len 有效。
fn parse_pick(input: &str, len: usize) -> Option<usize> {
    if input.is_empty() {
        return None;
    }
    input
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=len).contains(n))
        .map(|n| n - 1)
}

/// 列表文本（mtime 倒序；当前会话标 ►；task- 标 [task]）。
fn list_text(shell: &std::sync::Arc<ShellCore>) -> String {
    let entries = list_sessions(&shell.session_dir);
    if entries.is_empty() {
        return "尚无历史会话（对话开始后自动落盘）".into();
    }
    let current = shell.current_session_path.lock().expect("会话路径锁").clone();
    let mut text = String::from("会话列表（最近在前；/sessions <序号> 切换）：");
    for (i, e) in entries.iter().enumerate() {
        let mark = if e.path == current { "►" } else { " " };
        let task_tag = if e.is_task { " [task]" } else { "" };
        text.push_str(&format!(
            "\n{} {}  {}{}  {}  {}",
            mark,
            i + 1,
            e.name,
            task_tag,
            humanize_age(e.modified),
            humanize_size(&e.path)
        ));
    }
    text
}

/// 切换到列表第 n 项（1 起）；task- 条目拒绝（单任务槽语义保护）。
async fn switch_to(
    shell: &std::sync::Arc<ShellCore>,
    n: usize,
) -> yunyou::Result<CommandOutput> {
    let entries = list_sessions(&shell.session_dir);
    let Some(entry) = entries.get(n.saturating_sub(1)) else {
        return Ok(CommandOutput::error(format!(
            "序号 {n} 不在列表中（/sessions 查看有效序号）"
        )));
    };
    if entry.is_task {
        return Ok(CommandOutput::error(
            "单任务会话不参与切换（--task 一次性语义，task-36②）",
        ));
    }
    if entry.path == *shell.current_session_path.lock().expect("会话路径锁") {
        return Ok(CommandOutput::success(format!(
            "已是当前会话：{}",
            entry.name
        )));
    }
    shell.rebuild_session_to(Some(entry.path.clone())).await
}

/// 启动/切换后的尾部回放（run.rs 横幅行后、rebuild_session_to 重印横幅后调用）。
///
/// 返回实际回放条数（0=无可回放对话，调用方据此决定是否打提示）。
pub fn render_replay(
    events: &[SessionEvent],
    out: &mut impl Write,
    profile: Profile,
) -> std::io::Result<usize> {
    let dialogue: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                SessionEvent::UserMessage { .. } | SessionEvent::AssistantMessage { .. }
            )
        })
        .collect();
    let start = dialogue.len().saturating_sub(REPLAY_LIMIT);
    let mut count = 0usize;
    for e in &dialogue[start..] {
        // 前缀区分发言方（task-19① 收编定稿）：user ❯（输入语义符，与 §4.2
        // 提示符同源）/ assistant ☁（品牌符，§4.13 恢复头同源）——既有语义符
        // 复用而非新增 ▸◂：降级链现成（❯→>、☁→YunYou），Plain 档 ASCII 契约
        // 直接闭环，不往全局 symbol 表加端上专用键。
        let (prefix, tag, text) = match e {
            SessionEvent::UserMessage { text } => (profile.symbol(SYM_PROMPT, ascii::PROMPT), "你", text),
            SessionEvent::AssistantMessage { text, .. } => {
                (profile.symbol(SYM_BRAND, ascii::BRAND), "云悠", text)
            }
            _ => unreachable!("上方 filter 已限定两变体"),
        };
        let mut shown: String = text.chars().take(REPLAY_TRUNCATE).collect();
        if text.chars().count() > REPLAY_TRUNCATE {
            shown.push('…');
        }
        // 多行消息单行化：回放为摘要形态，正文换行折行会破列表式观感
        if shown.contains('\n') {
            shown = shown.replace('\n', " ");
        }
        // 发言方段（符+称谓+冒号）烟灰单源色，正文素色（摘要层级；Full 走
        // tokens 38;5;245，NoColor/Plain 由 profile 自动剥——废除裸 \x1b[90m）
        let sgr = profile.color(crate::render::terminal::SGR_TEXT_SECONDARY_256);
        writeln!(out, "{sgr}{prefix} {tag}：{}{shown}", profile.reset())?;
        count += 1;
    }
    out.flush()?;
    Ok(count)
}

/// mtime 人话（刚刚/N 分钟前/N 小时前/N 天前）。
fn humanize_age(modified: SystemTime) -> String {
    let then = modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let delta = now.saturating_sub(then);
    match delta {
        0..=59 => "刚刚".into(),
        60..=3599 => format!("{} 分钟前", delta / 60),
        3600..=86399 => format!("{} 小时前", delta / 3600),
        _ => format!("{} 天前", delta / 86400),
    }
}

/// 文件大小人话（B/KB）。
fn humanize_size(path: &Path) -> String {
    match std::fs::metadata(path).map(|m| m.len()) {
        Ok(len) if len >= 1024 => format!("{}KB", len / 1024),
        Ok(len) => format!("{len}B"),
        Err(_) => "?".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunyou-sessions-cmd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 列表扫描：mtime 倒序+*.lock 排除+task- 带 is_task 标。
    #[test]
    fn 列表倒序与过滤() {
        let dir = tmp_dir("list");
        std::fs::write(dir.join("old.jsonl"), "{}").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(dir.join("task-123-456.jsonl"), "{}").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(dir.join("new.jsonl"), "{}").unwrap();
        std::fs::write(dir.join("new.jsonl.lock"), "").unwrap();
        let entries = list_sessions(&dir);
        assert_eq!(entries.len(), 3, "三个 jsonl（.lock 不入列）");
        assert_eq!(entries[0].name, "new.jsonl", "mtime 倒序");
        assert_eq!(entries[2].name, "old.jsonl");
        assert!(entries[1].is_task, "task- 条目带标");
        assert!(!entries[0].is_task);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 恢复扫描：跳过 task-* 取最近常规会话；空目录 None。
    #[test]
    fn 恢复扫描排除task槽() {
        let dir = tmp_dir("pick");
        assert_eq!(pick_latest_session(&dir), None, "空目录无恢复");
        let sessions = dir.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("task-1-2.jsonl"), "{}").unwrap();
        assert_eq!(pick_latest_session(&dir), None, "仅 task 槽不恢复");
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(sessions.join("default.jsonl"), "{}").unwrap();
        assert_eq!(
            pick_latest_session(&dir),
            Some(sessions.join("default.jsonl")),
            "常规会话被选中"
        );
        // task-59d：点开头文件不参与恢复（隐藏惯例），即便 mtime 最新
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(sessions.join(".hidden.jsonl"), "{}").unwrap();
        assert_eq!(
            pick_latest_session(&dir),
            Some(sessions.join("default.jsonl")),
            "点开头文件不入恢复扫描（task-59d）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 回放：只取 user/assistant 尾部 10 条、200 字截断、多行单行化、tool/command 不入。
    #[test]
    fn 回放尾部截取与截断() {
        let mut events = Vec::new();
        for i in 0..15 {
            events.push(SessionEvent::UserMessage { text: format!("u{i}") });
            events.push(SessionEvent::ToolCall {
                call_id: format!("c{i}"),
                name: "bash".into(),
                arguments: "{}".into(),
                parent_tool_call_id: None,
                batch_id: None,
            });
            events.push(SessionEvent::AssistantMessage { text: format!("a{i}"), reasoning: None });
        }
        events.push(SessionEvent::CommandRun {
            command_id: "x".into(),
            name: "help".into(),
            raw_input: "/help".into(),
        });
        let mut buf: Vec<u8> = Vec::new();
        let n = render_replay(&events, &mut buf, Profile::Plain).unwrap();
        assert_eq!(n, 10, "尾部 10 条对话消息");
        let text = String::from_utf8(buf).unwrap();
        assert!(!text.contains("u9"), "头部消息被截掉");
        assert!(text.contains("u10"), "尾部 10 条从 u10 起（30 对话条取尾 10）");
        assert!(text.contains("a14"), "末条在位");
        assert!(!text.contains("bash"), "tool 事件不回放");
        assert!(!text.contains("/help"), "command 事件不回放");

        // 截断+单行化
        let long = SessionEvent::UserMessage { text: "长".repeat(300) + "\n第二行" };
        let mut buf2: Vec<u8> = Vec::new();
        render_replay(&[long], &mut buf2, Profile::Plain).unwrap();
        let t2 = String::from_utf8(buf2).unwrap();
        assert!(t2.contains('…'), "超长带省略号");
        assert_eq!(t2.lines().count(), 1, "正文单行化：仅 writeln 行尾一个换行");
        assert_eq!(t2.matches('长').count(), 200, "恰 200 字");
    }

    use super::sessions_arg;

    /// task-17①：参数提取别名感知——主名/中文别名/多参数/无参四态。
    #[test]
    fn 参数提取别名感知() {
        assert_eq!(sessions_arg("/sessions 2"), "2", "主名带参");
        assert_eq!(sessions_arg("/会话 2"), "2", "中文别名带参（修复前恒空）");
        assert_eq!(sessions_arg("/sessions"), "", "主名无参");
        assert_eq!(sessions_arg("/会话"), "", "别名无参");
        assert_eq!(sessions_arg("/sessions abc"), "abc", "非数字原样上抛由 parse 报用法");
        assert_eq!(sessions_arg("/会话  3 "), "3", "多空格容忍");
    }

    /// task-19① 收编锁：回放前缀走 profile.symbol 降级体系——Full 出 ❯/☁ 与
    /// tokens 烟灰 245；Plain 出 ASCII（>/YunYou）且零 ANSI 字节（裸 \x1b[90m
    /// 与 ▸◂ 无降级的破口不得回潮）。
    #[test]
    fn 回放符号走降级体系_纯色_245_收编锁() {
        use crate::render::Profile as P;
        let events = vec![
            SessionEvent::UserMessage { text: "你好".into() },
            SessionEvent::AssistantMessage { text: "在的".into(), reasoning: None },
        ];
        // Plain：ASCII 前缀+零转义
        let mut plain: Vec<u8> = Vec::new();
        render_replay(&events, &mut plain, P::Plain).unwrap();
        let t = String::from_utf8(plain).unwrap();
        assert!(t.contains("> 你：你好"), "Plain 应出 ASCII 降级符：{t:?}");
        assert!(t.contains("YunYou 云悠：在的"), "Plain 品牌符应降级 YunYou：{t:?}");
        assert!(!t.contains('\x1b'), "Plain 档零 ANSI 字节：{t:?}");
        // Full：❯/☁ 语义符+烟灰 245（profile.color 单源）
        let mut full: Vec<u8> = Vec::new();
        render_replay(&events, &mut full, P::Full).unwrap();
        let f = String::from_utf8(full).unwrap();
        assert!(f.contains("\x1b[38;5;245m❯ 你："), "Full 应出 ❯+245 单源色：{f:?}");
        assert!(f.contains("☁ 云悠："), "Full 应出 ☁ 品牌符：{f:?}");
        assert!(!f.contains("\x1b[90m"), "裸 16 色 SGR 90 不得回潮：{f:?}");
    }

    /// task-9：恢复候选=常规会话 mtime 倒序（task- 槽与点开头隐藏件不入）。
    #[test]
    fn 恢复候选排除task与隐藏件() {
        let dir = tmp_dir("cand");
        let sessions = dir.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("a.jsonl"), "{}").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(sessions.join("b.jsonl"), "{}").unwrap();
        std::fs::write(sessions.join("task-1-2.jsonl"), "{}").unwrap();
        std::fs::write(sessions.join(".h.jsonl"), "{}").unwrap();
        let cands = resume_candidates(&dir);
        let names: Vec<&str> = cands.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["b.jsonl", "a.jsonl"], "仅常规会话且 mtime 倒序: {names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-9：--resume 目标解析四态——序号/完整路径/文件名（含与不含 .jsonl）/未知名报错。
    #[test]
    fn 恢复目标解析四态() {
        let dir = tmp_dir("resolve");
        let sessions = dir.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("alpha.jsonl"), "{}").unwrap();
        // 序号（1 起）
        assert_eq!(
            resolve_resume_target("1", &dir).unwrap(),
            sessions.join("alpha.jsonl"),
            "序号 1 = 候选首条"
        );
        // 完整路径
        assert_eq!(
            resolve_resume_target(sessions.join("alpha.jsonl").to_str().unwrap(), &dir).unwrap(),
            sessions.join("alpha.jsonl")
        );
        // 文件名与名字干
        assert_eq!(resolve_resume_target("alpha.jsonl", &dir).unwrap(), sessions.join("alpha.jsonl"));
        assert_eq!(resolve_resume_target("alpha", &dir).unwrap(), sessions.join("alpha.jsonl"));
        // 未知名人话报错
        assert!(resolve_resume_target("ghost", &dir).unwrap_err().contains("找不到会话"));
        assert!(resolve_resume_target("9", &dir).unwrap_err().contains("不在历史会话列表"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-9：交互序号解析——空=放弃、越界拒绝、有效值回 0 基下标。
    #[test]
    fn 交互序号解析() {
        assert_eq!(parse_pick("", 3), None, "空输入=放弃恢复");
        assert_eq!(parse_pick("abc", 3), None, "非数字=放弃");
        assert_eq!(parse_pick("4", 3), None, "越界=放弃");
        assert_eq!(parse_pick("2", 3), Some(1), "1 起序号转 0 基下标");
    }
}
