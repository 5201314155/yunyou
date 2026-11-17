//! 流式回复视觉块（规范 §4.4 等待指示 / §4.5 流式回复 / 思考内容收敛）。
//!
//! - **混合流式（任务30 三期④）**：未满行**单视觉行可容纳时** `\r\e[K`
//!   原位重绘逐段流出（中文长句无换行符不再憋到段末一次性出——业主实测
//!   痛点）；**将溢出（渲染后可见宽 ≥ 列宽）立即落定**走既有满行语义。
//!   D-UI-1 禁令不破：多视觉行永不重绘——折行残留的根因是对超宽行反复
//!   重绘（`\r\e[K` 只清最后视觉行，前序行旧帧堆积），本机制只在渲染后
//!   仍为单视觉行时重绘（留 1 列余量防 deferred-wrap 终端差异），一旦将
//!   溢出立即一次性落定，折行成多视觉行后永不触碰；判定用渲染后可见宽
//!   （剥 SGR+全角计 2，[`super::tool::visible_width`]），markdown 行内层
//!   剥除的语法标记不计宽。落定行永不回改（§1 铁律 1）；
//! - **满行落定**：Delta 入缓冲，遇 `\n` 的完整行走 markdown+行内层落定；
//! - **markdown 行级简化**（§4.5 CLI 降级规则）：仅对落定行处理——
//!   H1/H2/H3、列表、引用、代码围栏；流中未闭合语法按普通文本先渲染；
//! - **思考流收敛**：见 [`super::thinking`]（活动行计数+冻结单行，原文
//!   不上屏；`YUNYOU_THINKING=full` 逃生口）。

// 单行渲染族与预切行工具拆至子模块（行数清偿批 task-22；纯搬移零行为变更，
// mod 声明在本文件——render/mod.rs 系他席在途件不叠 hunks）。
mod markdown_line;
mod wrap;

use markdown_line::{dim_line, fence_boundary, render_block_line};
use wrap::{prefix_cells, split_display_lines};
pub use markdown_line::markdown_line;

use super::terminal::{freeze_line, rewrite_active_line, Profile, SYM_THINKING_PREFIX};
use super::thinking::{self, ThinkingMode, ThinkingState};
use std::io::Write;

/// 正文/思考两种流内容块；块切换时先终结当前块（落定，不回改）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    #[default]
    Idle,
    /// assistant 正文（markdown 行级）。
    Text,
    /// 思考内容（reasoning）。
    Reasoning,
}

/// 流式渲染状态（按行缓冲；归属 Renderer，事件驱动）。
#[derive(Debug, Default)]
pub struct StreamState {
    block: BlockKind,
    /// 未满行缓冲（无换行尾）。
    pending: String,
    /// 活动行有未清的重绘帧——落定三路（满行/溢出/收尾）先清帧再写，
    /// 防新内容拼在旧帧之后（混合流式引入：旧语义未满行不重绘无此态）。
    painted: bool,
    /// 思考块状态（Collapse 活动行计数与冻结单行；见 [`thinking`]）。
    thinking: ThinkingState,
    /// 最近一次喂入的终端列宽（task-38①：settle 收尾路径预切行要用；宽度
    /// 随每次 stream_chunk 喂入刷新，Default=80 与列宽三层判定兜底一致）。
    width: usize,
    /// 代码围栏状态（task-51③，报告3 Bug-3）：``` 行翻转；围栏内原样直出
    /// 不进 markdown/inline（代码里的 *ptr/a_b_c/反引号 不得被解释——行级
    /// 渲染原为零状态，grep 坐实）。未闭合到回合尾=后半原样（fail-safe）。
    in_code_fence: bool,
    /// 最近一次原位重绘的可见字节量（task-48 接线：自愈检测口径喂入——
    /// 重绘分支置位，追加/冻结路径清零；Renderer 经访问器取走喂检测器）。
    last_rewrite_bytes: usize,
}

impl StreamState {
    /// 是否有未落盘的缓冲内容（回合收尾 [`close_block`] 统一落定用）。
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    /// 最近一次原位重绘的可见字节量（task-48 接线；0=自上次以来无重绘——
    /// 追加/冻结路径显式清零，防陈旧值误喂自愈检测口径）。
    pub fn take_last_rewrite_bytes(&mut self) -> usize {
        std::mem::take(&mut self.last_rewrite_bytes)
    }
}

/// 喂入 assistant 文本增量（块切换 + 混合流式行缓冲）。
pub fn text_delta<W: Write>(
    out: &mut W,
    state: &mut StreamState,
    profile: Profile,
    interactive: bool,
    width: usize,
    text: &str,
) -> std::io::Result<()> {
    if state.block != BlockKind::Text {
        // 思考块切回正文（Collapse）：碎思考吞时正文缓冲原样保留——
        // 正文流无缝续拼（任务30 三期⑥根除拦腰截断）；达标思考正文
        // 先补落再冻结思考行（保持时间序）。其余块切换走全量收口。
        if state.block == BlockKind::Reasoning && state.thinking.collapse {
            match state.thinking.close(out, profile, interactive, width)? {
                thinking::ThinkingClose::Swallowed => {}
                thinking::ThinkingClose::Frozen(line) => {
                    settle_pending(out, state, profile)?;
                    freeze_line(out, &line)?;
                }
            }
        } else {
            close_block(out, state, profile, interactive, width)?;
        }
        state.block = BlockKind::Text;
    }
    stream_chunk(out, state, profile, text, false, interactive, width)
}

/// 喂入思考内容增量（Collapse=活动行计数；Full=全文流式）。
pub fn reasoning_delta<W: Write>(
    out: &mut W,
    state: &mut StreamState,
    profile: Profile,
    interactive: bool,
    mode: ThinkingMode,
    width: usize,
    text: &str,
) -> std::io::Result<()> {
    if mode == ThinkingMode::Full {
        if state.block != BlockKind::Reasoning {
            close_block(out, state, profile, interactive, width)?;
            state.block = BlockKind::Reasoning;
            // 块首：提示这是思考内容（暗淡），随后增量与正文同节奏流淌。
            // task-46①：思考前缀 ▏（Plain 档 `>`）与 markdown 引用的 `│` 拆分
            // 双义——同屏可分辨「引用还是自言自语」；引用行不动。
            let bar = match profile {
                Profile::Plain => ">",
                _ => SYM_THINKING_PREFIX,
            };
            let line = format!(
                "{}{bar}{}{} {}{}",
                profile.color(super::terminal::SGR_TEXT_SECONDARY_256),
                profile.reset(),
                profile.dim(),
                "（思考）",
                profile.reset()
            );
            freeze_line(out, &line)?;
        }
        return stream_chunk(out, state, profile, text, true, interactive, width);
    }
    // Collapse（默认）：只记数与刷活动行，原文不上屏。**不终结正文缓冲**——
    // Collapse 思考不上屏（仅活动行计数，无需为它落正文未满行），碎思考
    // （mid-text 推理碎屑，任务30 三期⑥）结束后正文流无缝续拼。瞬间视觉
    // 交替（正文活动行↔思考活动行）由后续正文增量立即覆盖，不落定任何行。
    if state.block != BlockKind::Reasoning {
        state.block = BlockKind::Reasoning;
        state.thinking.begin();
    }
    state.thinking.delta(out, profile, text, interactive)
}

/// 回合级终结：思考块冻结单行、未满行补落定、复位块状态
/// （TurnDone/收尾路径调用）。
///
/// 未满行若从未上屏，此处按块渲染补输出再落定换行——LLM 回复的最后一段
/// 不丢字（审查 P1-1，交互态同此路径）；若已原位重绘过，先清帧再落定。
pub fn close_block<W: Write>(
    out: &mut W,
    state: &mut StreamState,
    profile: Profile,
    interactive: bool,
    width: usize,
) -> std::io::Result<()> {
    // 思考块收口（Collapse）：碎思考吞（不落行——活动行由 close 内清），
    // 达标思考正文 pending 先补落（时间序）再冻结单行（「落定不回改」不破
    // ——活动行不是落定行，冻结是该块的第一条落定记录）。
    // Full 逃生口无活动行无冻结行，思考文本在 pending 由下方统一落定。
    if state.block == BlockKind::Reasoning && state.thinking.collapse {
        match state.thinking.close(out, profile, interactive, width)? {
            thinking::ThinkingClose::Swallowed => {}
            thinking::ThinkingClose::Frozen(line) => {
                settle_pending(out, state, profile)?;
                freeze_line(out, &line)?;
            }
        }
    }
    settle_pending(out, state, profile)?;
    state.block = BlockKind::Idle;
    Ok(())
}

/// 正文/思考未满行落定（清重绘帧后按块渲染补输出；pending 清空）。
/// task-38①：收尾路径同规预切——回合收尾时 pending 可能已超列宽（最后一段
/// 未触发冻结分支即收尾），整行直写同样产生终端硬折孤字。
fn settle_pending<W: Write>(
    out: &mut W,
    state: &mut StreamState,
    profile: Profile,
) -> std::io::Result<()> {
    if state.has_pending() {
        super::terminal::clear_active_line(out, state.painted)?;
        state.last_rewrite_bytes = 0; // task-48：收尾落定不计重绘口径
        if state.in_code_fence {
            // task-51③：围栏内未落定行收尾——原样 dim（不进 inline/markdown）
            for segment in split_display_lines(&state.pending, state.width.saturating_sub(1)) {
                write!(out, "{}{}{}", profile.dim(), segment, profile.reset())?;
                writeln!(out)?;
            }
            state.pending.clear();
            state.painted = false;
            return Ok(());
        }
        let budget = state.width.saturating_sub(prefix_cells(state.block == BlockKind::Reasoning));
        for segment in split_display_lines(&state.pending, budget) {
            let rendered = render_block_line(profile, &segment, state.block == BlockKind::Reasoning);
            write!(out, "{rendered}")?;
            writeln!(out)?; // 未满行落定（规范：未闭合语法按普通文本先渲染）
        }
        state.pending.clear();
    }
    state.painted = false;
    Ok(())
}

/// 块内增量处理（混合流式，任务30 三期④）：
/// 按 \n 切分完整行走 markdown 落定（清帧后写，防拼旧帧）；未满行
/// 单视觉行可容纳则原位重绘逐段流出，将溢出立即落定——取舍论证见
/// 模块注释（D-UI-1 多视觉行永不重绘）。
fn stream_chunk<W: Write>(
    out: &mut W,
    state: &mut StreamState,
    profile: Profile,
    text: &str,
    reasoning: bool,
    interactive: bool,
    width: usize,
) -> std::io::Result<()> {
    state.pending.push_str(text);
    state.width = width; // task-38①：收尾 settle 预切行要用最近列宽
    while let Some(pos) = state.pending.find('\n') {
        settle_full_line(out, state, profile, reasoning, width, pos)?;
    }
    if !state.pending.is_empty() {
        feed_pending(out, state, profile, reasoning, interactive, width)?;
    }
    Ok(())
}

/// 完整行落定（原 stream_chunk while 体，行数清偿批 task-22 拆出；清帧→
/// 围栏状态机→预切/直落，逐段语义原样）。
fn settle_full_line<W: Write>(
    out: &mut W,
    state: &mut StreamState,
    profile: Profile,
    reasoning: bool,
    width: usize,
    pos: usize,
) -> std::io::Result<()> {
    // 完整行：出缓冲 → 渲染 → 清活动帧 → 落定
    let line: String = state.pending.drain(..=pos).collect();
    let line = line.trim_end_matches('\n');
    super::terminal::clear_active_line(out, state.painted)?;
    state.painted = false;
    state.last_rewrite_bytes = 0; // task-48：落定路径不计重绘口径
    // task-51③：围栏状态机——``` 行翻转边界；围栏内原样直出（至多 dim），
    // 不进 markdown/inline（代码里的 *ptr/a_b_c/反引号 不得被解释）。
    // 未闭合到回合尾=后半原样（fail-safe 可接受）。
    if line.trim_start().starts_with("```") {
        state.in_code_fence = !state.in_code_fence;
        freeze_line(out, &fence_boundary(profile, line))?;
        return Ok(());
    }
    if state.in_code_fence {
        for segment in split_display_lines(line, width.saturating_sub(1)) {
            freeze_line(out, &dim_line(profile, &segment))?;
        }
        return Ok(());
    }
    // task-38①：满行同规预切——散文段落=带换行的完整行，宽于列宽时旧路径
    // 整行落定 → 终端硬折孤字残行（场景 D 实证 5 处）。以原文宽作保守触发
    // （行内层只剥不增，渲染后只会更窄）。
    if super::tool::visible_width(line) >= width {
        let budget = width.saturating_sub(prefix_cells(reasoning));
        for segment in split_display_lines(line, budget) {
            let rendered = render_block_line(profile, &segment, reasoning);
            freeze_line(out, &rendered)?;
        }
    } else {
        let rendered = render_block_line(profile, line, reasoning);
        freeze_line(out, &rendered)?;
    }
    Ok(())
}

/// 未满行喂入（原 stream_chunk 尾段，行数清偿批 task-22 拆出）：围栏内溢出
/// 直落 / 将溢出预切落定 / 单视觉行原位重绘三路。
fn feed_pending<W: Write>(
    out: &mut W,
    state: &mut StreamState,
    profile: Profile,
    reasoning: bool,
    interactive: bool,
    width: usize,
) -> std::io::Result<()> {
    if state.in_code_fence {
        // task-51③：围栏内未满行溢出→原样 dim 落定（不进 inline）
        super::terminal::clear_active_line(out, state.painted)?;
        state.painted = false;
        for segment in split_display_lines(&state.pending, width.saturating_sub(1)) {
            freeze_line(out, &dim_line(profile, &segment))?;
        }
        state.pending.clear();
        return Ok(());
    }
    let rendered = render_block_line(profile, &state.pending, reasoning);
    // 宽度单算（task-36②，挑刺 P3-2）：同一 rendered 的 visible_width 在
    // 溢出判定与重绘记账各扫一遍=O(L)×2/delta——判定值存局部复用。
    let rendered_width = super::tool::visible_width(&rendered);
    if rendered_width >= width {
        // 将溢出：立即落定（task-38①：按显示宽度**预切行**——应用层折行，
        // 不给终端折。孤字残行根因=chunk 越界使落定行超物理宽（如 96 格
        // pending 一笔进 106 格），终端硬折出 2-4 字孤行且永不重排、resize
        // 不回流；预切后每段 ≤ 列宽，折行语义收敛在应用层）。预算按渲染前
        // 原文宽度计：行内渲染只剥不增可见宽，分段后逐段渲染只会更窄。
        super::terminal::clear_active_line(out, state.painted)?;
        state.painted = false;
        let budget = width.saturating_sub(prefix_cells(reasoning));
        for segment in split_display_lines(&state.pending, budget) {
            let line = render_block_line(profile, &segment, reasoning);
            freeze_line(out, &line)?;
        }
        state.pending.clear();
    } else if interactive {
        // 单视觉行可容纳：原位重绘逐段流出（非交互无 \r 语义，留缓冲）
        rewrite_active_line(out, &rendered)?;
        state.painted = true;
        // task-48 接线：重绘字节数记账（自愈口径=原位改写字节，含前缀与
        // SGR 允差常量——粗口径配粗阈值足够）。
        state.last_rewrite_bytes = rendered_width + prefix_cells(reasoning) + 16;
    }
    Ok(())
}

// markdown_line/fence_boundary/dim_line/render_block_line → stream/markdown_line.rs；
// prefix_cells/split_display_lines+split_tests → stream/wrap.rs（task-22 搬移）。

