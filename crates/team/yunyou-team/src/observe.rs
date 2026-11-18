//! 团队观测纯函数（task-21，lead 批停摆检测 v1 口径）：纯既有观测面推算，
//! 零 fiber 内部侵入（不读回合循环状态，只看 roster 投影+hub 可见活动）。
//!
//! # 停摆口径（lead 批，与 yun-bo K-11 进程模式 DeathEvent 两层并存）
//! busy=true 且「最近一次 hub 可见活动」早于 `stall_after_secs` → 停摆。
//! 长思考推理模型可能超阈——检测显示为 ⚠ 提示非错误（人判断，阈值可配）。

use crate::roster::RosterEntry;
use std::collections::HashMap;

/// 停摆判定纯函数：返回停摆成员名清单（BTreeMap 序稳定）。
///
/// * `entries`：roster 投影（含 busy/busy_since）。
/// * `last_activity_ms`：各成员最近一次 hub 可见活动时刻（from/to 命中，
///   unix 毫秒；无记录=缺项）。
/// * `now_ms`：当前时刻（unix 毫秒；注入时钟=测试锚）。
/// * `stall_after_secs`：静默阈值。
pub fn stalled_members(
    entries: &[RosterEntry],
    last_activity_ms: &HashMap<String, u64>,
    now_ms: u64,
    stall_after_secs: u64,
) -> Vec<String> {
    entries
        .iter()
        .filter(|e| e.busy)
        .filter(|e| {
            // 活动基准=hub 最近活动；无 hub 记录退化用 busy_since（本轮开始即
            // 无任何可见活动）。两者皆无（理论不可达：busy_since 恒有）=不停摆
            // 诚实不猜。
            let last = last_activity_ms
                .get(&e.name)
                .copied()
                .or(e.busy_since.map(|s| s * 1000));
            match last {
                Some(last_ms) => now_ms.saturating_sub(last_ms) > stall_after_secs * 1000,
                None => false,
            }
        })
        .map(|e| e.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, busy: bool, busy_since: Option<u64>) -> RosterEntry {
        RosterEntry {
            name: name.into(),
            description: "测试成员".into(),
            online: true,
            busy,
            busy_since,
            task: None,
        }
    }

    /// 停摆三态：busy+静默超阈=停摆；有活动不误报；busy=false 恒非停摆。
    #[test]
    fn 停摆判定三态() {
        let now = 10_000_000u64;
        let entries = vec![
            entry("stalled", true, Some(now / 1000 - 600)),
            entry("active", true, Some(now / 1000 - 600)),
            entry("idle", false, None),
        ];
        let mut activity = HashMap::new();
        activity.insert("stalled".to_string(), (now - 600_000)); // 10 分钟前
        activity.insert("active".to_string(), now - 1_000); // 刚刚
        let out = stalled_members(&entries, &activity, now, 300);
        assert_eq!(out, vec!["stalled".to_string()], "仅静默超阈的在忙者停摆");
        // busy=false 恒非停摆（idle 即使无活动）
        assert!(!out.contains(&"idle".to_string()));
    }

    /// 无 hub 记录退化 busy_since 口径；阈值边界（恰等阈不停摆）。
    #[test]
    fn 退化口径与阈值边界() {
        let now = 10_000_000u64;
        let entries = vec![
            entry("no-record", true, Some(now / 1000 - 301)),
            entry("edge", true, Some(now / 1000 - 300)),
        ];
        let out = stalled_members(&entries, &HashMap::new(), now, 300);
        assert_eq!(out, vec!["no-record".to_string()], "恰等阈（300s）不停摆");
    }

    /// 空输入=空输出（零团队/零在忙）。
    #[test]
    fn 空输入空输出() {
        assert!(stalled_members(&[], &HashMap::new(), 0, 300).is_empty());
    }
}
