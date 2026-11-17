//! tool_result 详情展开块（规范 §4.9 bash 摘要 / §4.10 文件折叠 / §4.11 diff）。
//!
//! 展开是**新插入的行**（线性流末尾追加），不是回头改历史行（§1 铁律 1）；
//! 详情正文缩进 4 空格（space.4）。

use super::event::DetailKind;
use super::terminal::{ascii, freeze_line, Profile, SYM_FOLD};
use std::io::Write;
use yunyou_session::{FOLD_THRESHOLD, PREVIEW_LINES};

// 折叠阈值与预览行数单源化（task-22 尾件，lead 裁方案甲）：原本文件私有
// 常量上移 yunyou-session（阈值历史=任务30 二期②立/三期③ 8→10 两处统一，
// 业主裁定「合适的地方折叠」；两处各持数字必漂移），值锁测试在 session lib.rs。

/// 折叠预览行数单源（任务30 四期 D-A，业主暴怒实锤）：折叠**必带预览**——
/// 「用户靠前面十几行内容来看工具在干嘛，一点不给怎么知道动了什么」。
/// 非零退出时 stderr 尾部展示行数（错误不能藏）。
const BASH_TAIL_LINES: usize = 5;

/// 按 DetailKind 渲染详情块（紧跟完成卡之后的若干新行）。
pub fn render<W: Write>(out: &mut W, profile: Profile, kind: &DetailKind) -> std::io::Result<()> {
    // task-65 P0-1：入口单点净化——详情块全系模型可见文本（文件内容/命令
    // 输出/diff），先净化再分发（\n 保留结构语义；\r 剥除保「落定永不回改」
    // =P1-1 同源收口；外来 CSI/OSC 剥除）。
    let kind = &match kind {
        DetailKind::Bash { exit_code, output } => {
            DetailKind::Bash { exit_code: *exit_code, output: super::sanitize::sanitize_text(output) }
        }
        DetailKind::Read { path, lines, from } => DetailKind::Read {
            path: super::sanitize::sanitize_text(path),
            lines: lines.iter().map(|l| super::sanitize::sanitize_text(l)).collect(),
            from: *from,
        },
        DetailKind::Diff { added, removed, hunks } => DetailKind::Diff {
            added: *added,
            removed: *removed,
            hunks: hunks
                .iter()
                .map(|h| crate::render::event::DiffHunk {
                    path: h.path.clone(),
                    context_lines: h.context_lines,
                    removed: h.removed.iter().map(|l| super::sanitize::sanitize_text(l)).collect(),
                    added: h.added.iter().map(|l| super::sanitize::sanitize_text(l)).collect(),
                })
                .collect(),
        },
        DetailKind::Plain { lines } => DetailKind::Plain {
            lines: lines.iter().map(|l| super::sanitize::sanitize_text(l)).collect(),
        },
        // task-66② 域（云点归属）：四变体全覆盖后 other 臂不可达——unreachable
        // pattern 清除（域内 -D warnings 口径清偿，lead 树绿令 2026-11-17）。
    };
    match kind {
        DetailKind::Bash { exit_code, output } => render_bash(out, profile, *exit_code, output),
        DetailKind::Read { lines, from, .. } => render_read(out, profile, lines, *from),
        DetailKind::Diff { added, removed, hunks } => render_diff(out, profile, *added, *removed, hunks),
        DetailKind::Plain { lines } => render_plain(out, profile, lines),
    }
}

/// bash：退出码 0 → 折叠摘要行；非 0 → stderr 尾部 ≤5 行直接展示（绯红）。
/// 行行带左缘束线（task-46③）。
fn render_bash<W: Write>(
    out: &mut W,
    profile: Profile,
    exit_code: Option<i32>,
    output: &str,
) -> std::io::Result<()> {
    let lines: Vec<&str> = output.lines().collect();
    // task-66 P1-3：None=未知退出码——不假设失败，无 stderr 尾行依据（tail
    // 是错误展示面，未知≠失败）；退出码语义由摘要层承载。
    if matches!(exit_code, Some(code) if code != 0) {
        let tail: Vec<&str> = lines.iter().rev().take(BASH_TAIL_LINES).copied().collect();
        for line in tail.iter().rev() {
            let rendered = rail_line(
                profile,
                &format!(
                    "{}{}{}",
                    profile.color(super::terminal::SGR_FEEDBACK_DANGER_256),
                    line,
                    profile.reset()
                ),
            );
            freeze_line(out, &rendered)?;
        }
        return Ok(());
    }
    // 成功：≤10 行直接全展（小输出折叠是让用户多一步才能看到结果，反效率）；
    // >10 行**永远先渲染前 10 行预览**再折叠（任务30 四期 D-A——零预览
    // 折叠让用户无从判断工具动了什么，业主实锤）；全文展开是后续交互项
    if lines.len() <= FOLD_THRESHOLD {
        for line in &lines {
            freeze_line(out, &rail_line(profile, line))?;
        }
        return Ok(());
    }
    for line in lines.iter().take(PREVIEW_LINES) {
        freeze_line(out, &rail_line(profile, line))?;
    }
    let note = fold_count(lines.len() - PREVIEW_LINES);
    freeze_line(out, &fold_note(profile, &note))
}

/// 文件读取：≤10 行全量带行号；>10 行首 10 行预览 + `… +N 行`（任务30
/// 四期 D-A 与 bash 拉齐）；>200 行只给范围不预览。
fn render_read<W: Write>(out: &mut W, profile: Profile, lines: &[String], from: usize) -> std::io::Result<()> {
    if lines.len() > 200 {
        let note = format!("共 {} 行（超长，仅给范围不预览）", lines.len());
        return freeze_line(out, &fold_note(profile, &note));
    }
    let show = if lines.len() > FOLD_THRESHOLD { &lines[..PREVIEW_LINES] } else { lines };
    for (offset, line) in show.iter().enumerate() {
        let rendered = code_line(profile, from + offset, line);
        freeze_line(out, &rendered)?;
    }
    if lines.len() > FOLD_THRESHOLD {
        let note = fold_count(lines.len() - show.len());
        freeze_line(out, &fold_note(profile, &note))?;
    }
    Ok(())
}

/// diff：+/− 着色、hunk 间 `⋮ 上下文 N 行` 折叠；hunk >3 仅首个全展。
fn render_diff<W: Write>(
    out: &mut W,
    profile: Profile,
    added: usize,
    removed: usize,
    hunks: &[super::event::DiffHunk],
) -> std::io::Result<()> {
    if hunks.is_empty() {
        let note = format!("+{added} −{removed}（无逐行上下文）");
        return freeze_line(out, &fold_note(profile, &note));
    }
    let show_all = hunks.len() <= 3;
    for (idx, hunk) in hunks.iter().enumerate() {
        if idx > 0 && !show_all {
            let note = format!("+{added} −{removed} · 共 {} 处", hunks.len());
            return freeze_line(out, &fold_note(profile, &note));
        }
        if let Some(path) = &hunk.path {
            // 批 2-T2a：多文件补丁的文件归属行（dim 束线；hunk 级——同文件
            // 多 hunk 重复标注可接受，换可读性信息零损）
            let line = format!(
                "    {}{}{} {}▎ {}{}",
                profile.dim(),
                rail(profile),
                profile.reset(),
                profile.dim(),
                path,
                profile.reset()
            );
            freeze_line(out, &line)?;
        }
        if hunk.context_lines > 0 {
            // task-45③（规范 §4.11「hunk 间以 `⋮ 上下文 N 行` 折叠」定谳=代码
            // 偏离）：diff hunk 上下文行用 ⋮（区别于 §4.9/4.10 通用折叠的 …；
            // 演示壳 mock 亦用 S_CONT=⋮）。束线与 ± 行连续（task-45 裁量）。
            let note = format!(
                "    {}{}{} {}⋮ 上下文 {} 行{}",
                profile.dim(),
                rail(profile),
                profile.reset(),
                profile.dim(),
                hunk.context_lines,
                profile.reset()
            );
            freeze_line(out, &note)?;
        }
        for line in &hunk.removed {
            let rendered = diff_line(profile, super::terminal::SYM_DIFF_MINUS, line, super::terminal::SGR_FEEDBACK_DANGER_256);
            freeze_line(out, &rendered)?;
        }
        for line in &hunk.added {
            let rendered = diff_line(profile, "+", line, super::terminal::SGR_FEEDBACK_SUCCESS_256);
            freeze_line(out, &rendered)?;
        }
    }
    Ok(())
}

/// 其余工具详情：逐行纯文本（阈值与 bash/read 同源；上限 FOLD_THRESHOLD+1
/// 为 8KB 降级文本的折叠计数行留位——D-A 单源原则：预览必须可见，折叠
/// 计数也必须可见，截掉计数行=用户不知还剩多少）。
fn render_plain<W: Write>(out: &mut W, profile: Profile, lines: &[String]) -> std::io::Result<()> {
    let show = &lines[..lines.len().min(FOLD_THRESHOLD + 1)];
    for line in show {
        let rendered = rail_line(profile, line);
        freeze_line(out, &rendered)?;
    }
    if lines.len() > show.len() {
        let note = fold_count(lines.len() - show.len());
        freeze_line(out, &fold_note(profile, &note))?;
    }
    Ok(())
}

/// 折叠/上下文提示行（左缘束线 + 秋香黄 … + 烟灰说明，缩进 4 空格；task-46③）。
fn fold_note(profile: Profile, note: &str) -> String {
    format!(
        "    {}{}{} {}{}{} {}{}",
        profile.dim(),
        rail(profile),
        profile.reset(),
        profile.color(super::terminal::SGR_FEEDBACK_WARNING_256),
        profile.symbol(SYM_FOLD, ascii::FOLD),
        profile.reset(),
        profile.dim(),
        note
    )
}

/// 详情左缘束线（task-46③）：4 空格缩进位落一根烟灰竖线，详情行从「散落
/// 文本」成「挂在完成卡上的抽屉」。Full/NoColor `▎`（1/4 块——与思考 `▏`、
/// 引用 `│` 三符三义），Plain 档 ASCII `|` 降级（降级矩阵不回退）。
fn rail(profile: Profile) -> &'static str {
    match profile {
        Profile::Plain => "|",
        _ => "▎",
    }
}

/// 束线行：`    {dim}▎{reset} {body}`——语义色由 body 自带（调用方拼装）。
fn rail_line(profile: Profile, body: &str) -> String {
    format!("    {}{}{} {body}", profile.dim(), rail(profile), profile.reset())
}

/// 折叠计数句式单源（task-46④：《CLI终端显示设计规范.md》§4.10 定稿）——
/// 全仓唯一折叠句式「折叠 +{n} 行」（省略号由 fold_note 的 SYM_FOLD 字形
/// 承担，此处不带，防双 …），bash/read/plain 三调用点归一，
/// 杜绝「已折叠 N 行／折叠 +N 行／丢 …」三种口吻再漂移（文案也是 UI）。
fn fold_count(hidden: usize) -> String {
    format!("折叠 +{hidden} 行")
}

/// 读文件预览行（行号烟灰右对齐 4 位；左缘束线 task-46③）。
/// task-65 P2 行宽预算：content 按 TERMINAL_WIDTH−9 束线/行号余量截断
///（宁截断不折行——折行破坏 rail 行序列对齐；超长行完整内容在会话 JSONL）。
fn code_line(profile: Profile, no: usize, content: &str) -> String {
    let budget = crate::panel_text::TERMINAL_WIDTH.saturating_sub(9);
    let content = &crate::render::tool::clip_width(content, budget);
    rail_line(
        profile,
        &format!(
            "{}{:>4}{} │ {content}",
            profile.color(super::terminal::SGR_TEXT_SECONDARY_256),
            no,
            profile.reset()
        ),
    )
}

/// diff 行（+/− 前缀着色；左缘束线 task-46③）。
fn diff_line(profile: Profile, sign: &str, content: &str, sgr_const: &'static str) -> String {
    rail_line(
        profile,
        &format!(
            "{}{sign}{} {content}",
            profile.color(sgr_const),
            profile.reset()
        ),
    )
}
