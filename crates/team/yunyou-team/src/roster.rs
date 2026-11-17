//! 名册（五点规格⑤）：成员元数据（描述/人设/在忙态/认领任务）——「谁在跑/谁
//! 空闲/谁的活」的观测面。在线态不在此存：hub `members()` 是消息侧投影的
//! 唯一真源（Pending/Online 状态机在 hub，融合终版裁③边界），名册在线标志
//! 由 service 查 hub 投影合成，两表不重复记账。
//!
//! 与名册服务边界（融合终版 §1.1）：名册三件套之一（role/capabilities/
//! max_concurrency/cost_quota）归 T2 名册服务——本表就是 T2 名册的最小落点，
//! 消息侧只持「可寻址名字+归属+信箱」（hub），互不越界。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 名册条目（观测面投影；hub members() 的在线标志+本表元数据合成）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    /// 成员名（可寻址名，=hub 在册名）。
    pub name: String,
    /// 职责描述（spawn 时设定）。
    pub description: String,
    /// 是否在线（hub Online：绑定存活 fiber）。
    pub online: bool,
    /// 正在跑回合（成员回合循环置位）。
    pub busy: bool,
    /// 当前认领的任务号（任务板投影）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<u64>,
}

/// 成员元数据（spawn 时登记；同名重拉续投时复用——fiber 死亡不清条目，与 hub
/// 「降级保留」语义对齐）。
#[derive(Debug, Clone)]
pub(crate) struct MemberMeta {
    pub description: String,
    /// 正在跑回合。
    pub busy: bool,
    /// 当前认领任务号。
    pub task: Option<u64>,
}

/// 名册本体（TeamService 单锁保护；BTreeMap 保投影输出稳定）。
pub(crate) struct Roster {
    members: BTreeMap<String, MemberMeta>,
}

impl Roster {
    pub(crate) fn new() -> Self {
        Self { members: BTreeMap::new() }
    }

    /// 登记成员（首次 spawn；同名重拉不覆盖——条目保留原描述）。
    pub(crate) fn register(&mut self, name: &str, description: String) {
        self.members.entry(name.to_string()).or_insert_with(|| MemberMeta {
            description,
            busy: false,
            task: None,
        });
    }

    /// 在册判定（任务板 actor 校验用：owner 必须是真实成员或 lead）。
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.members.contains_key(name)
    }

    /// 在忙态置位（成员回合循环起止时调用）。
    pub(crate) fn set_busy(&mut self, name: &str, busy: bool) {
        if let Some(meta) = self.members.get_mut(name) {
            meta.busy = busy;
        }
    }

    /// 移除成员（kick 时同步清簿记；hub 条目由 fiber dispose 降级）。
    pub(crate) fn remove(&mut self, name: &str) {
        self.members.remove(name);
    }

    /// 全员元数据快照（service 合成 RosterEntry 用）。
    pub(crate) fn snapshot(&self) -> Vec<(String, MemberMeta)> {
        self.members.iter().map(|(name, meta)| (name.clone(), meta.clone())).collect()
    }
}
