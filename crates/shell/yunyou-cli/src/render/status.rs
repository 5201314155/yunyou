//! 状态落款与恢复回放头部（规范 §4.12 本地聚合落款 / §4.13 会话恢复）。

use super::event::TurnEvent;
use super::terminal::{ascii, freeze_line, Profile, SYM_BRAND, SYM_FAILURE, SYM_FENCE_DOT};
use super::tool::visible_width;
use std::io::Write;
use std::time::Duration;

/// 状态落款行（回合完结后输出一次：耗时 · 上下文水位/累计 token · 模型；烟灰
/// dim 随流滚动）。
///
/// 语义分流（task-26，业主实证 2026-11-17：累计上行 1,725,916 被误读成当前
/// 上下文——「本回合」标签配累计数字系双重撒谎）：**水位段**（context_tokens=
/// 最后一步 usage.prompt——LLM 计费口径=每回合全量重发历史，端点报的 prompt
/// 即「当前上下文」的精确测量）为主位；**累计段**（↑↓ 整场流量）挪次位并
/// 显式「累计」标签，对齐 Claude Code 显示口径（状态行=当前水位，累计数字
/// 在 /cost 类命令——v1 以次位括注承载，独立 /cost 命令候后批）。usage 未报
/// （None）→ 无水位可示，只出「累计」明示段（诚实：没数据不假装有水位）。
/// 「X% until auto-compact」百分比形态候 task-27 auto-compact 落地（有阈值
/// 才有分母）。
///
/// 紧凑版式（任务30 二期④）沿用；窄屏（<60 列）去装饰（任务30 四期 1.3）
/// 且水位段省略累计括注（48 列预算下模型名优先）。
pub fn turn_done<W: Write>(
    out: &mut W,
    profile: Profile,
    done: &TurnEvent,
    width: usize,
) -> std::io::Result<()> {
    let TurnEvent::TurnDone {
        elapsed,
        prompt_tokens,
        completion_tokens,
        context_tokens,
        context_window,
        model,
    } = done
    else {
        return Ok(());
    };
    let dim_sgr = profile.color(super::terminal::SGR_TEXT_SECONDARY_256);
    let brand = profile.symbol(SYM_BRAND, ascii::BRAND);
    let elapsed_text = format_elapsed(elapsed);
    let up_text = format_count(profile, *prompt_tokens);
    let down_text = format_count(profile, *completion_tokens);
    // 语义分流主逻辑（见函数 doc）：宽屏=水位主位+累计明示次位（None 只出
    // 累计明示段）；窄屏（<60）**一律省 usage 段**——48 列预算下「累计 」
    // 标签或水位段都会挤掉模型名全名（render_width「落款行_80与48_恒单视觉行」
    // 既有定谳：模型名优先），不显示≠撒谎显示（诚实性不受损，任务30 四期
    // 1.3「装饰服务信息不能反过来吃信息」方向的延伸）。
    // task-27 联动（窗口在场→百分比+距压缩；窄屏降级序=先距压缩→再百分比→
    // token 数最后丢——极窄 <40 只留「上下文 X tok」；<60 整段省略沿用既有）。
    let pct = match (context_tokens, context_window) {
        (Some(ctx), Some(window)) if *window > 0 => {
            Some(ctx.saturating_mul(100) / window)
        }
        _ => None,
    };
    let trigger = context_window
        .map(|window| {
            window.saturating_sub(20_000).saturating_sub(32_000)
        });
    let until_compact = match (context_tokens, trigger) {
        (Some(ctx), Some(t)) => Some(t.saturating_sub(*ctx)),
        _ => None,
    };
    let usage_text = if width < 60 {
        // 极窄（<40）只留 token 数；40..60 整段省略（既有阈值——模型名优先）。
        if width < 40 {
            match context_tokens {
                Some(ctx) => format!("上下文 {} tok", format_count(profile, Some(*ctx))),
                None => String::new(),
            }
        } else {
            String::new()
        }
    } else {
        match (context_tokens, pct, until_compact) {
            (Some(ctx), Some(pct), Some(until)) => format!(
                "上下文 {} tok（{}% · 距压缩 {}）（累计 ↑{} ↓{}）",
                format_count(profile, Some(*ctx)),
                pct,
                format_count(profile, Some(until)),
                up_text,
                down_text
            ),
            (Some(ctx), _, _) => format!(
                "上下文 {} tok（累计 ↑{} ↓{}）",
                format_count(profile, Some(*ctx)),
                up_text,
                down_text
            ),
            (None, _, _) => format!("累计 ↑{up_text} ↓{down_text}"),
        }
    };
    // 窄屏（<60 列）去装饰（任务30 四期 1.3）：「──」对称横线吃 6 列、「本回合」
    // 又吃 10 列——模型名才是用户最需看全的信息（多模型中转要分辨当前谁在
    // 干活），装饰服务信息不能反过来吃信息。去装饰后 48 列下全名完整。
    let (lead_in, tail_text, metric) = if width >= 60 {
        ("  ── ", " ──", format!(" 本回合 {elapsed_text}"))
    } else {
        ("  ", "", format!(" {elapsed_text}"))
    };
    // 单源拼装（任务30 四期票②）：中间段只 format 一遍，纯文本版复用于
    // 预算计算（旧实现同参数两遍 format=改一忘二隐患）。窄屏无 usage 段时
    // 分隔符同步省略（不落孤悬「 · 」）。
    let middle = if usage_text.is_empty() {
        format!("{brand}{metric} · ")
    } else {
        format!("{brand}{metric} · {usage_text} · ")
    };
    let plain_head = format!("{lead_in}{middle}");
    let budget = width
        .saturating_sub(visible_width(&plain_head) + visible_width(tail_text))
        .max(3);
    let name = truncate_width(model, budget);
    let line = format!(
        // task-45②（规范 §4.12 定谳）：dim 从行首起——落款整行（含 ── lead_in）
        // 纳入 dim 区间，对齐演示样例「落款整行 dim 含 ──」的定稿形态。
        "{dim_sgr}{lead_in}{middle}{name}{tail_text}{}",
        profile.reset()
    );
    freeze_line(out, &line)
}

/// 模型名按显示宽度截断：超预算处截断并以 … 收尾（… 占 1 列，故正文预算减 1）；
/// 未超预算原样返回。窄屏落款恒单视觉行。
fn truncate_width(text: &str, budget: usize) -> String {
    if visible_width(text) <= budget {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = super::tool::char_width(ch);
        if used + w > budget.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// 会话恢复回放头部（烟灰：☁ 已恢复会话 <id> · N 条消息）。
pub fn resume_header<W: Write>(out: &mut W, profile: Profile, session_id: &str, messages: usize) -> std::io::Result<()> {
    let dim_sgr = profile.color(super::terminal::SGR_TEXT_SECONDARY_256);
    let brand = profile.symbol(SYM_BRAND, ascii::BRAND);
    freeze_line(
        out,
        &format!(
            "{dim_sgr}{brand} 已恢复会话 {session_id} {} {messages} 条消息{}",
            profile.symbol(SYM_FENCE_DOT, ascii::FENCE_DOT), // task-35② 追加小刀：间隔符走 D3 降级映射（Plain 出 .）
            profile.reset()
        ),
    )
}

/// 错误行（绯红，错误不能藏）。
pub fn error<W: Write>(out: &mut W, profile: Profile, message: &str) -> std::io::Result<()> {
    // 状态符走 profile.symbol 降级体系（task-44②）：硬编码 ✗ 在 Plain 档
    // （TERM=dumb）漏非 ASCII——同文件 resume_header/turn_done 的 ☁ 是正确
    // 示范，同一文件两种纪律即漂移；SYM_FAILURE/ascii::FAILURE 由 palette
    // 单源生成（build.rs semantic.symbol+ASCII 降级表 ✗→x）。
    freeze_line(
        out,
        &format!(
            "{}{}{} {}",
            profile.color(super::terminal::SGR_FEEDBACK_DANGER_256),
            profile.symbol(SYM_FAILURE, ascii::FAILURE),
            profile.reset(),
            message
        ),
    )
}

/// 耗时格式化：≥1s 显示秒（1 位小数），否则毫秒。
fn format_elapsed(elapsed: &Duration) -> String {
    if elapsed.as_secs() >= 1 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else {
        format!("{}ms", elapsed.as_millis())
    }
}

/// token 计数格式化（千分位；缺省：Full/NoColor 出 em dash「—」，Plain 按 §3
/// 整体退化出 ASCII「--」——task-35②：Plain 契约是「整体退化为 ASCII」，
/// em dash（U+2014）非 ANSI 亦非 ASCII，残留即违约（挑刺报告 P2-3 点位一）。
/// 不入 tokens symbol 表：这是文本标记而非界面语义符，profile 分流即够）。
fn format_count(profile: Profile, value: Option<u64>) -> String {
    match value {
        Some(count) => {
            let digits = count.to_string();
            let mut grouped = String::new();
            for (idx, ch) in digits.chars().enumerate() {
                if idx > 0 && (digits.len() - idx) % 3 == 0 {
                    grouped.push(',');
                }
                grouped.push(ch);
            }
            grouped
        }
        None => match profile {
            Profile::Plain => "--".to_string(),
            _ => "—".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task-44② 锁：error 状态符走 profile.symbol 降级体系——Plain 档
    /// （TERM=dumb）出 ASCII 降级符而非硬编码 ✗（同文件 ☁ 是正确示范）。
    #[test]
    fn error_状态符走降级体系_plain出ascii() {
        let mut buf = Vec::new();
        error(&mut buf, Profile::Plain, "测试").unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains('x'), "Plain 档应为 ASCII 降级符 x：{text}");
        assert!(!text.contains('\u{2717}'), "Plain 档不得出现非 ASCII ✗：{text}");
        let mut buf = Vec::new();
        error(&mut buf, Profile::Full, "测试").unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains('\u{2717}'), "Full 档应保留 ✗：{text}");
    }
}
