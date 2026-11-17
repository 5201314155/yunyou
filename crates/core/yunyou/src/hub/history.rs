//! 留痕环形缓冲（融合终版合并 d：容量定案 1000，丢最旧）。
//!
//! 容量常量在实现不进 config（冻结面 #10：机制语义进契约=「有界环形丢最旧」，
//! 调容量的触发=首个需调容量的部署出现，届时再议）。完整历史=各成员会话 JSONL，
//! 属会话域（云伍 §3.3 定案边界），不归 hub。

use super::types::{HistoryEntry, HistoryQuery};
use std::collections::VecDeque;

/// 环形容量（丢最旧的「最旧」=log_id 最小者，全局单调递增保证队头即最旧）。
const HISTORY_CAPACITY: usize = 1000;

/// 留痕环：受理同点落账（send 锁内 push），查询只读投影。
pub(super) struct HistoryRing {
    entries: VecDeque<HistoryEntry>,
}

impl HistoryRing {
    pub(super) fn new() -> Self {
        Self { entries: VecDeque::with_capacity(HISTORY_CAPACITY) }
    }

    /// 落账：满则丢最旧再入队（容量上限恒成立）。
    pub(super) fn push(&mut self, entry: HistoryEntry) {
        if self.entries.len() == HISTORY_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// 查询：最近 limit 条，log_id 升序返回（limit=0 得空；超存量得全量）。
    /// 返回升序而非倒序：消费方（/team 回看、面板）按时间正读是主流形态，
    /// 需要倒序者自行 rev——查询面不做两种排序的复用预造。
    pub(super) fn query(&self, query: &HistoryQuery) -> Vec<HistoryEntry> {
        let start = self.entries.len().saturating_sub(query.limit);
        self.entries.iter().skip(start).cloned().collect()
    }
}
