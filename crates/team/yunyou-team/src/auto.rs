//! 自动流转层（K-17，lead 批 16:3x 四机制+六执行约束）：空闲领取/自动送审/
//! 解锁通知/完工简报模板——K-8 六态机零改动，四机制只做「谁去扳挡杆+扳完
//! 通知谁」。
//!
//! ## 防过度自动化三闸（lead 批，产品行为关切的定案对答）
//!
//! 1. **默认全关**（enabled=false 逐席逐机制开）；
//! 2. **人工覆盖**（lead reassign/人工 claim 永远可推翻自动结果——既有闸原样）；
//! 3. **硬顶**（max_active_per_member=1 起步，硬上限非软目标）。
//!
//! 「谁领什么」=capability 精确匹配（非模糊推荐）；「领多少」=硬上限。


use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// 纯函数层先行入库（策略评估/通知合成/简报模板全单测锁死）；触发接线=
// member.rs idle 检查 + service.rs submit/review 挂接（B2 尾段同批），
// 届时本模块全部 pub 项被消费，观察期 allow 摘除。

/// 自动领取策略（TeamConfig 可配；默认全关=防过度自动化首闸）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct AutoClaimConfig {
    /// 是否启用自动领取（默认 false——lead 批默认保守）。
    #[serde(default)]
    pub enabled: bool,
    /// 每成员同时持有 claimed 态任务数硬上限（默认 1）。
    #[serde(default = "default_max_active")]
    pub max_active_per_member: u32,
    /// capability 标签（成员与任务精确匹配；空=不筛选全领——防误领建议非空）。
    #[serde(default)]
    pub capability: String,
}

fn default_max_active() -> u32 {
    1
}

impl Default for AutoClaimConfig {
    fn default() -> Self {
        Self { enabled: false, max_active_per_member: default_max_active(), capability: String::new() }
    }
}

/// 领取评估产出（纯函数——板/名册快照入，领取决策出，零副作用可测）。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // 触发接线 B2 尾段（member.rs idle 检查消费）
pub struct ClaimCandidate {
    pub task_id: u64,
    pub member: String,
}

/// 自动领取评估（纯函数）。
///
/// 入参快照语义：pending_tasks=板内 Pending 任务 (id, capability) 升序；
/// member_capabilities=成员名→capability 标签；member_active=成员当前持有
/// claimed 态任务数。产出=按序领取候选（member, task_id）——调用方逐条走
/// board.claim 既有闸（非 pending/已领拒原样=SelfReview 同效纪律②）。
///
/// 筛选规则（三闸联合）：
/// 1. enabled=false → 空产出（默认关态零动作，约束③）；
/// 2. capability 非空 → 任务与成员 capability 精确匹配（无匹配=静默不动，
///    约束⑤禁广播）；
/// 3. 成员持有 claimed 数 ≥ max_active_per_member → 该成员本轮不再领（硬顶）。
#[allow(dead_code)] // 同上：B2 尾段接线消费
pub fn evaluate_claims(
    config: &AutoClaimConfig,
    pending_tasks: &[(u64, String)],
    member_capabilities: &[(String, String)],
    member_active_counts: &[(String, u32)],
) -> Vec<ClaimCandidate> {
    if !config.enabled {
        return Vec::new();
    }
    let active = |member: &str| -> u32 {
        member_active_counts
            .iter()
            .find(|(name, _)| name == member)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    };
    // capability 匹配（三层过滤）：任务 capability == 成员 capability（谁会做
    // 什么活）；config.capability 非空=追加全局筛选（只领此类型任务——首版
    // 保守建议非空）。成员无 capability 标签=不匹配（无标签成员不参与自动
    // 领取，约束⑤静默）。
    let matches_capability =
        |task_cap: &str, member_cap: &str, global_filter: &str| -> bool {
            if task_cap != member_cap {
                return false;
            }
            global_filter.is_empty() || task_cap == global_filter
        };
    let mut out = Vec::new();
    for (member, member_cap) in member_capabilities {
        if active(member) >= config.max_active_per_member {
            continue;
        }
        for (task_id, task_cap) in pending_tasks {
            if !matches_capability(task_cap, member_cap, &config.capability) {
                continue;
            }
            if out.iter().any(|c: &ClaimCandidate| c.task_id == *task_id) {
                continue; // 同任务不重复分派
            }
            out.push(ClaimCandidate { task_id: *task_id, member: member.clone() });
            break; // 一成员一轮至多领一任务（硬顶内步进）
        }
    }
    out
}

/// 完工简报模板（④：结论/涉及路径/下一步三段+pathspec 预核清单段）——
/// 纯函数，成员 send_message 时调用产出标准化 Report body。
/// 冻结提醒保持流程级不入模板（lead 批：防过度自动化）。
#[allow(dead_code)] // 同上：member.rs 完工 Report 组装消费
pub fn compose_report(summary: &str, pathspec: &[String], next_step: Option<&str>) -> String {
    let mut report = String::new();
    report.push_str("【完工简报】\n");
    report.push_str(&format!("结论：{summary}\n"));
    if !pathspec.is_empty() {
        report.push_str("涉及路径（pathspec 预核清单）：\n");
        for path in pathspec {
            report.push_str(&format!("  - {path}\n"));
        }
    } else {
        report.push_str("涉及路径：无文件改动\n");
    }
    if let Some(next) = next_step {
        report.push_str(&format!("下一步：{next}\n"));
    }
    report
}

/// 自动送审通知正文（②：标题+完工摘要；不含凭证与敏感面——约束⑤）。
#[allow(dead_code)] // 同上：service.rs task_submit 通知消费
pub fn review_notify_body(task_id: u64, title: &str, summary: &str) -> String {
    format!("任务 #{task_id}「{title}」已提交送审。\n完工摘要：{summary}")
}

/// 自动解锁通知正文（③：前置已通过，发给 lead 的看板面）。
#[allow(dead_code)] // 同上：service.rs task_review 通知消费
pub fn unlock_notify_body(task_id: u64, title: &str) -> String {
    format!("前置任务 #{task_id}「{title}」已通过审查，你的任务已解锁可推进。")
}

/// 自动解锁通知正文·owner 面（③）：指名被解锁任务，owner 一眼知道哪张卡能动。
pub fn unblocked_notify_body(prereq_id: u64, prereq_title: &str, task_id: u64) -> String {
    format!(
        "前置任务 #{prereq_id}「{prereq_title}」已通过审查，你的任务 #{task_id} 已解锁可推进。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 约束③：默认关态零动作（enabled=false→空产出）。
    #[test]
    fn 默认关态零动作() {
        let config = AutoClaimConfig::default();
        assert!(!config.enabled, "默认 off");
        let out = evaluate_claims(
            &config,
            &[(1, "review".into())],
            &[("alice".into(), "review".into())],
            &[("alice".into(), 0)],
        );
        assert!(out.is_empty(), "默认关态必须零动作");
    }

    /// 启用+capability 匹配→领取候选产出；无匹配=静默不动（约束⑤）。
    #[test]
    fn 启用后匹配领取() {
        let config = AutoClaimConfig {
            enabled: true,
            max_active_per_member: 1,
            capability: "review".into(),
        };
        // 成员 review 标签匹配任务 review
        let out = evaluate_claims(
            &config,
            &[(1, "review".into()), (2, "coding".into())],
            &[("alice".into(), "review".into())],
            &[("alice".into(), 0)],
        );
        assert_eq!(out.len(), 1, "只领匹配的 #1（#2 coding 不匹配静默跳过=约束⑤）");
        assert_eq!(out[0].task_id, 1);
        assert_eq!(out[0].member, "alice");
    }

    /// 硬顶：成员持有数达 max_active→不再领（约束③）。
    #[test]
    fn 硬顶内步进() {
        let config = AutoClaimConfig {
            enabled: true,
            max_active_per_member: 1,
            capability: String::new(),
        };
        // alice 已持有 1 个 claimed=达硬顶→本轮不领
        let out = evaluate_claims(
            &config,
            &[(1, "review".into())],
            &[("alice".into(), "review".into())],
            &[("alice".into(), 1)],
        );
        assert!(out.is_empty(), "达硬顶不再领");
        // 换 bob（0 持有）→领
        let out = evaluate_claims(
            &config,
            &[(1, "review".into())],
            &[("bob".into(), "review".into())],
            &[("bob".into(), 0)],
        );
        assert_eq!(out.len(), 1);
    }

    /// 无匹配 capability=静默不动（禁广播，约束⑤）。
    #[test]
    fn 无匹配静默() {
        let config = AutoClaimConfig {
            enabled: true,
            max_active_per_member: 1,
            capability: "coding".into(),
        };
        let out = evaluate_claims(
            &config,
            &[(1, "review".into())],
            &[("alice".into(), "review".into())],
            &[("alice".into(), 0)],
        );
        assert!(out.is_empty(), "capability 不匹配=静默不动");
    }

    /// 完工简报模板：三段+pathspec 清单段（④系统化）。
    #[test]
    fn 简报模板三段() {
        let report = compose_report(
            "K-8 状态机入库",
            &["crates/team/yunyou-team/src/board.rs".to_string()],
            Some("候云点复核"),
        );
        assert!(report.contains("【完工简报】"));
        assert!(report.contains("结论：K-8 状态机入库"));
        assert!(report.contains("涉及路径（pathspec 预核清单）："));
        assert!(report.contains("  - crates/team/yunyou-team/src/board.rs"));
        assert!(report.contains("下一步：候云点复核"));
        // 空 pathspec=无文件改动诚实标注
        let empty = compose_report("摘要", &[], None);
        assert!(empty.contains("涉及路径：无文件改动"));
        // 无下一步=不产该段
        assert!(!empty.contains("下一步："));
    }

    /// 通知正文：标题+完工摘要（不含凭证与敏感面=约束⑤净面）。
    #[test]
    fn 通知正文净面() {
        let body = review_notify_body(7, "评审接口文档", "三条意见已交");
        assert!(body.contains("#7") && body.contains("评审接口文档") && body.contains("三条意见已交"));
        let unlock = unlock_notify_body(7, "评审接口文档");
        assert!(unlock.contains("已通过审查") && unlock.contains("解锁"));
    }
}
