//! 渲染入口单点净化层（task-65 P0-1，挑刺报告4 P0-1/P3-1）：
//!
//! 模型可见内容（工具输出/文件内容/命令输出）可能含 ANSI 转义序列与 C0 控制
//! 字符——直通上屏=终端状态被外部数据改写（光标移动/清屏/改色），渲染层自身
//! 的 SGR 与活动行契约被破坏。三类入口（工具行体/块行/详情块）统一在渲染前
//! 调 [`sanitize_text`] 净化：
//!
//! - **保留本层自产 SGR**：净化发生在「模型文本插值前」——本层此后自行拼装的
//!   SGR 不经过本函数，天然保留；
//! - **剥外来 CSI/OSC**：`\x1b[…` 与 `\x1b]…`（模型可注入任意终端控制）；
//! - **C0 控制字符**：`\n` 保留（结构语义，上游行拆分负责）、`\t` 展开四空格
//!   （等宽对齐）、其余 C0（含 `\r`）剥除——`\r` 在块行内=进度条重绘残留，
//!   保留会破坏「落定永不回改」（P1-1 同源）；
//! - **fence 信息串并入**（P3-1）：代码围栏语言标注同样经模型文本路径，
//!   净化覆盖。

/// 净化一段模型可见文本：剥 CSI/OSC 转义与 C0 控制字符（`\n`/`\t` 除外），
/// `\t` 展开四空格。
pub fn sanitize_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                // CSI：ESC [ 参数字母；OSC：ESC ] … BEL/ST——整段剥除
                match chars.peek() {
                    Some('[') => {
                        chars.next();
                        for c in chars.by_ref() {
                            // 终止符：0x40-0x7E 区间字母（CSI final byte）
                            if ('\x40'..='\x7e').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        chars.next();
                        while let Some(c) = chars.next() {
                            // 终止符：BEL 或 ST（ESC \）
                            if c == '\x07' {
                                break;
                            }
                            if c == '\x1b' && chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    _ => {} // 孤立 ESC：剥字节本身
                }
            }
            '\n' => out.push('\n'), // 结构语义保留（上游行拆分负责）
            '\t' => out.push_str("    "), // 等宽展开
            c if c.is_control() => {}     // 其余 C0（含 \r）剥除
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::sanitize_text;

    /// P0-1：CSI 剥除（光标移动/清屏/改色三类注入全拦）。
    #[test]
    fn csi_剥除() {
        assert_eq!(sanitize_text("\x1b[2A上移"), "上移");
        assert_eq!(sanitize_text("\x1b[2J清屏"), "清屏");
        assert_eq!(sanitize_text("\x1b[38;5;196m红\x1b[0m"), "红");
    }

    /// P0-1：OSC 剥除（窗口标题/剪贴板注入）。
    #[test]
    fn osc_剥除() {
        assert_eq!(sanitize_text("\x1b]0;标题\x07正文"), "正文");
        assert_eq!(sanitize_text("\x1b]52;c;base64\x1b\\正文"), "正文");
    }

    /// P0-1/P1-1：\r 剥除（进度条重绘残留——落定永不回改）。
    #[test]
    fn cr_剥除() {
        assert_eq!(sanitize_text("50%\r100%"), "50%100%", "剥除语义：\r 移除前后段拼接（进度条重绘在行拆分层折叠）");
        assert_eq!(sanitize_text("a\rb"), "ab");
    }

    /// 结构语义保留：\n 不剥（上游行拆分负责）、\t 展开四空格。
    #[test]
    fn 换行制表语义() {
        assert_eq!(sanitize_text("a\nb"), "a\nb");
        assert_eq!(sanitize_text("a\tb"), "a    b");
    }

    /// P3-1：fence 信息串（模型文本路径）同样净化。
    #[test]
    fn fence信息串净化() {
        assert_eq!(sanitize_text("```rust\x1b[31m"), "```rust");
    }

    /// 字节级断言（挑刺报告4 P0-1 验收）：visible_width 对净化后文本不再
    /// 把转义序列计入宽度——净化后 "…\x1b[2A…" 的宽度=可见字符宽。
    #[test]
    fn 净化后宽度不含转义() {
        let raw = "\x1b[2A进度";
        let clean = sanitize_text(raw);
        assert_eq!(clean, "进度");
        assert_eq!(
            crate::render::tool::visible_width(&clean),
            4,
            "净化后宽度=可见字符宽（CJK×2）"
        );
    }
}
