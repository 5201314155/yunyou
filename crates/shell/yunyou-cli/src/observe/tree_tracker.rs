//! 树归集器（任务 30-④，自 observe.rs 按职责拆出，行数清偿批 task-22；
//! 纯搬移零行为变更）：按 parent 关联计算子调用深度与收尾符号。

use std::collections::HashMap;

/// 树归集器（任务 30-④）：按 parent 关联计算子调用深度与收尾符号。
#[derive(Default)]
pub(super) struct TreeTracker {
    /// call_id → (parent_tool_call_id, depth)；depth(parent)+1，根为 0。
    nodes: HashMap<String, (Option<String>, usize)>,
    /// parent call_id → 在途子调用数（is_last 判定依据）。
    pending: HashMap<String, usize>,
}

impl TreeTracker {
    /// ToolCall 到达：登记归属；有父则深度 +1 且父的在途子数 +1。
    pub(super) fn on_call(&mut self, call_id: &str, parent: Option<&str>) {
        let depth = match parent {
            Some(pid) => {
                let parent_depth = self.nodes.get(pid).map(|(_, depth)| *depth).unwrap_or(0);
                *self.pending.entry(pid.to_string()).or_default() += 1;
                parent_depth + 1
            }
            None => 0,
        };
        self.nodes.insert(call_id.to_string(), (parent.map(str::to_string), depth));
    }

    /// ToolResult 到达：返回 (depth, is_last)；有父则父的在途子数 -1，
    /// 减后为 0 即当前最后一个完成者（└─），否则 ├─。
    ///
    /// **防重**：用 `remove` 消费节点——同一 call_id 的重复 result 事件
    /// （防御 A 线时序异常）不再二次扣减 pending 污染 is_last，顺带收敛表。
    pub(super) fn on_result(&mut self, call_id: &str) -> (usize, bool) {
        let Some((parent, depth)) = self.nodes.remove(call_id) else {
            return (0, true); // 未知/已消费的 call（防御）：平铺
        };
        let is_last = match &parent {
            Some(pid) => {
                if let Some(count) = self.pending.get_mut(pid) {
                    *count = count.saturating_sub(1);
                    *count == 0
                } else {
                    true
                }
            }
            None => true,
        };
        (depth, is_last)
    }

    /// 回合收尾：清空（call_id 不跨轮残留）。
    pub(super) fn clear(&mut self) {
        self.nodes.clear();
        self.pending.clear();
    }
}
