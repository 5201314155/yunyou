//! apply_patch 补丁文法解析（task-22；算法依据 docs/架构/参照/apply-patch算法拆解报告.md
//! §一，Apache-2.0 项目行为研究，实现为本仓自写）。
//!
//! # 文法
//! ```text
//! *** Begin Patch
//! *** Add File: <path>        内容行一律 "+" 前缀
//! *** Delete File: <path>
//! *** Update File: <path>     可选 "*** Move to: <path>"，后接 1..n 个 chunk
//! *** Move to: <path>
//! @@ [单行锚点]               chunk 头
//!  上下文行（空格前缀）
//! -删除行
//! +新增行
//! *** End of File             可选：chunk 锚死文件尾
//! *** End Patch
//! ```
//! 一个 Update hunk 可含多 chunk；一个补丁可含多 hunk 覆盖多文件。
//!
//! # 宽容层（只两样，研究报告 §1.3 取舍）
//! marker 行首尾 trim 后比对；heredoc 壳剥离（模型把补丁包进 `<<EOF … EOF`
//! 的误传形态，整体 ≥4 行且首尾为 heredoc 壳时剥壳重析）。
//!
//! # 错误纪律（报告 §四：事实+示范+补救）
//! ParseError 携带补丁行号（1 起）与合法形式示范，模型可对照自纠。

/// 补丁标记常量（与报告 §1.1 文法一一对应）。
pub(crate) const BEGIN: &str = "*** Begin Patch";
pub(crate) const END: &str = "*** End Patch";
const ADD_PREFIX: &str = "*** Add File: ";
const DELETE_PREFIX: &str = "*** Delete File: ";
const UPDATE_PREFIX: &str = "*** Update File: ";
const MOVE_PREFIX: &str = "*** Move to: ";
const EOF_MARKER: &str = "*** End of File";
const CHUNK_PREFIX: &str = "@@";

/// 解析错误（事实=第几行什么形态；示范=合法头枚举/前缀语义；code=错误码族
/// tool/patch-*，机器路由与人读分离——报告 §四纪律）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseError {
    /// 补丁文本中的行号（1 起）。
    pub line: usize,
    pub code: &'static str,
    pub message: String,
}

/// Update chunk（报告 §1.2 五字段同构；context 索引对区分「上下文行」与
/// 「恰好内容相同的改动行」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateChunk {
    pub change_context: Option<String>,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub context_pairs: Vec<(usize, usize)>,
    pub is_end_of_file: bool,
}

/// 补丁 hunk（三级：新增/删除/更新+可选移动）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Hunk {
    Add { path: String, lines: Vec<String> },
    Delete { path: String },
    Update { path: String, move_to: Option<String>, chunks: Vec<UpdateChunk> },
}

fn err(line: usize, message: impl Into<String>) -> ParseError {
    err_c(line, "tool/patch-syntax", message)
}

fn err_c(line: usize, code: &'static str, message: impl Into<String>) -> ParseError {
    ParseError { line, code, message: message.into() }
}

/// 宽容层②：heredoc 壳剥离——整体被 `<<[-']?EOF` … `EOF` 包裹且 ≥4 行时，
/// 去掉首尾壳行（模型把补丁当 shell 文档误传的形态；研究 §1.3）。
fn strip_heredoc(lines: &[&str]) -> Option<Vec<String>> {
    let first = lines.first()?.trim();
    let last = lines.last()?.trim();
    // 开壳形态：<<EOF / <<'EOF' / <<"EOF"（引号可双侧或省略）
    let quoted = first
        .strip_prefix("<<")
        .map(|body| body.trim_matches(|c| c == '\'' || c == '"'))
        .map(|body| body == "EOF")
        .unwrap_or(false);
    if lines.len() < 4 || !quoted || last != "EOF" {
        return None;
    }
    Some(lines[1..lines.len() - 1].iter().map(|l| l.to_string()).collect())
}

/// 解析补丁全文为 hunk 列表（宽容层：marker 行 trim；heredoc 剥壳）。
pub(crate) fn parse_patch(text: &str) -> Result<Vec<Hunk>, ParseError> {
    let raw: Vec<&str> = text.lines().collect();
    let lines = strip_heredoc(&raw).unwrap_or_else(|| raw.iter().map(|l| l.to_string()).collect());
    let n = lines.len();
    if n == 0 || lines[0].trim() != BEGIN {
        return Err(err(1, format!(
            "首行须为 \"{BEGIN}\"，实收「{}」",
            lines.first().map(|l| l.trim().to_string()).unwrap_or_default()
        )));
    }
    let mut hunks = Vec::new();
    let mut i = 1usize; // 已消费 Begin
    while i < n {
        let trimmed = lines[i].trim();
        if trimmed == END {
            return Ok(hunks);
        }
        if let Some(path) = trimmed.strip_prefix(ADD_PREFIX) {
            let (hunk, next) = parse_add(&lines, i, path.trim());
            hunks.push(hunk);
            i = next;
        } else if let Some(path) = trimmed.strip_prefix(DELETE_PREFIX) {
            hunks.push(Hunk::Delete { path: path.trim().to_string() });
            i += 1;
        } else if let Some(path) = trimmed.strip_prefix(UPDATE_PREFIX) {
            let (hunk, next) = parse_update(&lines, i, path.trim(), n)?;
            hunks.push(hunk);
            i = next;
        } else {
            return Err(err_c(i + 1, "tool/patch-hunk", format!(
                "「{trimmed}」不是合法 hunk 头。合法形式：\"{ADD_PREFIX}<path>\"、\"{DELETE_PREFIX}<path>\"、\"{UPDATE_PREFIX}<path>\""
            )));
        }
    }
    Err(err(n, format!("缺收尾标记 \"{END}\"")))
}

/// Add File：内容行一律 "+" 前缀（空行=空内容行，宽容）。
fn parse_add(lines: &[String], head: usize, path: &str) -> (Hunk, usize) {
    let mut body = Vec::new();
    let mut i = head + 1;
    while i < lines.len() {
        let t = lines[i].trim();
        if t == END || t.starts_with(ADD_PREFIX) || t.starts_with(DELETE_PREFIX)
            || t.starts_with(UPDATE_PREFIX) {
            break;
        }
        match lines[i].strip_prefix('+') {
            Some(content) => body.push(content.to_string()),
            None if lines[i].is_empty() => body.push(String::new()),
            None => body.push(lines[i].to_string()), // 容容错：Add 块容忍无前缀行（模型常见漏 +）
        }
        i += 1;
    }
    (Hunk::Add { path: path.to_string(), lines: body }, i)
}

/// Update File：可选 Move to + 1..n 个 chunk（@@ 锚点/前缀行/EOF 哨兵）。
fn parse_update(
    lines: &[String],
    head: usize,
    path: &str,
    total: usize,
) -> Result<(Hunk, usize), ParseError> {
    let mut i = head + 1;
    let mut move_to = None;
    if let Some(t) = lines.get(i).map(|l| l.trim()) {
        if let Some(dest) = t.strip_prefix(MOVE_PREFIX) {
            move_to = Some(dest.trim().to_string());
            i += 1;
        }
    }
    let mut chunks: Vec<UpdateChunk> = Vec::new();
    let mut current: Option<UpdateChunk> = None;
    while i < total {
        let raw = &lines[i];
        let t = raw.trim();
        if t == END || t.starts_with(ADD_PREFIX) || t.starts_with(DELETE_PREFIX)
            || t.starts_with(UPDATE_PREFIX) {
            break;
        }
        if let Some(rest) = t.strip_prefix(CHUNK_PREFIX) {
            if let Some(chunk) = current.take() {
                if chunk.old_lines.is_empty() && chunk.new_lines.is_empty() {
                    return Err(err_c(i + 1, "tool/patch-empty", format!(
                        "文件 {path} 的更新块为空（@@ 后须至少一行上下文或增删行）"
                    )));
                }
                chunks.push(chunk);
            }
            let anchor = rest.trim();
            current = Some(UpdateChunk {
                change_context: if anchor.is_empty() { None } else { Some(anchor.to_string()) },
                old_lines: Vec::new(),
                new_lines: Vec::new(),
                context_pairs: Vec::new(),
                is_end_of_file: false,
            });
            i += 1;
            continue;
        }
        if t == EOF_MARKER {
            match current.as_mut() {
                Some(chunk) => chunk.is_end_of_file = true,
                None => return Err(err_c(i + 1, "tool/patch-context", format!(
                    "\"{EOF_MARKER}\" 必须出现在某个 @@ 更新块内部"
                ))),
            }
            i += 1;
            continue;
        }
        let chunk = current.as_mut().ok_or_else(|| {
            err_c(i + 1, "tool/patch-context", format!(
                "更新块须以 \"{CHUNK_PREFIX}\" 上下文标记开始，实收「{t}」"
            ))
        })?;
        // 行前缀三态（报告错误形态 #5：解释三种前缀语义）
        match raw.strip_prefix(' ') {
            Some(content) => {
                chunk.context_pairs.push((chunk.old_lines.len(), chunk.new_lines.len()));
                chunk.old_lines.push(content.to_string());
                chunk.new_lines.push(content.to_string());
            }
            _ => match raw.strip_prefix('-') {
                Some(content) => chunk.old_lines.push(content.to_string()),
                _ => match raw.strip_prefix('+') {
                    Some(content) => chunk.new_lines.push(content.to_string()),
                    _ if raw.is_empty() => {
                        chunk.context_pairs.push((chunk.old_lines.len(), chunk.new_lines.len()));
                        chunk.old_lines.push(String::new());
                        chunk.new_lines.push(String::new());
                    }
                    _ => return Err(err_c(i + 1, "tool/patch-line", format!(
                        "更新块内每行须以 ' '（上下文）、'-'（删除）或 '+'（新增）开头，实收「{raw}」"
                    ))),
                },
            },
        }
        i += 1;
    }
    if let Some(chunk) = current.take() {
        if chunk.old_lines.is_empty() && chunk.new_lines.is_empty() {
            return Err(err_c(total, "tool/patch-empty", format!("文件 {path} 的更新块为空")));
        }
        chunks.push(chunk);
    }
    if chunks.is_empty() {
        return Err(err(head + 1, format!("文件 {path} 的更新 hunk 为空（缺 @@ 块）")));
    }
    // 返回断点位置（END/下一 hunk 头），外层循环继续消费——返回 total 会跳过
    // End Patch 检测与后续 hunk（多 hunk/收尾双炸，task-22 会话内已证）。
    Ok((Hunk::Update { path: path.to_string(), move_to, chunks }, i))
}

#[cfg(test)]
#[path = "patch_parser_tests.rs"]
mod tests;
