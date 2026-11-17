//! 行内 markdown 渲染（规范 §4.5 行内层；任务 30-①）。
//!
//! # 语法映射（一期）
//! - `` `x` `` → 淡云青着色，**反引号去除**：反引号是标记语法而非内容，
//!   着色已独立表达「这是代码」，保留定界符属双份表达且打断视觉；
//! - `**x**` → SGR 粗体，星号去除；
//! - `*x*` / `_x_` → SGR 斜体（SGR 3，现代终端普遍支持）。
//!   **取舍注明**：Plain 档（YUNYOU_PLAIN/TERM=dumb）退化为无样式而非 dim——
//!   dim 已承担「烟灰次要文本」语义（思考块/摘要/树干），斜体退化成烟灰会混淆层级；
//! - 配对扫描顺序：**先代码段 → 粗体（`**`）→ 斜体（单 `*`/`_`）**——
//!   代码段内容不再参与标记扫描（段内 `**` 不误染）；
//! - 未闭合一律原样输出（fail-safe：定界符不成对即普通文本；空内容同样原样）；
//! - **内嵌行内代码组合**（任务30 四期 2.2）：粗体/斜体内含 `` `code` ``
//!   按段切色渲染（SGR 无栈：code 段前复位、后重开基础样式）；code 未
//!   闭合（奇数反引号）整段按普通样式原样（fail-safe 与外层一致）；
//!   其余嵌套（粗体内斜体等）仍平铺——pulldown-cmark 提案候业主拍板
//!   （docs/架构/参照/行内渲染依赖提案-pulldown-cmark.md）。
//!
//! 架构红利：满行才落屏（D-UI-1）＝行内渲染只在落定行跑，无跨 chunk 问题。

use super::terminal::Profile;

/// 行内层渲染入口：一遍扫描（由 [`super::stream::markdown_line`] 调用）。
pub fn render_inline(profile: Profile, line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 32);
    let mut rest = line;
    while let Some(pos) = rest.find(['`', '*', '_']) {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        // 定界符均为单字节 ASCII，切片按字节前进安全（中文内容不受影响）
        match rest.as_bytes()[0] {
            b'`' => match rest[1..].find('`') {
                Some(end) if end > 0 => {
                    out.push_str(&code(profile, &rest[1..1 + end]));
                    rest = &rest[end + 2..];
                }
                _ => {
                    out.push('`');
                    rest = &rest[1..];
                }
            },
            b'*' if rest.starts_with("**") => match rest[2..].find("**") {
                Some(end) if end > 0 => {
                    out.push_str(&bold(profile, &rest[2..2 + end]));
                    rest = &rest[end + 4..];
                }
                _ => {
                    out.push_str("**");
                    rest = &rest[2..];
                }
            },
            b'*' | b'_' => {
                // 同种定界符才配对（任务30 四期⑦）：**先搜同符**——旧实现先找
                // 任意定界符再验同符性，`*a_b_c*` 首遇 `_` 即判不配对而整段
                // 原样输出（外部位嵌套内容误伤）；改为直接搜与开头同符的
                // 闭合点，异符定界符（`_`）自然落入内容原样保留
                let open = rest.as_bytes()[0] as char;
                match rest[1..].find(open) {
                    Some(end) if end > 0 => {
                        out.push_str(&italic(profile, &rest[1..1 + end]));
                        rest = &rest[end + 2..];
                    }
                    _ => {
                        out.push(rest.as_bytes()[0] as char);
                        rest = &rest[1..];
                    }
                }
            }
            _ => unreachable!("find 只产出 ` * _"),
        }
    }
    out.push_str(rest);
    out
}

/// 行内代码：淡云青着色，反引号去除（理由见模块注释：着色即表达，定界符不重复）。
fn code(profile: Profile, inner: &str) -> String {
    format!(
        "{}{inner}{}",
        profile.color(super::terminal::SGR_BRAND_SECONDARY_256),
        profile.reset()
    )
}

/// 粗体：SGR 1（Plain 档 bold() 已内置无样式降级）；内嵌 `` `code` ``
/// 组合走 [`styled_mix`]（任务30 四期 2.2）。
fn bold(profile: Profile, inner: &str) -> String {
    styled_mix(profile, profile.bold(), inner)
}

/// 样式与行内代码混排（任务30 四期 2.2）：SGR 无栈——终端样式无嵌套栈，
/// code 段切换色号前必须复位、复位后重开基础样式，否则 code 色污染后续
/// 文本。code 段数不成对（未闭合）时退化为整段基础样式（与外层未闭合
/// fail-safe 同语义）。code 段内不递归（代码内容零改写，规范 §4.5）。
fn styled_mix(profile: Profile, base_sgr: &str, inner: &str) -> String {
    let reset = profile.reset();
    if !inner.contains('`') {
        return format!("{base_sgr}{inner}{reset}");
    }
    if inner.split('`').count().is_multiple_of(2) {
        // 奇数个反引号（split 产物为偶数段）= code 未闭合，整体按普通样式
        return format!("{base_sgr}{inner}{reset}");
    }
    let code_sgr = profile.color(super::terminal::SGR_BRAND_SECONDARY_256);
    let mut out = String::new();
    let mut in_code = false;
    for seg in inner.split('`') {
        if seg.is_empty() {
            continue; // 相邻反引号/首尾定界：空段无内容可渲染
        }
        if in_code {
            out.push_str(&format!("{code_sgr}{seg}{reset}"));
        } else {
            out.push_str(&format!("{base_sgr}{seg}"));
        }
        in_code = !in_code;
    }
    // 收尾复位：末段为普通段时样式仍开着；code 段已自带 reset
    if !in_code {
        out.push_str(reset);
    }
    out
}

/// 斜体：SGR 3（Plain 档无样式降级，取舍见模块注释）；内嵌 `` `code` ``
/// 组合同粗体（任务30 四期 2.2）。
fn italic(profile: Profile, inner: &str) -> String {
    styled_mix(profile, profile.italic(), inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Profile = Profile::Full;

    #[test]
    fn 行内代码_去反引号_着色() {
        let rendered = render_inline(FULL, "用 `epoch` 表达");
        assert!(rendered.contains("\x1b[38;5;110mepoch\x1b[0m"), "{rendered:?}");
        assert!(!rendered.contains('`'), "反引号应去除：{rendered:?}");
    }

    #[test]
    fn 粗体_去星号_着色() {
        let rendered = render_inline(FULL, "**重要**事项");
        assert!(rendered.contains("\x1b[1m重要\x1b[0m"), "{rendered:?}");
        assert!(!rendered.contains("**"), "{rendered:?}");
    }

    #[test]
    fn 斜体_单星与下划线() {
        for line in ["*强调*词", "_强调_词"] {
            let rendered = render_inline(FULL, line);
            assert!(rendered.contains("\x1b[3m强调\x1b[0m"), "{line} → {rendered:?}");
        }
    }

    #[test]
    fn 代码段内星号不误染() {
        let rendered = render_inline(FULL, "`a**b` 与 **粗**");
        assert!(rendered.contains("a**b"), "代码段内容原样：{rendered:?}");
        assert!(rendered.contains("\x1b[1m粗\x1b[0m"), "段外粗体生效：{rendered:?}");
    }

    #[test]
    fn 未闭合原样() {
        for line in ["** 未闭合", "* 未闭合", "_ 未闭合", "` 未闭合", "**** 空", "*a_ 跨样式"] {
            let rendered = render_inline(FULL, line);
            assert!(!rendered.contains('\x1b'), "{line} → 未闭合不应着色：{rendered:?}");
        }
    }

    #[test]
    fn 纯文本档全退化() {
        let rendered = render_inline(Profile::Plain, "**粗**与`码`");
        assert_eq!(rendered, "粗与码");
    }
}
