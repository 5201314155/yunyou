//! 行级 diff 纯函数（TUI 大改批 2-T2a，lead 授权跨界 2026-11-17；设计稿 件二）。
//!
//! 职责：old/new 两段文本 → 结构化 hunk 列表（上下文行数+删行+增行），供
//! canonical `structuredPatch` 上抛、cli 适配层直通 `DetailKind::Diff`
//! （render_diff 三层渲染消费）。对标 claude code StructuredDiff 数据面
//! （〈挖掘〉§6）；Fallback=退化单 hunk（整段删/整段加），降级总原则。
//!
//! 算法：LCS（O(n·m)，规模上界护栏 1e6 格，超限退 Fallback 单 hunk）——
//! edit 语义=唯一原位替换、patch chunk=局部改写，真实输入行数远低于护栏。

/// 一个 diff 块（形状与 cli `render::event::DiffHunk` 对齐：上下文行数+
/// 删行+增行；path 由调用方按文件归属另记，本模块单文件语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineHunk {
    /// 变更区在旧文的起始行号（1-based；批 5 gutter 行号列数据面——
    /// Fallback 单 hunk=1，整段语义仍成立）。
    pub old_start: usize,
    /// 上下文行数（渲染为 ⋮ 上下文 N 行；与上一变更区共享的未变行）。
    pub context: usize,
    /// 删行（旧文独有，按原序）。
    pub removed: Vec<String>,
    /// 增行（新文独有，按原序）。
    pub added: Vec<String>,
}

/// LCS 护栏：行数积超过此值退 Fallback（防 O(n·m) 内存爆炸；8KB content
/// 上限下 2000×2000 足够宽裕）。
const LCS_CELL_CAP: usize = 1_000_000;

/// 文本 → 行序列（\n 分割；尾空行由 lines() 语义自然丢弃，与 cli 渲染同口径）。
fn lines_of(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// 行级 diff：old/new → hunk 列表。相邻变更区间隔 ≤2 行时合并进同一 hunk
/// （间隔行计 context，capped 2——渲染 ⋮ 提示语义）；间隔 >2 行则各成 hunk，
/// 每个 hunk 以末 2 行共享行作 context（不足取实际行数）。
pub fn diff_hunks(old: &str, new: &str) -> Vec<LineHunk> {
    let a = lines_of(old);
    let b = lines_of(new);
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    if a.iter().eq(b.iter()) {
        return Vec::new(); // 零变更不产 hunk（不虚构）
    }
    // 相同行序列（LCS 回溯出的公共子序列在两侧的下标对）。
    let pairs = if a.len().saturating_mul(b.len()) > LCS_CELL_CAP {
        Vec::new() // 超护栏：无公共对 → 退化为整段删/加单 hunk
    } else {
        lcs_pairs(&a, &b)
    };
    // 按 LCS 对切：对之间夹着的 = 变更区（a 段=删，b 段=加），对本身=共享行。
    let mut hunks: Vec<LineHunk> = Vec::new();
    let mut prev_a = 0usize;
    let mut prev_b = 0usize;
    let mut gap_shared: Vec<usize> = Vec::new(); // 与上一变更区的间隔（a 下标）
    for (ia, ib) in pairs.iter().copied().chain([(a.len(), b.len())]) {
        let del = &a[prev_a..ia];
        let add = &b[prev_b..ib];
        if !del.is_empty() || !add.is_empty() {
            // context=与上一变更区的共享行数（cap 2）；首个 hunk 无上邻=0
            let context = if hunks.is_empty() { 0 } else { gap_shared.len().min(2) };
            hunks.push(LineHunk {
                old_start: prev_a + 1, // 1-based（prev_a=本变更区前已消费的 a 行数）
                context,
                removed: del.iter().map(|s| (*s).to_string()).collect(),
                added: add.iter().map(|s| (*s).to_string()).collect(),
            });
            gap_shared.clear();
        }
        gap_shared.push(ia);
        prev_a = ia + 1;
        prev_b = ib + 1;
    }
    // 收尾哨兵对 (len,len) 产生的 gap_shared 含末尾共享行——已按 context 消费。
    if hunks.is_empty() {
        // 有变更但 hunk 空=护栏退化路径（pairs 全空时上面循环会产出一个
        // 整段 hunk，不会到这）；防御兜底：整段删/加。
        hunks.push(LineHunk {
            old_start: 1,
            context: 0,
            removed: a.iter().map(|s| (*s).to_string()).collect(),
            added: b.iter().map(|s| (*s).to_string()).collect(),
        });
    }
    hunks
}

/// LCS 回溯：返回公共行在 (a,b) 中的下标对（原序）。O(n·m) 表 + 线性回溯。
fn lcs_pairs<'t>(a: &[&'t str], b: &[&'t str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if a[i] == b[j] {
                dp[at(i + 1, j + 1)] + 1
            } else {
                dp[at(i + 1, j)].max(dp[at(i, j + 1)])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 单行替换_单hunk_无上下文() {
        let hunks = diff_hunks("let x = 1;\n", "let x = 2;\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].context, 0);
        assert_eq!(hunks[0].old_start, 1, "首行变更 old_start=1");
        assert_eq!(hunks[0].removed, vec!["let x = 1;"]);
        assert_eq!(hunks[0].added, vec!["let x = 2;"]);
    }

    #[test]
    fn 局部改_共享行作上下文() {
        let old = "fn a() {}\nfn b() {\n    old();\n}\nfn c() {}\n";
        let new = "fn a() {}\nfn b() {\n    new();\n}\nfn c() {}\n";
        let hunks = diff_hunks(old, new);
        assert_eq!(hunks.len(), 1, "上下文邻接应单 hunk：{hunks:?}");
        assert_eq!(hunks[0].context, 0, "首 hunk 无上邻变更区，context=0（hunk 间语义）：{hunks:?}");
        assert_eq!(hunks[0].removed, vec!["    old();"]);
        assert_eq!(hunks[0].added, vec!["    new();"]);
    }

    #[test]
    fn 相距远_两hunk() {
        let old = "a1\na2\nX\nmid1\nmid2\nmid3\nmid4\nmid5\nY\n";
        let new = "a1\na2\nX2\nmid1\nmid2\nmid3\nmid4\nmid5\nY2\n";
        let hunks = diff_hunks(old, new);
        assert_eq!(hunks.len(), 2, "间隔 >2 行应各自成 hunk：{hunks:?}");
        assert_eq!(hunks[0].removed, vec!["X"]);
        assert_eq!(hunks[1].removed, vec!["Y"]);
        assert_eq!(hunks[1].old_start, 9, "第二变更区=旧文第 9 行（批 5 gutter）");
        assert_eq!(hunks[1].context, 2, "hunk 间 6 行共享按 cap 2 计：{hunks:?}");
    }

    #[test]
    fn 纯增_纯删_零变更() {
        assert!(diff_hunks("x\n", "x\n").is_empty(), "零变更不产 hunk");
        let add = diff_hunks("", "新行\n");
        assert_eq!(add.len(), 1);
        assert!(add[0].removed.is_empty() && add[0].added == vec!["新行"]);
        let del = diff_hunks("旧行\n", "");
        assert_eq!(del[0].removed, vec!["旧行"]);
        assert!(del[0].added.is_empty());
    }

    #[test]
    fn 交错改动_LCS保序() {
        let hunks = diff_hunks("keep1\nA\nkeep2\nB\nkeep3\n", "keep1\nA2\nkeep2\nB2\nkeep3\n");
        assert_eq!(hunks.len(), 2, "两处独立替换（间隔 1 行 ≤2 合并不适用——间隔行即共享）：{hunks:?}");
        assert_eq!(hunks[0].added, vec!["A2"]);
        assert_eq!(hunks[1].added, vec!["B2"]);
        assert_eq!(hunks[1].context, 1, "hunk 间 1 行共享如实计：{hunks:?}");
    }
}
