//! 共享任务板（五点规格④）：写审分离状态机（K-8，lead 三补②「纸面家规变主板
//! 齿轮」）。
//!
//! 状态机：`pending → claimed → submitted → reviewing → approved｜rejected`。
//! **系统级自审拒绝**：审查人=提交人时系统拒绝（BoardOpError::SelfReview）——
//! 「状态流转只由审核方写、拒绝自审自过」在板层强制，不依赖工具层自觉。
//!
//! 一期形态=进程内共享态（蓝图 §7「一期无跨进程存储」；持久化随二期任务池演
//! 进）。`blocked` 态为留位（dsh 任务板同款；首个「挂起等依赖」消费方立项时
//! 再加，防预造）。rejected 为终态：重做=新建任务（重开语义候真实消费方）。
//!
//! owner 规则：claim 落真实成员名；submit 仅 owner（交自己的活）；review 的
//! 审查人≠提交人（系统拒绝）；reassign 仅 lead 或现 owner 且仅限
//! pending/claimed（提交后换人=审查关系失效，改派走重建）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// 任务板条目状态（写审分离六态；blocked 留位注释见模块头）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamTaskState {
    /// 已创建无人认领。
    Pending,
    /// 已认领（owner 落真实成员名）。
    Claimed,
    /// owner 已提交成果，待审。
    Submitted,
    /// 审查人已领取审查（v1 与裁决同点原子通过，态保留供观测与二期拆分）。
    Reviewing,
    /// 审查通过（终态）。
    Approved,
    /// 审查驳回（终态：重做=新建任务）。
    Rejected,
}

/// 任务板条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTask {
    /// 板内自增号。
    pub id: u64,
    /// 标题（一句话任务名）。
    pub title: String,
    /// 详情与验收标准。
    pub description: String,
    /// 当前状态。
    pub state: TeamTaskState,
    /// 当前负责人（None=无人认领）。
    pub owner: Option<String>,
    /// 创建者（lead 或成员名）。
    pub created_by: String,
    /// 提交人（submit 时记录；自审拒绝的锚点）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_by: Option<String>,
    /// 审查人（review 时记录）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    /// capability 标签（自动领取匹配用；空=仅人工派）。
    #[serde(default)]
    pub capability: String,
    /// 阻塞本任务的前置任务号列表（approved 后自动解锁通知）。
    #[serde(default)]
    pub blocked_by: Vec<u64>,
}

/// 任务板只读投影（列表面；不暴露内部簿记字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTaskView {
    /// 板内号。
    pub id: u64,
    /// 标题。
    pub title: String,
    /// 状态。
    pub state: TeamTaskState,
    /// 负责人（可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// 任务板操作违规（工具面转 ToolError::Err；程序化匹配面）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardOpError {
    /// 任务号不存在。
    UnknownTask(u64),
    /// 状态不允许该操作（如完成未认领任务、认领已完成任务）。
    InvalidState { task_id: u64, from: &'static str, op: &'static str },
    /// 操作者与 owner 不符（submit 仅 owner；非本人不可动）。
    NotOwner { task_id: u64, owner: String, actor: String },
    /// **系统级自审拒绝（K-8 最值钱一条）**：审查人=提交人，系统拒绝——
    /// 「拒绝自审自过」在板层强制，任何调用方（含 lead）不可绕过。
    SelfReview { task_id: u64, submitter: String, actor: String },
}

impl fmt::Display for BoardOpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTask(id) => write!(f, "任务 #{id} 不存在"),
            Self::InvalidState { task_id, from, op } => {
                write!(f, "任务 #{task_id} 状态 {from} 不允许 {op}")
            }
            Self::NotOwner { task_id, owner, actor } => {
                write!(f, "任务 #{task_id} 负责人是 {owner}，{actor} 无权操作（非本人不可动）")
            }
            Self::SelfReview { task_id, submitter, actor } => write!(
                f,
                "任务 #{task_id} 写审分离拒绝：提交人 {submitter} 不得自审自过（审查人 {actor} 与提交人同一）"
            ),
        }
    }
}

/// 任务板本体（TeamService 单锁保护；每个操作即一次原子状态迁移）。
pub(crate) struct TaskBoard {
    next_id: u64,
    tasks: BTreeMap<u64, TeamTask>,
}

impl TaskBoard {
    pub(crate) fn new() -> Self {
        Self { next_id: 1, tasks: BTreeMap::new() }
    }

    /// 创建任务（created_by=调用方；lead 经工具创建）。
    pub(crate) fn create(
        &mut self,
        title: String,
        description: String,
        created_by: &str,
        capability: String,
        blocked_by: Vec<u64>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tasks.insert(
            id,
            TeamTask {
                id,
                title,
                description,
                state: TeamTaskState::Pending,
                owner: None,
                created_by: created_by.to_string(),
                submitted_by: None,
                reviewer: None,
                capability,
                blocked_by,
            },
        );
        id
    }

    /// 认领：pending→claimed。已 claimed 不可再抢（owner 落真实成员后改名=篡权）。
    pub(crate) fn claim(&mut self, task_id: u64, owner: &str) -> Result<(), BoardOpError> {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return Err(BoardOpError::UnknownTask(task_id));
        };
        if task.state != TeamTaskState::Pending {
            return Err(BoardOpError::InvalidState {
                task_id,
                from: task.state.state_str(),
                op: "claim",
            });
        }
        task.state = TeamTaskState::Claimed;
        task.owner = Some(owner.to_string());
        Ok(())
    }

    /// 提交成果：claimed→submitted，仅 owner（交自己的活）；记录提交人=
    /// 自审拒绝的锚点。
    pub(crate) fn submit(&mut self, task_id: u64, actor: &str) -> Result<(), BoardOpError> {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return Err(BoardOpError::UnknownTask(task_id));
        };
        if task.state != TeamTaskState::Claimed {
            return Err(BoardOpError::InvalidState {
                task_id,
                from: task.state.state_str(),
                op: "submit",
            });
        }
        if task.owner.as_deref() != Some(actor) {
            return Err(BoardOpError::NotOwner {
                task_id,
                owner: task.owner.clone().unwrap_or_default(),
                actor: actor.to_string(),
            });
        }
        task.state = TeamTaskState::Submitted;
        task.submitted_by = Some(actor.to_string());
        Ok(())
    }

    /// 审查裁决：submitted→reviewing→approved/rejected（同点原子通过）。
    ///
    /// **系统级自审拒绝**：审查人=提交人（含 lead 自审自己的活）一律
    /// SelfReview 拒绝——裁决权在系统不在调用方。审查人记录入条目。
    pub(crate) fn review(
        &mut self,
        task_id: u64,
        actor: &str,
        approve: bool,
    ) -> Result<(), BoardOpError> {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return Err(BoardOpError::UnknownTask(task_id));
        };
        if task.state != TeamTaskState::Submitted {
            return Err(BoardOpError::InvalidState {
                task_id,
                from: task.state.state_str(),
                op: "review",
            });
        }
        let submitter = task.submitted_by.clone().unwrap_or_default();
        if actor == submitter {
            return Err(BoardOpError::SelfReview { task_id, submitter, actor: actor.to_string() });
        }
        task.state = TeamTaskState::Reviewing;
        task.reviewer = Some(actor.to_string());
        task.state = if approve { TeamTaskState::Approved } else { TeamTaskState::Rejected };
        Ok(())
    }

    /// 改派：仅 lead 或现 owner，且仅限 pending/claimed——提交后换人=审查关系
    /// 失效（submitted 及之后不可改派，重建任务走新审查链）。
    pub(crate) fn reassign(
        &mut self,
        task_id: u64,
        new_owner: &str,
        actor: &str,
    ) -> Result<(), BoardOpError> {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return Err(BoardOpError::UnknownTask(task_id));
        };
        if !matches!(task.state, TeamTaskState::Pending | TeamTaskState::Claimed) {
            return Err(BoardOpError::InvalidState {
                task_id,
                from: task.state.state_str(),
                op: "reassign",
            });
        }
        let is_lead = actor == "lead";
        let is_owner = task.owner.as_deref() == Some(actor);
        if !is_lead && !is_owner {
            return Err(BoardOpError::NotOwner {
                task_id,
                owner: task.owner.clone().unwrap_or_default(),
                actor: actor.to_string(),
            });
        }
        task.owner = Some(new_owner.to_string());
        Ok(())
    }

    /// 自动领取候选：Pending 任务中 capability 匹配的 (id, capability) 列表。
    pub(crate) fn pending_matching(&self, capability: &str) -> Vec<(u64, String)> {
        self.tasks
            .values()
            .filter(|t| t.state == TeamTaskState::Pending)
            .filter(|t| capability.is_empty() || t.capability == capability)
            .map(|t| (t.id, t.capability.clone()))
            .collect()
    }

    /// 成员当前持有 claimed 态任务数（自动领取硬顶检查用）。
    pub(crate) fn active_claimed_count(&self, member: &str) -> u32 {
        self.tasks
            .values()
            .filter(|t| t.owner.as_deref() == Some(member) && t.state == TeamTaskState::Claimed)
            .count() as u32
    }

    /// 被指定任务阻塞的任务列表（解锁通知扫描用）。
    ///
    /// Pending 与 Claimed 都算「在等前置」：Claimed 是 owner 已认领、专等解锁
    /// 的形态，恰是解锁通知（T-1）最该点名的收件人——只扫 Pending 会让
    /// 「先认领后等前置」的成员永远收不到「前置已通过」。
    pub(crate) fn tasks_blocked_by(&self, task_id: u64) -> Vec<u64> {
        self.tasks
            .values()
            .filter(|t| {
                t.blocked_by.contains(&task_id)
                    && matches!(t.state, TeamTaskState::Pending | TeamTaskState::Claimed)
            })
            .map(|t| t.id)
            .collect()
    }

    /// 取任务标题（通知正文用）。
    pub(crate) fn task_title(&self, task_id: u64) -> Option<String> {
        self.tasks.get(&task_id).map(|t| t.title.clone())
    }

    /// 取任务 owner（解锁通知收件人解析用；未认领=None 不通知）。
    pub(crate) fn task_owner(&self, task_id: u64) -> Option<String> {
        self.tasks.get(&task_id).and_then(|t| t.owner.clone())
    }

    /// 取任务 (title, description)（自动领取后组装回合输入用）。
    pub(crate) fn get_task_info(&self, task_id: u64) -> Option<(String, String)> {
        self.tasks.get(&task_id).map(|t| (t.title.clone(), t.description.clone()))
    }

    /// 列表投影（板内号升序；输出稳定）。
    pub(crate) fn list(&self) -> Vec<TeamTaskView> {
        self.tasks
            .values()
            .map(|task| TeamTaskView {
                id: task.id,
                title: task.title.clone(),
                state: task.state,
                owner: task.owner.clone(),
            })
            .collect()
    }
}

impl TeamTaskState {
    /// 违规信息用的状态名。
    fn state_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Submitted => "submitted",
            Self::Reviewing => "reviewing",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}
