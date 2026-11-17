//! 块内单行渲染族（自 stream.rs 按职责拆出，行数清偿批 task-22；纯搬移
//! 零行为变更）：markdown 行级渲染（规范 §4.5 CLI 降级规则的一期子集）+
//! 行内层调度 + 思考行烟灰 + 围栏边界。

use crate::render::inline;
use crate::render::terminal::{ascii, Profile, SYM_BULLET};

/// 块内单行渲染（正文 markdown+行内层；思考烟灰 dim + ▏ 前缀——task-46①
/// 与引用 `│` 拆分双义，Plain 档 `>`）。
pub(super) fn render_block_line(profile: Profile, line: &str, reasoning: bool) -> String {
    // task-65 P0-1：入口单点净化——正文块行系模型可见文本（流式增量聚合），
    // 外来 CSI/OSC/C0 剥除（\n 已由上游行拆分处理，此处无换行语义）。
    let line = &super::super::sanitize::sanitize_text(line);
    if reasoning {
        let bar = match profile {
            Profile::Plain => ">",
            _ => crate::render::terminal::SYM_THINKING_PREFIX,
        };
        format!("{bar} {}", dim_line(profile, line))
    } else {
        markdown_line(profile, line)
    }
}

/// 围栏边界行（task-46②/51③）：`· ─── [信息串]`，反引号不上屏、语言标记保留。
pub(super) fn fence_boundary(profile: Profile, line: &str) -> String {
    let info = line.trim().trim_start_matches('`').trim();
    let body = if info.is_empty() {
        "· ───".to_string()
    } else {
        format!("· ─── {info}")
    };
    format!("{}{body}{}", profile.dim(), profile.reset())
}

/// markdown 行级渲染（规范 §4.5 CLI 降级规则的一期子集）＋行内层（任务 30-①）。
pub fn markdown_line(profile: Profile, line: &str) -> String {
    let trimmed = line.trim_start();
    let brand = profile.color(crate::render::terminal::SGR_BRAND_PRIMARY_256);
    let bold = profile.bold();
    let reset = profile.reset();
    let heading = |level: usize| -> Option<String> {
        let prefix = "#".repeat(level);
        let rest = trimmed.strip_prefix(&prefix)?;
        (!rest.starts_with('#')).then(|| {
            // H1 粗体+云青；H2 仅粗体；H3 云青常规（规范 §2 字体层级映射）
            match level {
                1 => format!("{bold}{brand}{}{reset}", rest.trim()),
                2 => format!("{bold}{}{reset}", rest.trim()),
                _ => format!("{brand}{}{reset}", rest.trim()),
            }
        })
    };
    if let Some(rendered) = heading(1).or_else(|| heading(2)).or_else(|| heading(3)) {
        return rendered;
    }
    // task-51④：H4+ 兜底——三连败后 #### 及更深标题按 H3 同款渲染、剥全部
    // 前导 #（LLM 长文档 #### 不罕见；「#### 」原样穿透是一期子集的账）。
    // markdown 严格语法：# 串后须有空格才算标题。
    {
        let stripped = trimmed.trim_start_matches('#');
        let hashes = trimmed.len() - stripped.len();
        if hashes >= 4 && stripped.starts_with(' ') {
            return format!("{brand}{}{reset}", stripped.trim());
        }
    }
    // 列表：• 替换 -/*/+
    let list = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "));
    if let Some(item) = list {
        return format!(
            "{brand}{}{reset} {item}",
            profile.symbol(SYM_BULLET, ascii::BULLET)
        );
    }
    // 引用：│ 前缀 + 烟灰
    if let Some(quote) = trimmed.strip_prefix("> ") {
        return format!(
            "{}│{} {}",
            profile.color(crate::render::terminal::SGR_TEXT_SECONDARY_256),
            profile.reset(),
            quote
        );
    }
    // 代码围栏行（task-51③）：状态机接管后 markdown_line 不再见到围栏行
    // （stream_chunk 先行拦截翻转并走 fence_boundary）——本分支留作防御性
    // 兜底（直接调用 markdown_line 的测试/未来调用方），形态与状态机一致。
    if trimmed.starts_with("```") {
        return fence_boundary(profile, line);
    }
    // 行内层（任务 30-①）：代码/粗体/斜体，未闭合原样
    inline::render_inline(profile, line)
}

/// 思考块行：烟灰 dim 全行（tokens text.secondary；无色档仅剩缩进）。
pub(super) fn dim_line(profile: Profile, line: &str) -> String {
    format!(
        "{}{}{}{}",
        profile.dim(),
        profile.color(crate::render::terminal::SGR_TEXT_SECONDARY_256),
        line,
        profile.reset()
    )
}
