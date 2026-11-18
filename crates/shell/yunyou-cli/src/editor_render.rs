//! 编辑行渲染（编辑行整族升级 IO 壳的绘制面）：全行重绘+光标定位策略
//! （光标可停在行中后，「尾部差分追加」优化只对光标在行尾的追加键生效——
//! task-42 差分基准语义收窄保留）。

use super::panel_text::display_width;
use std::io::Write;

/// 去 ANSI 转义（提示符着色串的显示宽度计算前置；零依赖最小状态机——
/// 只处理 CSI `ESC [ … 终止字节` 形态，行内其他转义罕见不展开）。
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c2 in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c2) {
                    break; // final 字节：序列结束
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 显示列宽（strip 后逐字符 panel_text::display_width 求和）。
pub(crate) fn cols_of(s: &str) -> usize {
    strip_ansi(s).chars().map(|c| display_width(&c.to_string())).sum()
}

/// 全行重绘：`\r` 回行首 → `ESC[2K` 清行 → 提示符+左段+右段 → 光标回位。
/// （提示符可含 ANSI 色码；光标定位=右移 提示符列宽+左侧正文列宽。）
pub(crate) fn redraw(
    out: &mut impl Write,
    prompt: &str,
    chars: &[char],
    cursor: usize,
) -> std::io::Result<()> {
    let left: String = chars[..cursor].iter().collect();
    let right: String = chars[cursor..].iter().collect();
    write!(out, "\r\x1b[2K{prompt}{left}\x1b[K{right}")?;
    let col = cols_of(prompt) + chars[..cursor].iter().map(|c| display_width(&c.to_string())).sum::<usize>();
    // 光标回位（右移 col 列；col=行宽则停在行尾=右移 0 等价，写之无害）
    write!(out, "\r\x1b[{col}C")?;
    out.flush()
}

/// 尾部差分刷新（光标在行尾且 new 为 prev 扩展时——大粘贴性能路径，
/// task-42 语义收窄保留）：只写新增尾部；不满足条件返回 false（调用方
/// 回退全行重绘）。
pub(crate) fn tail_extend(
    out: &mut impl Write,
    prev: &str,
    new: &str,
) -> std::io::Result<bool> {
    if new.len() > prev.len() && new.starts_with(prev) {
        let added = &new[prev.len()..];
        write!(out, "{added}")?;
        out.flush()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// 输入行上方的反查提示行（Ctrl-R 态；上屏后光标回输入行——调用方紧随 redraw）。
pub(crate) fn search_hint(out: &mut impl Write, hint: &str) -> std::io::Result<()> {
    write!(out, "\r\x1b[K{hint}")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// strip_ansi：CSI 序列剔除（提示符色码不影响宽度）与普通串透传。
    #[test]
    fn strip_ansi_csi剔除与透传() {
        assert_eq!(strip_ansi("\x1b[38;5;245m❯ \x1b[0m"), "❯ ");
        assert_eq!(strip_ansi("普通文本"), "普通文本");
        assert_eq!(strip_ansi(""), "");
        assert_eq!(strip_ansi("a\x1b[1;2Hb"), "ab");
    }

    /// cols_of：CJK 计 2（panel_text::display_width 同源）、色码零宽。
    #[test]
    fn 列宽计算_中文与色码() {
        assert_eq!(cols_of("汉汉"), 4);
        assert_eq!(cols_of("\x1b[31mab\x1b[0m"), 2);
    }

    /// 尾部差分：扩展命中/非扩展（编辑中段）回退全行重绘。
    #[test]
    fn 尾部差分判定() {
        assert_eq!(tail_extend(&mut Vec::new(), "abc", "abcd").unwrap(), true);
        assert_eq!(tail_extend(&mut Vec::new(), "abd", "abcd").unwrap(), false);
        assert_eq!(tail_extend(&mut Vec::new(), "abcd", "abc").unwrap(), false);
    }
}
