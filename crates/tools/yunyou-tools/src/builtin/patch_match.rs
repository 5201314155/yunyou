//! 补丁匹配与行尾保真（task-22；算法依据拆解报告 §二/§三——四级递降匹配、
//! 游标+锚点推进、EOF 哨兵重试、行尾三态保真、落盘前预检试算。实现为本仓
//! 自写；「±N 行模糊窗」两参照源一致否定，不收）。

use crate::builtin::patch_parser::UpdateChunk;

/// 行尾三态（报告 §2.3：Lf/CrLf/Cr/无尾——混合行尾文件只动真正变化的行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Cr 变体留位注释见上
pub(crate) enum Ending {
    Lf,
    CrLf,
    Cr,
    None,
}

/// 带行尾的源行。
#[derive(Debug, Clone)]
pub(crate) struct SourceLine {
    pub text: String,
    pub ending: Ending,
}

/// 源文件行级视图（parse 保留每行行尾；preferred_ending=首个出现的行尾样式）。
#[derive(Debug, Clone)]
pub(crate) struct SourceFile {
    pub lines: Vec<SourceLine>,
    pub preferred_ending: Ending,
}

impl SourceFile {
    /// 逐行拆分并保留行尾三态（无独立空行元素——换行挂在前一行）。
    pub(crate) fn parse(bytes: &str) -> Self {
        let mut lines: Vec<SourceLine> = Vec::new();
        let mut preferred: Option<Ending> = None;
        let mut pending = String::new();
        for ch in bytes.chars() {
            match ch {
                '\n' => {
                    let ending = if pending.ends_with('\r') {
                        pending.pop();
                        Ending::CrLf
                    } else {
                        Ending::Lf
                    };
                    lines.push(SourceLine { text: std::mem::take(&mut pending), ending });
                    preferred = preferred.or(Some(ending));
                }
                '\r' => pending.push(ch),
                c => pending.push(c),
            }
        }
        if !pending.is_empty() {
            lines.push(SourceLine { text: pending, ending: Ending::None });
        }
        Self { lines, preferred_ending: preferred.unwrap_or(Ending::Lf) }
    }

    /// 重组文件内容（未动行保留原行尾；插入行用 preferred_ending；无尾行补
    /// preferred——update 后补尾换行为 apply-patch 历史行为，报告 §2.3）。
    pub(crate) fn render(&self, lines: &[SourceLine]) -> String {
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            out.push_str(&line.text);
            match line.ending {
                Ending::Lf => out.push('\n'),
                Ending::CrLf => out.push_str("\r\n"),
                Ending::Cr => out.push('\r'),
                Ending::None => {
                    if i + 1 == lines.len() && self.preferred_ending != Ending::None {
                        match self.preferred_ending {
                            Ending::Lf => out.push('\n'),
                            Ending::CrLf => out.push_str("\r\n"),
                            _ => out.push('\r'),
                        }
                    }
                }
            }
        }
        out
    }
}

/// Unicode 标点归一（报告 §2.1 第 4 级：弯引号→直引号、dash 族→-、特殊
/// 空格→普通空格——git apply 对排版符号差异可落，此前字节级匹配失败）。
fn normalize_punct(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' => '"',
            '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}' | '\u{2212}' | '\u{2010}'
            | '\u{2011}' => '-',
            '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{2009}' | '\u{2002}' | '\u{2003}'
            | '\u{2005}' | '\u{2006}' | '\u{2008}' | '\u{2004}' | '\u{205F}' => ' ',
            other => other,
        })
        .collect()
}

/// 四级判定器数组类型（闭包元数统一——clippy 复杂类型收敛）。
type MatchJudges = [fn(&[SourceLine], &[String], usize) -> bool; 4];

/// 四级递降匹配（报告 §2.1）：先精确后宽松，任一命中即返回；eof 时先按
/// 文件尾对齐试（补丁声明锚死尾），失败回 start 正常搜。
fn seek_sequence(
    lines: &[SourceLine],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }
    // 候选起点：eof 声明=尾对齐位置优先，其后 start 起逐位扫描
    let mut positions: Vec<usize> = Vec::new();
    if eof {
        positions.push(lines.len() - pattern.len());
    }
    positions.extend(start..=lines.len() - pattern.len());
    // 四级递降，级内全位置扫描（先精确后宽松，任一命中即返回——报告 §2.1：
    // 逐位扫而不仅试起始位，否则 pattern 不在 start 处即漏配）
    let judges: MatchJudges = [
        |ls, pt, b| ls[b..b + pt.len()].iter().zip(pt).all(|(l, p)| l.text == *p),
        |ls, pt, b| ls[b..b + pt.len()].iter().zip(pt).all(|(l, p)| l.text.trim_end() == p.trim_end()),
        |ls, pt, b| ls[b..b + pt.len()].iter().zip(pt).all(|(l, p)| l.text.trim() == p.trim()),
        |ls, pt, b| ls[b..b + pt.len()].iter().zip(pt).all(|(l, p)| normalize_punct(&l.text) == normalize_punct(p)),
    ];
    for judge in judges {
        for &base in &positions {
            if judge(lines, pattern, base) {
                return Some(base);
            }
        }
    }
    None
}



/// 行区间替换指令（预检产物；写盘阶段按序应用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replacement {
    pub start: usize,
    pub end: usize,
    pub new_lines: Vec<String>,
}

/// 匹配失败（事实+补救：hunk 序号/补丁行号/pattern 预览——报告 §四 #7）。
#[derive(Debug, Clone)]
pub(crate) struct MatchFailure {
    pub chunk_index: usize,
    pub patch_line: usize,
    pub preview: String,
}

/// 单文件 chunk 试算（报告 §2.2 compute_replacements 同构；纯内存零副作用）：
/// 游标单调推进+change_context 锚点先定位+EOF 哨兵重试（old 尾空串形态）+
/// 上下文行拆段保真（context_pairs 之间的原文行不进替换）。
pub(crate) fn compute_replacements(
    file: &SourceFile,
    chunks: &[UpdateChunk],
) -> Result<Vec<Replacement>, MatchFailure> {
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut line_index = 0usize;
    for (ci, chunk) in chunks.iter().enumerate() {
        if let Some(anchor) = &chunk.change_context {
            if let Some(found) = seek_sequence(
                &file.lines[line_index.min(file.lines.len())..],
                std::slice::from_ref(anchor),
                0,
                false,
            ) {
                line_index += found + 1; // 推进到锚点后
            }
        }
        if chunk.old_lines.is_empty() {
            // 纯新增 chunk：锚定文件尾追加（报告 §2.2 L86-89）
            replacements.push(Replacement {
                start: file.lines.len(),
                end: file.lines.len(),
                new_lines: chunk.new_lines.clone(),
            });
            continue;
        }
        let eof = chunk.is_end_of_file;
        // EOF 哨兵预处理：old 末尾空串=diff 终结换行伪影（SourceFile 把换行挂
        // 前一行，无独立空行元素）——直搜失败时 old/new 同步去尾+pairs 过滤后
        // 重搜（替换跨度=去尾后跨度，防越界——v2 修 pos+len-1 跨度外扩错误）。
        let mut eff_old = &chunk.old_lines[..];
        let mut eff_new = &chunk.new_lines[..];
        let mut eff_pairs = &chunk.context_pairs[..];
        let trimmed = false;
        if chunk.old_lines.last().map(|l| l.is_empty()).unwrap_or(false)
            && seek_sequence(&file.lines, &chunk.old_lines, line_index, eof).is_none()
        {
            eff_old = &chunk.old_lines[..chunk.old_lines.len() - 1];
            eff_new = &chunk.new_lines[..chunk.new_lines.len().saturating_sub(1)];
            eff_pairs = &chunk.context_pairs[..];
            let _ = trimmed; // 跨度换用已由 eff_* 承载；留痕=哨兵路径标记
        }
        let found = seek_sequence(&file.lines, eff_old, line_index, eof);
        let start_idx = found.ok_or_else(|| MatchFailure {
            chunk_index: ci,
            patch_line: 0,
            preview: chunk
                .old_lines
                .first()
                .map(|l| l.chars().take(80).collect())
                .unwrap_or_default(),
        })?;
        let end_idx = start_idx + eff_old.len();
        // 上下文行拆段（F-1 修复：单趟按行序推进，官方 compute_replacements
        // 形态——游标 oi 走 old 侧、ni 走 new 侧，context_pairs 的上下文行把
        // 两侧同步对齐后推进；'-' 行开/延 old 段，new 侧内容在段收口时按
        // [ni, 下一上下文 np) 一并记入。不做「old 偏移切 new 侧」的事后下标
        // 换算——非对称增删天然正确；纯插入段（无相邻 '-' 行）产零宽替换，
        // 不再静默丢行。上下文行原样保留原行尾，混合行尾文件只动真正变化的行）
        let pair_new = |offset: usize| {
            eff_pairs.iter().find(|(o, _)| *o == offset).map(|(_, n)| *n)
        };
        let mut run: Option<(usize, Vec<String>)> = None; // (old 段起, 累计 new 行)
        let mut ni = 0usize; // new 侧游标：随上下文对齐推进，段收口取 [ni, np)
        for (offset, _old_text) in eff_old.iter().enumerate() {
            if let Some(np) = pair_new(offset) {
                // 上下文行：先收口当前段（新增行=两侧上下文之间的 new 区间）
                if let Some((rs, mut newl)) = run.take() {
                    newl.extend_from_slice(&eff_new[ni..np]);
                    replacements.push(Replacement {
                        start: start_idx + rs,
                        end: start_idx + offset,
                        new_lines: newl,
                    });
                } else if ni < np {
                    // 纯插入段（上下文之间只有 '+' 行）：零宽替换
                    replacements.push(Replacement {
                        start: start_idx + offset,
                        end: start_idx + offset,
                        new_lines: eff_new[ni..np].to_vec(),
                    });
                }
                ni = np + 1;
            } else {
                // '-' 行：old 侧删除，开段或延段（new 侧内容随段收口计入）
                run.get_or_insert_with(|| (offset, Vec::new()));
            }
        }
        match run.take() {
            Some((rs, mut newl)) => {
                // 段收口在 old 末尾：新增行=尾上下文之后的 new 区间
                newl.extend_from_slice(&eff_new[ni..]);
                replacements.push(Replacement {
                    start: start_idx + rs,
                    end: start_idx + eff_old.len(),
                    new_lines: newl,
                });
            }
            None if ni < eff_new.len() => {
                // 尾部纯插入段（末上下文之后只有 '+' 行）：零宽替换
                replacements.push(Replacement {
                    start: start_idx + eff_old.len(),
                    end: start_idx + eff_old.len(),
                    new_lines: eff_new[ni..].to_vec(),
                });
            }
            None => {}
        }
        line_index = end_idx;
    }
    Ok(replacements)
}

/// 预检+应用（纯内存）：把 replacements 应用到行视图，产出新文件内容字符串。
/// 未动行保留原行尾；插入行用 preferred_ending（报告 §2.3）。
pub(crate) fn apply_replacements(
    file: &SourceFile,
    replacements: &[Replacement],
) -> String {
    let mut lines: Vec<SourceLine> = file.lines.clone();
    // 倒序应用避免位移（区间互不重叠由游标单调推进保证）
    for r in replacements.iter().rev() {
        let mut new_lines: Vec<SourceLine> = r
            .new_lines
            .iter()
            .map(|text| SourceLine { text: text.clone(), ending: file.preferred_ending })
            .collect();
        // 段末行继承被替换段末行的行尾（保留该位置「有无换行」的事实；文件尾
        // 无尾行继承 None 后由 render 补 preferred——update 后补尾换行）。
        if let (Some(last_new), Some(old_last)) =
            (new_lines.last_mut(), lines.get(r.end.saturating_sub(1)))
        {
            last_new.ending = old_last.ending;
        }
        lines.splice(r.start..r.end, new_lines);
    }
    file.render(&lines)
}

#[cfg(test)]
#[path = "patch_match_tests.rs"]
mod tests;
