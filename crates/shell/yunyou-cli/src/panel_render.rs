//! panel 帧渲染段（task-33⑤b 预防拆：主体贴 400 硬顶，render_frame/collapse
//! 两方法独立成文件——impl Panel 跨文件续写（同 crate 合法），零语义改动。

use super::*;

impl Panel {
    /// 渲染一帧：重绘输入行 + 候选区（两列/截断/封顶/滚动窗口/选中高亮）。
    ///
    /// 调用方保证光标位于输入行（上一帧结束或新提示符处）；返回后光标仍回输入行。
    /// 残影处理（审三 P2-1）：候选由多变少时旧帧多画的行会留屏——帧首记上一帧
    /// 行数 `old`，画毕若 `old` 更多则 `\e[J` 清光标至屏底（只擦我方暂态行，
    /// 规范 §5 条款 5 已定性），再按新行数上移回输入行。
    pub fn render_frame<W: Write>(
        &mut self,
        out: &mut W,
        profile: Profile,
        prompt: &str,
        buffer: &str,
        width: usize,
    ) -> std::io::Result<()> {
        let old_rows = self.drawn_rows;
        // 候选可见窗口：8 行封顶，含选中项；溢出时末行让位给「共 N 项」提示
        let total = self.candidates.len();
        let (start, shown, overflow) = if total <= MAX_PANEL_ROWS {
            (0, total, 0usize)
        } else {
            let rows_for_items = MAX_PANEL_ROWS - 1; // 末行留给溢出提示
            let start = self
                .selected
                .saturating_sub(rows_for_items - 1)
                .min(total - rows_for_items);
            (start, rows_for_items, total - rows_for_items)
        };
        // task-66 P1-2：width<40 单列降级（候选名为主，描述列让位——窄终端
        // 两列布局必折行破坏帧序）；截断基准=调用方注入实测宽（参数化替代
        // 逐帧 ioctl，帧渲染可单测）。
        let single_column = width < 40;
        let col1_width = if single_column {
            0
        } else {
            (start..start + shown)
                .map(|i| display_width(&first_column(&self.candidates[i])))
                .max()
                .unwrap_or(0)
                + 2
        };
        // 光标复位到输入行行首（上一帧后光标已在输入行），清行重绘输入行
        crate::render::terminal::rewrite_input_line(out, &format!("{prompt}{buffer}"))?;
        // 提示行（发现性票③）：候选区首行上方一行操作提示，随面板同画同擦
        if self.hint && shown > 0 {
            crate::render::terminal::append_cleared_line(
                out,
                &format!(
                    "{}（输入过滤 · ↑↓ 选 · Tab 补全 · Enter 执行 · Esc 收）{}",
                    profile.dim(),
                    profile.reset()
                ),
            )?;
        }
        // 逐行写候选区：每行 append_cleared_line 换行+原位清写（与思考活动行同机制）
        for index in start..start + shown {
            let cmd = &self.candidates[index];
            let marker = if index == self.selected { profile.bold() } else { "" };
            let marker_off = if marker.is_empty() { "" } else { profile.reset() };
            let col1 = truncate_width(&first_column(cmd), col1_width);
            let pad = " ".repeat(col1_width.saturating_sub(display_width(&col1)));
            // 截断基准=注入宽（task-66 P1-2 参数化；单列降级时仅候选名）
            let row_text = if single_column {
                col1.clone()
            } else {
                format!("{col1}{pad}{}", cmd.description)
            };
            let row = truncate_width(&row_text, width.saturating_sub(2));
            crate::render::terminal::append_cleared_line(out, &format!("{marker}{row}{marker_off}"))?;
        }
        if overflow > 0 {
            crate::render::terminal::append_cleared_line(out, &format!("…共 {total} 项，↑↓/Tab 循环"))?;
        }
        let hint_rows = usize::from(self.hint && shown > 0);
        self.drawn_rows = hint_rows + if overflow > 0 { shown + 1 } else { shown };
        // 残影差额擦除（P2-1）：旧帧比新帧多画的行留在屏上，从新帧末行向下
        // 清到屏底（\e[J 只擦我方暂态行，不触碰更下方的历史落定行之外的内容——
        // 面板区下方本无他物：输入行之下全是面板暂态）
        if old_rows > self.drawn_rows {
            write!(out, "\x1b[J")?;
        }
        if self.drawn_rows > 0 {
            write!(out, "\x1b[{}A", self.drawn_rows)?; // 上移回输入行
            write!(out, "\r")?;
        }
        out.flush()
    }

    /// 收起面板：清掉本面板画的候选区，只留输入行（Esc 或过滤空时调用）。
    pub fn collapse<W: Write>(
        &mut self,
        out: &mut W,
        prompt: &str,
        buffer: &str,
    ) -> std::io::Result<()> {
        self.candidates.clear();
        self.selected = 0;
        self.drawn_rows = 0;
        // 光标在输入行：重绘输入行后 \e[J 清下方全部暂态行（不回改历史）
        crate::render::terminal::rewrite_input_line(out, &format!("{prompt}{buffer}\x1b[J"))?;
        out.flush()
    }
}

#[cfg(test)]
mod tests {
    /// task-54 逃逸锁：面板渲染源码不得直写 \r\e[K——收编后回归防线（新文件
    /// 不再逃逸；grep 全仓口径的文件级测试化）。
    #[test]
    fn 面板渲染无裸逃逸直写() {
        let source = include_str!("panel_render.rs");
        assert!(
            !source.contains("\r\x1b[K"),
            "panel_render.rs 不得直写 \\r\\e[K——一律走 terminal 原语（task-54 收编纪律）"
        );
    }
}
