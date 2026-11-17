//! 树状分支视觉块（规范 §4.8：并行/嵌套工具调用的 ├─ └─ 缩进）。
//!
//! 字段定案（云枢裁「加」）：`parent_tool_call_id`（可空）+ `batch_id`（可空）；
//! 按 parent 组装层级、按 batch 聚并行批次；缺省（历史会话回放）退化为平铺。
//! 缩进单位 2 空格，最大深度 3 层，超出以 … 合并。

use super::terminal::{ascii, Profile, SYM_TREE_BRANCH, SYM_TREE_CORNER, SYM_TREE_PIPE};

/// 单位缩进（tokens space.2 → CLI 2 空格）。
const INDENT_UNIT: &str = "  ";
/// 最大树深（超过以 … 合并）。
const MAX_DEPTH: usize = 3;

/// 按事件携带的 parent 关联计算缩进前缀（树干 │ 与枝线 ├─/└─ 皆烟灰，规范 §4.8）。
///
/// `depth` 由调用方的树归集器给出（REPL 接线层按 call_id→parent 映射计算）；
/// 超过 MAX_DEPTH 返回 `…` 合并占位。
pub fn indent_prefix(profile: Profile, depth: usize, is_last: bool) -> String {
    if depth == 0 {
        return String::new();
    }
    if depth > MAX_DEPTH {
        return format!("{INDENT_UNIT}{}{}{}", profile.color(super::terminal::SGR_TEXT_SECONDARY_256), "…", profile.reset());
    }
    let sgr = profile.color(super::terminal::SGR_TEXT_SECONDARY_256);
    let reset = profile.reset();
    let pipe = profile.symbol(SYM_TREE_PIPE, ascii::TREE_PIPE);
    let mut out = String::new();
    for _ in 1..depth {
        // 树干 │：烟灰（此前漏着色，nit 批修正——规范 §4.8「树干 │ 与枝线用烟灰」）
        out.push_str(INDENT_UNIT);
        out.push_str(&sgr);
        out.push_str(pipe);
        out.push_str(reset);
        out.push(' ');
    }
    out.push_str(INDENT_UNIT);
    out.push_str(&sgr);
    out.push_str(profile.symbol(
        if is_last { SYM_TREE_CORNER } else { SYM_TREE_BRANCH },
        if is_last { ascii::TREE_CORNER } else { ascii::TREE_BRANCH },
    ));
    out.push_str(reset);
    out
}

/// 层级深度上限裁剪：树归集器把深层子树折叠到 MAX_DEPTH。
pub fn clamp_depth(depth: usize) -> usize {
    if depth > MAX_DEPTH { MAX_DEPTH + 1 } else { depth }
}
