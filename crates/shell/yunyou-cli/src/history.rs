//! 输入历史导航（面板交互升级票①，业主令「方向键从二期提到一期」）。
//!
//! ↑↓ 循环会话内提交历史（shell 标配）：↑ 从草稿位进最后一条、再 ↑ 逐条上溯、
//! ↓ 回退、到草稿位恢复上行的未提交草稿；会话内缓冲（重启持久化留二期——
//! 历史落盘涉会话文件词汇扩展，一期不做）。线程安全：REPL 循环持有一份，
//! Editor 每轮重建时共享同一实例（Arc）。
//!
//! 纯逻辑无 I/O：导航/提交/草稿暂存全部可离线单测（票①测试用例落此）。

use std::sync::Mutex;

/// 会话内输入历史（跨 Editor 重建共享）。
pub struct LineHistory {
    inner: Mutex<HistoryState>,
}

struct HistoryState {
    entries: Vec<String>,
    /// 浏览游标：`entries.len()` = 草稿位（不在历史里——↑ 从草稿进最后一条）。
    cursor: usize,
    /// 草稿暂存：从草稿位 ↑ 上行时存下当前未提交输入，↓ 回草稿位时恢复。
    draft: Option<String>,
}

impl Default for LineHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl LineHistory {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HistoryState { entries: Vec::new(), cursor: 0, draft: None }) }
    }

    /// ↑：光标上移一格返回该条历史；草稿位上行时暂存当前输入为草稿；
    /// 已在最早一条（首条）再 ↑ 原地不动（仍返回首条——shell 惯例：停在最早）。
    /// 空历史返回 None（无历史可导航，输入行保持原样）。
    pub fn up(&self, draft: &str) -> Option<String> {
        let mut state = self.inner.lock().expect("历史锁");
        if state.entries.is_empty() {
            return None;
        }
        if state.cursor == state.entries.len() {
            // 从草稿位上行：暂存草稿（↓ 回来时恢复），进最后一条
            state.draft = Some(draft.to_string());
        }
        if state.cursor > 0 {
            state.cursor -= 1;
        }
        Some(state.entries[state.cursor].clone())
    }

    /// ↓：光标下移返回该条；回到草稿位时返回暂存草稿（无暂存=留在原条，
    /// 返回 None 让调用方保持现状——用户没编辑过就无草稿可回）。
    pub fn down(&self) -> Option<String> {
        let mut state = self.inner.lock().expect("历史锁");
        if state.cursor >= state.entries.len() {
            return None; // 已在草稿位，再 ↓ 无处可去
        }
        state.cursor += 1;
        if state.cursor == state.entries.len() {
            state.draft.take()
        } else {
            Some(state.entries[state.cursor].clone())
        }
    }

    /// 按内部下标取条目（Ctrl-R Enter 接受命中）。
    pub fn entry_at(&self, idx: usize) -> Option<String> {
        self.inner.lock().expect("历史锁").entries.get(idx).cloned()
    }

    /// Ctrl-R 反查：从 `before`（内部下标，不含）向更旧找第一条含 query 的
    /// 条目，返回 (下标, 文本)。query 空串视为匹配任意（=纯 ↑ 上溯等价）。
    pub fn search_before(&self, query: &str, before: usize) -> Option<(usize, String)> {
        let state = self.inner.lock().expect("历史锁");
        let end = before.min(state.entries.len());
        (0..end)
            .rev()
            .find(|&idx| query.is_empty() || state.entries[idx].contains(query))
            .map(|idx| (idx, state.entries[idx].clone()))
    }

    /// 提交一行（回车）：非空且与最后一条不同才入史（连按回车/重复命令不灌史）；
    /// 游标回草稿位、草稿清空（新提交即新草稿起点）。
    pub fn commit(&self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        let mut state = self.inner.lock().expect("历史锁");
        if state.entries.last().map(String::as_str) != Some(trimmed) {
            state.entries.push(trimmed.to_string());
        }
        state.cursor = state.entries.len();
        state.draft = None;
    }

    /// 历史条数（观测/测试用）。
    pub fn len(&self) -> usize {
        self.inner.lock().expect("历史锁").entries.len()
    }

    /// 是否空（测试用）。
    pub fn is_empty(&self) -> bool {
        self.inner.lock().expect("历史锁").entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 票①用例：方向键历史循环——提交三行后 ↑↑↑ 逐条上溯、首条再 ↑ 停住、
    /// ↓↓ 回退、回草稿位恢复草稿；草稿暂存语义（编辑中 ↑ 后 ↓ 回到编辑内容）。
    #[test]
    fn 历史上溯下回与草稿暂存() {
        let history = LineHistory::new();
        assert!(history.up("当前草稿").is_none(), "空历史无可导航");
        history.commit("/help");
        history.commit("/new");
        history.commit("/quit");
        assert_eq!(history.len(), 3);
        // ↑ 三次：quit → new → help
        assert_eq!(history.up("草稿A").as_deref(), Some("/quit"));
        assert_eq!(history.up("").as_deref(), Some("/new"));
        assert_eq!(history.up("").as_deref(), Some("/help"));
        // 首条再 ↑：停在最早一条（shell 惯例，不循环）
        assert_eq!(history.up("").as_deref(), Some("/help"));
        // ↓ 两次：new → quit
        assert_eq!(history.down().as_deref(), Some("/new"));
        assert_eq!(history.down().as_deref(), Some("/quit"));
        // ↓ 回草稿位：恢复上行时暂存的草稿
        assert_eq!(history.down().as_deref(), Some("草稿A"), "草稿应被恢复");
        // 草稿位再 ↓：无处可去
        assert_eq!(history.down(), None);
        // 草稿位上行时未编辑（空草稿）→ 回草稿位无暂存 → None
        assert_eq!(history.up("编辑中").as_deref(), Some("/quit"));
        assert_eq!(history.up("").as_deref(), Some("/new"), "再 ↑ 应上溯到倒数第二条");
        assert_eq!(history.down().as_deref(), Some("/quit"));
        assert_eq!(history.down().as_deref(), Some("编辑中"), "草稿应被恢复");
    }

    /// 提交去重与空行不入史；提交后游标回草稿位（↑ 从最新一条开始）。
    #[test]
    fn 提交去重与游标回位() {
        let history = LineHistory::new();
        history.commit("");
        history.commit("   ");
        assert!(history.is_empty(), "空行不入史");
        history.commit("/help");
        history.commit(" /help "); // trim 后与最后一条相同 → 不入史
        assert_eq!(history.len(), 1, "重复提交不灌史");
        history.commit("/new");
        assert_eq!(history.len(), 2);
        // 提交后游标在草稿位：↑ 进最新提交
        assert_eq!(history.up("").as_deref(), Some("/new"));
        // 编辑中切走再提交新行：草稿清空、游标回位
        assert_eq!(history.up("半截编辑").as_deref(), Some("/help"));
        history.commit("/quit");
        assert_eq!(history.down(), None, "提交后应回草稿位");
        assert_eq!(history.up("").as_deref(), Some("/quit"));
    }
}
