//! 思考流收敛（任务30 二期②立；三期④自 stream.rs 按职责拆出，承接⑤节流
//! 与⑥碎思考抑制增量）。
//!
//! 默认不倾倒：流式中仅活动行「│（思考中… N 字）」原位重绘（复用
//! `\r\e[K` 机制＋非交互静默判据），块终/回合收尾冻结单行「▏ 想了 X.Xs ·
//! N 字」（dim，块首起算）；原始思考默认不上屏——思考过程属中间产物，
//! 上屏会淹没正文主信号，且通常冗长（全文仍完整入会话 JSONL，可回查）。
//! 「落定不回改」不破——思考行从未落定，只活动行重绘。
//! `YUNYOU_THINKING=full` 保留全文流式逃生口（默认关，烟灰 dim + `│` 直出）。

use super::terminal::{clear_active_line, rewrite_active_line, Profile, SGR_TEXT_SECONDARY_256};
use std::io::Write;
use std::time::{Duration, Instant};

/// 思考活动行重绘最小间隔（任务30 三期⑤：80ms→200ms）。
///
/// 取舍自证：LLM 思考增量到达节奏远快于人眼感知（数十毫秒连发），
/// 逐增量重绘在手机 SSH 高延迟链路上与终端 `\r\e[K` 行为叠加产生
/// 「滚动堆积」观感异常（业主真机反馈）；200ms 与人眼对纯数字变化的
/// 感知阈值匹配——计数仍平滑增长，重绘频次降一个量级。节流只作用于
/// 暂态活动行；冻结单行（最终字数）不受影响，收尾信息无损。
const THINKING_REDRAW_INTERVAL: Duration = Duration::from_millis(200);

/// 思考冻结行首句预览的可见列宽（T3-a 大改，设计稿件三 2026-11-17）：
/// 旧 22 列硬顶（任务30 四期 1.4）≈10 个汉字几乎无信息量——改按当前列宽
/// 预算（=列宽−20：前缀+「想了 X.Xs · 」固定开销+行尾呼吸），120 列终端
/// 给 100 列预览；head 收集封顶 [`THINKING_PREVIEW_MAX_COLS]（内存上界），
/// 落定再按预算裁（resize 后预算变化不吃亏）。截断以可见列计（CJK 2 列），
/// 换行压为空格保单行形态。
const THINKING_PREVIEW_MAX_COLS: usize = 80;

/// 冻结行预览预算：列宽 − 固定开销 20（`▏ 想了 88.8s · `≈15 列+呼吸 5；
/// 秒数位数波动由预算余量吸收）。
fn preview_budget(width: usize) -> usize {
    width.saturating_sub(20)
}

/// 碎思考判定阈值（任务30 三期⑥）：收尾累计字数低于此值视为 mid-text
/// 推理碎屑（如 1 字），不落冻结行直接吞。
///
/// 阈值自证：真实思考块（哪怕是短回复的开场权衡）几乎必然超过 20 字；
/// 模型在正文 mid-text 插入的推理碎屑典型为个位数字符。连发抖动合并
/// 策略：同一思考块内的碎屑增量累计计数，收尾统一判定——多次 1 字
/// 碎屑若累计仍不足阈值则一并吞掉，不逐次落行抖动；累计达标则正常
/// 冻结，不丢思考痕迹。
const THINKING_FRAG_THRESHOLD: usize = 20;

/// 思考流模式：默认收敛为活动行＋冻结单行；full 为全文流式逃生口。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    /// 收敛（默认）：流式中只刷活动行计数，块终冻结「│ 想了 X.Xs · N 字」。
    #[default]
    Collapse,
    /// 全文流式（YUNYOU_THINKING=full）：烟灰 dim + `│` 前缀直出（旧行为）。
    Full,
}

/// 思考块收口形态（[`ThinkingState::close`] 返回；落定顺序由调用方编排）。
pub(super) enum ThinkingClose {
    /// 碎思考（<20 字）：整体吞——不落冻结行（mid-text 推理碎屑落行会把
    /// 正文拦腰截断，业主实测）；调用方保留正文缓冲无缝续拼。
    Swallowed,
    /// 达标思考：落冻结单行（调用方先补落正文 pending 再冻结，保持时间序）。
    Frozen(String),
}

/// 思考块状态（归属 StreamState；Collapse 活动行计数与冻结单行用）。
#[derive(Debug, Default)]
pub struct ThinkingState {
    /// 思考块首时刻（冻结单行耗时起算点；块终取走）。
    started: Option<Instant>,
    /// 思考块累计字符数（活动行计数与冻结单行共用）。
    chars: usize,
    /// 思考正文首句预览（任务30 四期 1.4；增量到达时累计至
    /// [`THINKING_PREVIEW_COLS] 可见列即封顶——预览只需要开头，不全文缓存）。
    head: String,
    /// 上次活动行重绘时刻（节流判据；None=本块尚未绘过）。
    last_paint: Option<Instant>,
    /// 节流间隔（测试注入用；生产恒 [`THINKING_REDRAW_INTERVAL]）。
    paint_interval: Duration,
    /// 当前思考块是否 Collapse 收敛（块切回正文时据此分流收口路径；
    /// Full 逃生口不进 begin、恒 false）。
    pub(super) collapse: bool,
}

impl ThinkingState {
    /// 进思考块（Collapse）：起点复位——连发思考块各自独立计时计字，
    /// 不跨块累计（两段不相邻的思考合算会虚报单块时长）。
    pub(super) fn begin(&mut self) {
        self.started = Some(Instant::now());
        self.chars = 0;
        self.head.clear();
        self.last_paint = None;
        self.paint_interval = THINKING_REDRAW_INTERVAL;
        self.collapse = true;
    }

    /// 喂思考增量（Collapse）：计数照常累计（计数无损），活动行重绘按
    /// [`THINKING_REDRAW_INTERVAL] 节流——非交互静默（\r 在管道/dumb 终端
    /// 不回位，Renderer 判据同款）；节流窗口内丢弃的只是重绘帧，最终
    /// 字数由冻结单行呈现。
    pub(super) fn delta<W: Write>(
        &mut self,
        out: &mut W,
        profile: Profile,
        text: &str,
        interactive: bool,
    ) -> std::io::Result<()> {
        self.chars += text.chars().count();
        // 首句预览累计：未封顶才收，换行压空格（冻结行单行形态），按收集
        // 上界封顶——落定时再按列宽预算裁（T3-a：预览「它开头在想什么」）
        if self.head_capacity() > 0 {
            for ch in text.chars() {
                let w = if ch == '\n' {
                    ' '
                } else {
                    ch
                };
                if w == ' ' && self.head.ends_with(' ') {
                    continue; // 折叠连续空白
                }
                let cw = super::tool::visible_width(&w.to_string());
                if cw > self.head_capacity() {
                    break; // 本字符放不下即封顶（下一增量重新探测）
                }
                self.head.push(w);
            }
        }
        if !interactive {
            return Ok(());
        }
        let now = Instant::now();
        let due = self
            .last_paint
            .is_none_or(|t| now.duration_since(t) >= self.paint_interval);
        if due {
            let elapsed = self.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            rewrite_active_line(out, &active_line(profile, elapsed))?;
            self.last_paint = Some(now);
        }
        Ok(())
    }

    /// 首句预览剩余可见列容量（封顶后为 0，停止收集）。
    fn head_capacity(&self) -> usize {
        THINKING_PREVIEW_MAX_COLS.saturating_sub(super::tool::visible_width(&self.head))
    }

    /// 收口思考块（任务30 三期⑥）：清活动行（交互且有重绘痕迹）后按碎思考
    /// 阈值分流——<20 字吞（Swallowed），≥20 字落冻结单行（Frozen）。
    /// 状态无条件复位（连发碎块各自独立判定）。正文 pending 的落定顺序
    /// 由调用方按返回值编排（达标=正文先落保持时间序）。
    /// `width`：当前列宽（T3-a——冻结行预览按列宽预算，窄终端自动收缩）。
    pub(super) fn close<W: Write>(
        &mut self,
        out: &mut W,
        profile: Profile,
        interactive: bool,
        width: usize,
    ) -> std::io::Result<ThinkingClose> {
        let painted = self.last_paint.take().is_some();
        let started = self.started.take();
        let chars = std::mem::take(&mut self.chars);
        let head = std::mem::take(&mut self.head);
        self.collapse = false;
        clear_active_line(out, interactive && painted)?;
        match started {
            Some(started) if chars >= THINKING_FRAG_THRESHOLD => Ok(ThinkingClose::Frozen(
                frozen_line(profile, started.elapsed(), &head, preview_budget(width)),
            )),
            // 碎思考（<20 字）或无起点（异常态防御）：吞
            _ => Ok(ThinkingClose::Swallowed),
        }
    }
}

/// Collapse 活动行：`▏ 思考中… Ns`（dim；`\r\e[K` 原位重绘由调用方承担）。
/// T3-a 大改（设计稿件三）：秒数上活动行（比字数计数更有决策价值，业主
/// 点名「思考了多少秒」），字数计数退役入冻结行。task-45 前缀 ▏ 不变
/// （Plain 档 `>`）——活动行与冻结行同前缀，防生命周期割裂。
fn active_line(profile: Profile, elapsed_secs: u64) -> String {
    let prefix = match profile {
        Profile::Plain => ">",
        _ => crate::render::terminal::SYM_THINKING_PREFIX,
    };
    format!(
        "{}{}{}{} 思考中… {elapsed_secs}s{}",
        profile.color(SGR_TEXT_SECONDARY_256),
        prefix,
        profile.reset(),
        profile.dim(),
        profile.reset()
    )
}

/// Collapse 冻结单行：`▏ 想了 X.Xs · 首句预览…`（dim；块终/回合收尾各至
/// 多一次；预览按列宽预算（T3-a，设计稿件三）——秒数之外首句才是「想什么」
/// 的信号；超预算以 … 收尾，未满不补点）。前缀 ▏ 对齐 46①（Plain `>`）。
pub(super) fn frozen_line(
    profile: Profile,
    elapsed: std::time::Duration,
    head: &str,
    budget: usize,
) -> String {
    // clip_width 自带「超预算按 budget-1 切+…」语义——此处不再叠第二枚
    // 省略号（首版双 `……` 缺陷回归锁：测试 thinking/degrade 两族守）
    let preview = if head.is_empty() {
        String::new()
    } else {
        format!(" · {}", super::tool::clip_width(head, budget))
    };
    let prefix = match profile {
        Profile::Plain => ">",
        _ => crate::render::terminal::SYM_THINKING_PREFIX,
    };
    format!(
        "{}{}{}{} 想了 {:.1}s{preview}{}",
        profile.color(SGR_TEXT_SECONDARY_256),
        prefix,
        profile.reset(),
        profile.dim(),
        elapsed.as_secs_f64(),
        profile.reset()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::terminal::Profile;

    /// 节流三态：首增量必绘；节流窗口内连发只计数不重绘；窗口过后恢复绘。
    /// 间隔经 paint_interval 注入（测试免真实等待）；字符计数全程无损。
    #[test]
    fn 节流_首绘_窗口内不绘_过窗恢复() {
        let mut state = ThinkingState::default();
        state.begin();
        state.paint_interval = Duration::from_millis(50);
        let mut out: Vec<u8> = Vec::new();
        let profile = Profile::Plain; // 无色档：输出只剩 \r\e[K 与纯文本，断言最简
        state.delta(&mut out, profile, "第一", true).unwrap();
        assert_eq!(state.chars, 2);
        let paints_after_first = out.iter().filter(|b| **b == b'\r').count();
        assert_eq!(paints_after_first, 1, "首增量必绘：{out:?}");
        // 窗口内连发：计数照加，零重绘
        state.delta(&mut out, profile, "第二", true).unwrap();
        state.delta(&mut out, profile, "第三", true).unwrap();
        assert_eq!(state.chars, 6, "计数无损：{state:?}");
        assert_eq!(
            out.iter().filter(|b| **b == b'\r').count(),
            paints_after_first,
            "节流窗口内不得重绘：{out:?}"
        );
        // 窗口过后：恢复重绘
        std::thread::sleep(Duration::from_millis(60));
        state.delta(&mut out, profile, "第四", true).unwrap();
        assert_eq!(state.chars, 8);
        assert!(
            out.iter().filter(|b| **b == b'\r').count() > paints_after_first,
            "窗口过后应恢复重绘：{out:?}"
        );
    }

    /// 非交互静默不因节流改变：任何增量都零输出（\r 在管道不回位）。
    #[test]
    fn 非交互_静默不变() {
        let mut state = ThinkingState::default();
        state.begin();
        let mut out: Vec<u8> = Vec::new();
        state.delta(&mut out, Profile::Plain, "思考", false).unwrap();
        state.delta(&mut out, Profile::Plain, "继续", false).unwrap();
        assert_eq!(state.chars, 4);
        assert!(out.is_empty(), "非交互零输出：{out:?}");
    }
}