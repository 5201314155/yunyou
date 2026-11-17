//! 面板文本度量与截断（任务29 C3；自 panel.rs 拆出——面板状态机与纯文本
//! 工具两职责分行数硬顶，拆分不改逻辑）。
//!
//! 显示宽度口径：CJK 统一表意/扩展/兼容/全角区按 2 列，其余按 1 列——
//! 覆盖中文面板文案与命令别名的实际使用面；emoji 精确宽度一期不做
//!（候选行来自注册表，出现 emoji 时按 1 列估宽，仅影响截断点微偏）。

use yunyou_commands::CommandDescriptor;

/// 定宽基准（与 D-UI-1 渲染测试同口径）：候选行截断到「终端宽-2」。
/// task-17③ 起改为函数单源：三层探测（ioctl→COLUMNS→80 兜底，复用
/// render::terminal_cols）——修复前写死 80，40 列终端候选行 43 列未截
/// （硬件折行破坏帧序，D-UI-1 同机理回归实测在案）。兜底 80 与渲染域
/// 「非交互/缺失一律 80 不猜」口径一致。
/// 兜底宽（探测失效时的 panel_width 退路；测试与文档口径锚点）。
pub(crate) const TERMINAL_WIDTH: usize = 80;

/// 显示宽度（简易口径）：CJK 统一表意/扩展/兼容/全角区按 2 列，其余按 1 列。
pub fn display_width(text: &str) -> usize {
    // 单源化（task-33③）：手撸 Unicode 区段表退役，宽度判定收敛到
    // unicode-width 标准件（render/tool::char_width 同源）——区段手撸表
    // 漏 emoji/组合序列等形态，双判定并存即漂移面。
    text.chars().map(crate::render::tool::char_width).sum()
}

/// 候选行第一列文案：`/name（别名1/别名2）`；无别名时 `/name`。
pub(crate) fn first_column(cmd: &CommandDescriptor) -> String {
    if cmd.aliases.is_empty() {
        format!("/{}", cmd.name)
    } else {
        format!("/{}（{}）", cmd.name, cmd.aliases.join("/"))
    }
}

/// 按显示宽截断，超宽部分以 `…` 收尾（省略号本身占 1 列）。
pub(crate) fn truncate_width(text: &str, max: usize) -> String {
    if display_width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for c in text.chars() {
        // 直调 char_width（task-36④，挑刺 P3-4）：每字符临时 String 堆分配
        // 退役——char_width(ch) 同语义零分配。
        let w = crate::render::tool::char_width(c);
        if width + w + 1 > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        width += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_commands::CommandKind;

    /// 造目录条目（与 panel.rs 测试同款）。
    fn cmd(name: &str, aliases: &[&str], description: &str) -> CommandDescriptor {
        CommandDescriptor {
            name: name.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            description: description.to_string(),
            kind: CommandKind::Action,
            accepts_args: false,
        }
    }

    /// 两列截断：超宽行截到 78 列内并带省略号（自 panel.rs 迁入）。
    #[test]
    fn 候选行截断到定宽() {
        let long = "很长的描述".repeat(30);
        let text = truncate_width(&format!("/help{}{}", " ".repeat(4), long), TERMINAL_WIDTH - 2);
        assert!(display_width(&text) <= TERMINAL_WIDTH - 2, "截断后仍超宽: {}", display_width(&text));
        assert!(text.ends_with('…'), "截断行应以省略号收尾: {text}");
        let short = truncate_width("/help 看命令", TERMINAL_WIDTH - 2);
        assert_eq!(short, "/help 看命令", "不超宽不截断");
        assert_eq!(display_width("中文abc"), 7, "CJK 计 2 列、ASCII 计 1 列");
    }

    /// 第一列合成：有别名带括号、无别名裸名。
    #[test]
    fn 第一列合成() {
        assert_eq!(first_column(&cmd("help", &["帮助"], "")), "/help（帮助）");
        assert_eq!(first_column(&cmd("new", &[], "")), "/new");
        assert_eq!(
            first_column(&cmd("quit", &["退出", "q"], "")),
            "/quit（退出/q）",
            "多别名斜杠连接"
        );
    }
}
