//! 按显示宽度预切行工具（自 stream.rs 按职责拆出，行数清偿批 task-22；
//! 纯搬移零行为变更）：应用层折行，不给终端硬折（task-38①）。


/// 前缀可见宽（task-38①）：思考行带「▏ 」前缀（unicode-width 口径 ▏=1，task-46① 起），
/// 正文行无前缀。预切预算从列宽里扣掉它，保证「前缀+段」不超物理宽。
pub(super) fn prefix_cells(reasoning: bool) -> usize {
    if reasoning { 2 } else { 0 }
}

/// 按显示宽度预切行（task-38①）：把一段未满行原文切成若干 ≤ budget 显示格
/// 的段（贪心装箱，CJK 按字符原子切，不在格内劈半；零控制符假设——pending
/// 是模型增量原文，SGR 由渲染层追加）。空文本产出空段表（调用方有非空守卫）。
pub(super) fn split_display_lines(text: &str, budget: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_cells = 0usize;
    for ch in text.chars() {
        let w = crate::render::tool::char_width(ch);
        if current_cells + w > budget && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_cells = 0;
        }
        current.push(ch);
        current_cells += w;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod split_tests {
    use super::*;

    /// task-38① 核心回归：100 列终端、200 连续 CJK（原「两百个云」场景）——
    /// 每段严格 ≤ 预算格，无超宽段（超宽落定=孤字残行根因）。
    #[test]
    fn 长连串_按预算切齐() {
        let text = "云".repeat(200);
        let lines = split_display_lines(&text, 100);
        assert_eq!(lines.len(), 4, "200 字×2 格=400 格，100 预算应切 4 段：{lines:?}");
        for line in &lines {
            assert_eq!(super::super::super::tool::visible_width(line), 100, "每段恰满预算：{line:?}");
        }
        assert_eq!(lines.concat(), text, "切行不丢字：拼接应还原原文");
    }

    /// 散文场景：51 字 CJK（102 格）在 100 列下切 50+1，第二段是 1 字孤行——
    /// 但孤行由**应用层**落定且每段 ≤ 列宽（终端不再自行硬折）。
    #[test]
    fn 越界一笔_切两段() {
        let text = "夜".repeat(51);
        let lines = split_display_lines(&text, 100);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chars().count(), 50);
        assert_eq!(lines[1], "夜");
    }

    /// 短文本不切（预算内原样单段）；空文本不出空段。
    #[test]
    fn 短文本与空文本() {
        assert_eq!(split_display_lines("云悠", 100), vec!["云悠".to_string()]);
        assert!(split_display_lines("", 100).is_empty());
    }

    /// ASCII 与 CJK 混排：按显示格贪心，不在格内劈半（CJK 原子）。
    #[test]
    fn 混排_原子切() {
        let text = format!("{}{}", "a".repeat(99), "云");
        let lines = split_display_lines(&text, 100);
        assert_eq!(lines.len(), 2, "99 格 ASCII+2 格 CJK=101 格应切两段：{lines:?}");
        assert_eq!(lines[0].len(), 99, "首段止于预算内");
        assert_eq!(lines[1], "云", "CJK 不被劈半");
    }

    /// 预算为 0（极端窄端 width<4，不支持场景）不死循环：每字符一段的退化
    /// 但有限——task-51⑥：本测试只锁「有限段数」，**不承诺段 ≤ 预算**
    /// （width<4 视为不支持场景，WIDE_MIN_COLS 之外的行为不承诺；
    /// 勿把本测试当正确性证据——正确性锚在列宽三层判定的 20 列下限）。
    #[test]
    fn 零预算_有限段_不保证达预算() {
        let lines = split_display_lines("云云", 0);
        assert_eq!(lines, vec!["云", "云"]);
    }
}
