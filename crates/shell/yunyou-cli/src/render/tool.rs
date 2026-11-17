//! 工具调用视觉块（规范 §4.6 进行中 / §4.7 完成卡 / 摘要规则）。
//!
//! - 行首两空格（space.2）；spinner 亮云青、✓ 苔绿、✗ 绯红；
//!   工具名粗体；参数摘要烟灰（截断到 48 列，全角按 2 列计）；
//! - 快路径：结果先到则无 spinner 直接落定（不假装忙碌）；
//! - spinner 循环由 REPL 侧驱动（每帧调 [`tool_running_frame`]），
//!   本文件只渲染单帧与落定行——纯函数可测。

use super::event::TurnEvent;
use super::terminal::{ascii, rewrite_active_line, freeze_line, Profile, SYM_FAILURE, SYM_SUCCESS};
use std::io::Write;

/// 摘要列宽预算（任务30 四期 1.2，建议书公式）：`width − 前缀实际宽 − 8`
/// 随终端列宽单源走——120 列终端不再固定截 48 浪费半屏（B1 ioctl 落地后
/// 列宽可信）。前缀实测不估值：`"  {符} {名} · "`=缩进 2+状态符 1+空格 1+
/// 工具名（长短不一，bash 4 列 vs webfetch 8 列）+「 · 」3；8=行尾呼吸
/// 空间。SUMMARY_WIDTH=48 硬编码退役；窄终端预算随之收缩但总行恒 ≤ 列宽
/// （摘要截断优于折行，比 48 硬编码在窄终端折行更优）。
fn summary_budget(width: usize, name: &str, prefix_cells: usize) -> usize {
    // task-51⑤：树前缀计入预算——tool_done_with_prefix 会在行首先写
    // indent_prefix（depth×2 列+树枝符），不传入则 depth≥5 时超预算折行
    // 孤字复发（报告3 Bug-5，一期 A 线 depth 恒 0 不触发的潜伏账）。
    width.saturating_sub(prefix_cells + visible_width(name) + 7 + 8)
}

/// 工具名 → 类型分色 SGR（tokens semantic.color.tool.*，task-47①）。
///
/// 只上工具名（粗体承载区分），状态符仍归状态色管；未知工具（含 webfetch 等
/// 非内置名）归中性——中性=不与摘要争色，三主工具（bash/read/edit）滚屏一眼
/// 可辨。精确匹配内置名：工具名是注册表唯一键，无需前缀猜测。
fn tool_type_sgr(name: &str) -> &'static str {
    match name {
        "read" | "grep" | "glob" => super::terminal::SGR_TOOL_READ_256,
        "write" | "edit" => super::terminal::SGR_TOOL_EDIT_256,
        _ => super::terminal::SGR_TOOL_BASH_256,
    }
}

/// 提取事件里的工具调用字段（ToolCall 专用）。
pub struct ToolCallView<'a> {
    pub name: &'a str,
    pub summary: &'a str,
}

/// 工具进行中行的单帧渲染（规范 §4.6：spinner+工具名+参数摘要+运行耗时）。
///
/// `frame` 为帧下标（REPL 侧 80ms 定时递增，帧序列由 Profile 降级决定）；
/// `elapsed_secs` 为运行耗时秒级取整（任务47②：由帧驱动按帧数换算注入，
/// 渲染层零时钟——纯函数可测纪律不变）。
pub fn tool_running_frame<W: Write>(
    out: &mut W,
    profile: Profile,
    call: &ToolCallView<'_>,
    frame: usize,
    elapsed_secs: Option<u64>,
    width: usize,
    prefix_cells: usize,
) -> std::io::Result<()> {
    let frames = profile.spinner_frames();
    let spinner = frames[frame % frames.len()];
    let line = tool_line_body(
        profile,
        ToolState { sgr: profile.color(super::terminal::SGR_BRAND_BRIGHT_256), mark: spinner },
        call.name,
        call.summary,
        elapsed_secs,
        width,
        prefix_cells,
    );
    rewrite_active_line(out, &line)
}

/// 工具完成卡（规范 §4.7：✓/✗ + 摘要；失败必须带原因——summary 由调用方保证含退出码/错误首行）。
pub fn tool_done<W: Write>(
    out: &mut W,
    profile: Profile,
    done: &TurnEvent,
    width: usize,
    prefix_cells: usize,
) -> std::io::Result<()> {
    let TurnEvent::ToolResult { name, ok, summary, parent_tool_call_id, .. } = done else {
        return Ok(()); // 非工具结果事件不渲染（调用方过滤）
    };
    let (sgr, mark) = if *ok {
        (profile.color(super::terminal::SGR_FEEDBACK_SUCCESS_256), profile.symbol(SYM_SUCCESS, ascii::SUCCESS))
    } else {
        (profile.color(super::terminal::SGR_FEEDBACK_DANGER_256), profile.symbol(SYM_FAILURE, ascii::FAILURE))
    };
    let body = tool_line_body(profile, ToolState { sgr, mark }, name, summary, None, width, prefix_cells);
    // 树状缩进：有 parent 的子调用挂树（规范 §4.8，缩进 2 空格/层，此处一层由 tree.rs 承担）
    let _ = parent_tool_call_id;
    freeze_line(out, &body)
}

/// 工具取消卡（task-37① 三态之三）：回合被取消时在途工具落中性卡——
/// 不打 ✓（没成功）也不打 ✗（没失败，是不再等待）：烟灰「–」+「已取消」，
/// 不留「无结果」空白（取消前 ToolResult 永不到达，spinner 被 abort 的旧态
/// 是屏上悬空一片）。由观察者在回合收尾（finish）时调用。
pub fn tool_cancelled<W: Write>(out: &mut W, profile: Profile, name: &str) -> std::io::Result<()> {
    let line = tool_line_body(
        profile,
        ToolState { sgr: profile.color(super::terminal::SGR_TEXT_SECONDARY_256), mark: super::terminal::SYM_CANCEL_DASH },
        name,
        "已取消",
        None,
        usize::MAX, // 取消摘要为固定短语，无需列宽裁剪
        0,
    );
    freeze_line(out, &line)
}

/// 工具行主体（进行中与完成共用形态：`  {符} {名} · {摘要}[ · {耗时}]`）。
///
/// 工具名=粗体+类型分色（task-47①：tool_type_sgr，只上名字，状态符归状态色）；
/// `elapsed` 仅进行中帧携带（Some=行尾追加 `· {n}s`，摘要预算扣除其宽度——
/// 秒数无界增长时不挤爆行宽）；None=完成/取消卡（耗时数字由结果摘要承载，
/// 调用方保证，规范 §4.6 摘要规则不变）。
fn tool_line_body(
    profile: Profile,
    state: ToolState<'_>,
    name: &str,
    summary: &str,
    elapsed: Option<u64>,
    width: usize,
    prefix_cells: usize,
) -> String {
    // task-65 P0-1：入口单点净化——summary 系模型可见文本（工具输出摘要），
    // 外来 CSI/OSC/C0 直通会破坏本层 SGR 与活动行契约；本层自产 SGR 在
    // 净化后拼接，天然保留。
    let dim = profile.dim();
    let reset = profile.reset();
    let summary = &super::sanitize::sanitize_text(summary);
    let elapsed_part = elapsed.map(|secs| format!(" · {secs}s")).unwrap_or_default();
    let budget = summary_budget(width, name, prefix_cells).saturating_sub(visible_width(&elapsed_part));
    // task-65 P1-3 空摘要契约：summary 空=「· 」段不出（空摘要占位行=观感
    // 噪音；上游无摘要语义时渲染层不虚构）。
    let summary_seg = if summary.is_empty() {
        String::new()
    } else {
        // task-65 ⑧ D1 裁决（tokens.json 87117e1）：正文级摘要 245 不叠 dim
        //（dim 叠 245=对比度跌破线）；· 分隔符保留 dim=结构元素可叠。
        format!(
            " {dim}·{reset} {}{elapsed_part}{reset}",
            clip_width(summary, budget),
        )
    };
    format!(
        "  {sgr}{mark}{} {}{}{}{}{summary_seg}",
        profile.reset(),
        profile.bold(),
        profile.color(tool_type_sgr(name)),
        name,
        profile.reset(),
        sgr = state.sgr,
        mark = state.mark,
    )
}

/// 状态段二元组（task-51⑤ 收参：profile/state/name/summary/elapsed/width/
/// prefix_cells 七参压回 clippy 7 上限——prefix_cells 入参后 8 参触发
/// too_many_arguments；状态色与状态符同生共死，捆扎语义自然）。
struct ToolState<'a> {
    sgr: String,
    mark: &'a str,
}

/// 按显示宽度截断（全角按 2 列计；超宽以 … 收尾——tokens symbol.fold）。
pub fn clip_width(text: &str, max: usize) -> String {
    // 恰满不截（task-44①，与本文件 truncate_width 同构正解）：先整判可见宽度，
    // 未超预算原样返回——旧实现把 `…` 的 1 列余量恒扣进判定阈值
    // （max.saturating_sub(1)），恰好卡满预算的文本被无谓截断
    // （clip_width("abc",3)=="ab…"）；只有真超预算才按 max-1 切、留 1 列给 …。
    if visible_width(text) <= max {
        return text.to_string();
    }
    let mut width = 0usize;
    for (idx, ch) in text.char_indices() {
        let w = char_width(ch);
        if width + w > max.saturating_sub(1) {
            let mut cut = text[..idx].to_string();
            cut.push('…');
            return cut;
        }
        width += w;
    }
    text.to_string()
}

/// 字符显示宽度薄封装（任务30 四期 2.3）：宽度判定委托 `unicode-width`
/// 标准件（Unicode 权威表——emoji/组合序列/全角变体全覆盖），本函数只
/// 承担接口形态与兜底语义：控制字符等标准件返回 None 的形态按 1 列计
/// （与手撸版口径一致——渲染文本中控制字符罕见，宁多算 1 列不误判 0）。
pub fn char_width(ch: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    ch.width().unwrap_or(1)
}

/// 可见列宽：剥 SGR 转义序列后按显示宽度计——混合流式溢出判定（任务30
/// 三期④）与落款截断共用。只识别 `\x1b[ … m` 形态（本渲染层只产出 SGR；
/// 若出现其他转义按 1 列估，fail-safe 不崩）。列宽判定的意义：终端实际
/// 占位只由可见字符决定，SGR 转义零占位，混入计算会把「未满」误判为
/// 「溢出」提前落定、流式观感退化。
pub fn visible_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n == 'm' {
                    break; // SGR 终结符
                }
            }
            continue;
        }
        width += char_width(c);
    }
    width
}

#[cfg(test)]
mod width_tests {
    use super::*;

    /// unicode-width 薄封装行为锁（任务30 四期 2.3）：emoji 2 列（手撸表
    /// 覆盖不到的区域）、CJK 2 列、ASCII 1 列、控制字符兜底 1 列。
    #[test]
    fn char_width_标准件替换行为锁() {
        assert_eq!(char_width('中'), 2, "CJK 2 列");
        assert_eq!(char_width('a'), 1, "ASCII 1 列");
        assert_eq!(char_width('🔥'), 2, "emoji U+1F525 手撸表外应 2 列");
        assert_eq!(char_width('Ａ'), 2, "全角拉丁 2 列");
        assert_eq!(char_width('\u{7}'), 1, "控制字符兜底 1 列");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task-47① 锁：三主工具类型分色映射（tokens tool.* 单源生成的常量），
    /// 未知工具归中性。
    #[test]
    fn 工具类型分色映射() {
        assert_eq!(tool_type_sgr("read"), super::super::terminal::SGR_TOOL_READ_256);
        assert_eq!(tool_type_sgr("grep"), super::super::terminal::SGR_TOOL_READ_256);
        assert_eq!(tool_type_sgr("edit"), super::super::terminal::SGR_TOOL_EDIT_256);
        assert_eq!(tool_type_sgr("write"), super::super::terminal::SGR_TOOL_EDIT_256);
        assert_eq!(tool_type_sgr("bash"), super::super::terminal::SGR_TOOL_BASH_256);
        assert_eq!(tool_type_sgr("webfetch"), super::super::terminal::SGR_TOOL_BASH_256, "未知工具归中性");
    }

    /// task-44① 锁：恰满不截——旧实现把 `…` 的 1 列余量恒扣进阈值，
    /// clip_width("abc",3)=="ab…" 差一误截；正解=先整判再按 max-1 切。
    #[test]
    fn clip_width_恰满原样_超截留省略号() {
        assert_eq!(clip_width("abc", 3), "abc", "恰满不截（差一回归锁）");
        assert_eq!(clip_width("ab", 3), "ab");
        assert_eq!(clip_width("abcd", 3), "ab…", "真超预算按 max-1 切+…");
        assert_eq!(clip_width("你好", 4), "你好", "CJK 全角 2 列恰满不截");
        assert_eq!(clip_width("你好世界", 4), "你…", "CJK 超预算截断+…");
        assert_eq!(clip_width("", 3), "", "空串原样");
    }
}
