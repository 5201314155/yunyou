//! 团队域数据类型与服务契约（K-17 收尾拆分②：从 service.rs 迁出）。
//!
//! 职责：数据类型（ContextMode/TeammateSpec/TeammateSpawned/TeamConfig）+
//! TeamService trait——service.rs 保留 TeamCore/实现体/register 工具。

/// fork/fresh 一期语义（裁②）：fresh=空白上下文；fork 一期降级=注入 lead 对话
/// 摘要为人设段文本（真 fork 随二期会话投影，换实现不换参数面）。
use crate::board::TeamTaskView;
use crate::roster::RosterEntry;
use yunyou::hub::{Address, DeliveredMessage, MessageKind, SentReceipt};
use serde::{Deserialize, Serialize};
use yunyou::Result;
use yunyou_util::BoxFut;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContextMode {
    /// 空白上下文（默认；Default 派生取首变体）。
    #[default]
    Fresh,
    /// 注入 lead 对话摘要（一期降级形态）。
    Fork,
}

/// spawn 规格（S4 规格参数面：name/description/prompt/context）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateSpec {
    /// 成员名（全局唯一；重名任何态 fail-loud=HubDuplicateMember）。
    pub name: String,
    /// 职责一句话（名册可见）。
    pub description: String,
    /// 成员人设与任务指令（persona 槽注入）。
    pub prompt: String,
    /// 上下文形态（默认 fresh）。
    #[serde(default)]
    pub context: ContextMode,
}

/// spawn 产出（名册摘要，工具面返回值）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateSpawned {
    /// 成员名。
    pub name: String,
    /// 职责描述。
    pub description: String,
    /// 上线提示（信箱已预注册，补投就绪）。
    pub note: String,
}

/// 任务板写操作结构化错误（挂账清理批）：web 端 details 字段化直透
/// （from/op/owner/actor/submitter），Display=板层人话文案不变（兼容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamOpError {
    /// 板层违例（状态机/owner/自审/任务号）。
    Board(crate::board::BoardOpError),
    /// actor/owner 不在名册（ensure_actor 失败）。
    UnknownActor(String),
}

impl std::fmt::Display for TeamOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Board(err) => write!(f, "{err}"),
            Self::UnknownActor(name) => write!(f, "\"{name}\" 不在名册：任务板操作者必须是 lead 或在册成员"),
        }
    }
}

/// 团队配置。
///
/// **段共享设计（⑤改判落地，lead 2026-11-16 批）**：bundled 条目 segment=
/// `"session"`——本卡与 session 卡读同一顶层段，`path` 与 `[session].path`
/// 同 key 同默认值（装配期 assemble.rs 已把缺省补成 `<home>/sessions/
/// default.jsonl` 绝对路径，本卡经 merge_segment 拿到同一最终值）→ 成员会话
/// 目录单源=config 而非 store 实例（SessionStore trait 无路径口是 task-36
/// 定案，storage trait 不泄漏路径语义）。`[plugins.yunyou-team.config]`
/// 浅合并叠加在段值之上（max_concurrency 等团队私有键不进 session 卡视图）。
/// 执行体模式（K-11 双模式切换；缺省 Fiber=现态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WorkerMode {
    /// 进程内 fiber（现态；K-8 六态机/hub 直投）。
    #[default]
    Fiber,
    /// 进程外 worker（K-11 新链：launcher spawn+procmgr 登记+DeathEvent 消费）。
    Process,
}

// [救场修复 yun-mu 2026-11-18 22:2x，留痕] 上方 WorkerMode 块原被插在
// TeamConfig 的 derive 与 struct 之间（语法断裂=#[serde] 属性悬空编译红
// 约 1h，挡全队 cli 链路验证）。本修复仅把 enum 块上移出 derive/struct 之
// 间，零语义变更；请 team 域承办席复核并自接续在途件。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TeamConfig {
    /// 会话文件路径（与 [session].path 同 key 同缺省；成员会话派生其父目录）。
    #[serde(default = "default_session_path")]
    pub path: String,
    /// 成员回合并发上限（信号量；超出者排队——单机进程内资源闸，裁③）。
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    /// 执行体模式（K-11 双模式：fiber=进程内现态/process=进程外 worker 新链；
    /// 缺省 fiber=向后兼容，切换点唯一=装配处，六态机零改）。
    #[serde(default)]
    pub worker_mode: WorkerMode,
    /// 自动领取策略（K-17；默认 off=防过度自动化首闸）。
    #[serde(default)]
    pub auto_claim: crate::auto::AutoClaimConfig,
    /// 自动续跑上限（K-16②；默认 2=超限升级不自动续）。
    #[serde(default = "default_resume_max")]
    pub resume_max: u32,
    /// 中断上报 lead 开关（K-16③；默认 true）。
    #[serde(default = "default_resume_notify")]
    pub resume_notify: bool,
    /// 成员工具单次调用超时秒数（0=不限时；对齐 [agent_loop].tool_timeout_secs
    /// 语义与默认 300——云垒 P2-8：成员工具零超时×并发闸=挂死成员耗尽 4 permit
    /// 全队停摆，超时熔断给其余成员解围；task-30③）。
    #[serde(default = "default_tool_timeout_secs")]
    pub tool_timeout_secs: u64,
}

fn default_tool_timeout_secs() -> u64 {
    300
}

fn default_resume_max() -> u32 {
    2
}

fn default_resume_notify() -> bool {
    true
}

fn default_max_concurrency() -> u32 {
    4
}

fn default_session_path() -> String {
    // 与 yunyou-session SessionConfig::default_path 同值（config 单源纪律）
    "sessions/default.jsonl".into()
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            path: default_session_path(),
            max_concurrency: default_max_concurrency(),
            worker_mode: WorkerMode::default(), // 缺省 Fiber=现态（K-11 向后兼容）
            auto_claim: crate::auto::AutoClaimConfig::default(),
            resume_max: default_resume_max(),
            resume_notify: default_resume_notify(),
            tool_timeout_secs: default_tool_timeout_secs(),
        }
    }
}

/// 团队插件服务契约（工具面唯一入口；object-safe）。
pub trait TeamService: Send + Sync + 'static {
    /// 拉起成员（五点规格①AI 自拉人）：open_mailbox 预注册 spawn 窗口→建成员
    /// fiber（挂团队 fiber，禁用全收）→成员 setup 内 register 认领信箱。
    fn spawn_teammate(&self, spec: TeammateSpec) -> BoxFut<'static, Result<TeammateSpawned>>;

    /// lead 身份发消息（寻址/缓冲/留痕全在 hub；身份=团队 fiber，不可自报）。
    fn send_message(&self, to: Address, kind: MessageKind, body: String) -> Result<SentReceipt>;

    /// 任务板操作（五点规格④；写审分离状态机见 board.rs）。
    fn task_create(&self, title: String, description: String) -> Result<u64>;
    /// 带_capability/阻塞链创建（自动领取与解锁通知的字段源）。
    fn task_create_full(
        &self,
        title: String,
        description: String,
        capability: String,
        blocked_by: Vec<u64>,
    ) -> Result<u64>;
    /// 提交成果（claimed→submitted，仅 owner；记录提交人=自审拒绝锚点）。
    fn task_submit(&self, task_id: u64, actor: &str) -> Result<(), crate::task_types::TeamOpError>;
    /// 审查裁决（submitted→approved/rejected；审查人=提交人时系统拒绝——K-8）。
    fn task_review(&self, task_id: u64, actor: &str, approve: bool) -> Result<(), crate::task_types::TeamOpError>;
    fn task_reassign(&self, task_id: u64, new_owner: &str) -> Result<(), crate::task_types::TeamOpError>;

    /// 踢人（主 agent 管理/踢人）：dispose 成员 fiber（hub 条目降回 Pending，
    /// 信箱保留）；名册条目同步清簿记。
    fn kick(&self, name: &str) -> BoxFut<'static, Result<()>>;

    /// lead 侧信箱消费缝（task-53 P1）：取净 lead 信箱（身份=团队 fiber，与本
    /// 服务 send_message 同一身份源）。成员完工 Report 等经 hub 受理后落此信箱，
    /// REPL 回合起点经本缝取净注入对话——「hub 受理≠对话看见」的非对称缝补点。
    /// 返回按全局 log_id 升序（投递序）。
    fn drain_lead_inbox(&self) -> Result<Vec<DeliveredMessage>>;

    /// 任务板列表（只读投影）。
    fn task_list(&self) -> Vec<TeamTaskView>;

    /// 任务板写操作四方法返回（挂账清理批，lead 2026-11-19 批）：结构化错误
    /// 直透（web details 字段化 from/op/owner/actor/submitter；Display 人话=板层
    /// 文案不变兼容）；UnknownActor=ensure_actor 失败（不在名册）。
    fn task_claim(&self, task_id: u64, actor: &str) -> Result<(), crate::task_types::TeamOpError>;

    /// Web 全量投影口（web.v1 tasks/list，lead 2026-11-19 批）：板内全字段
    /// 只读快照（createdBy/submittedBy/reviewer/capability/blockedBy——看板
    /// 渲染状态与自审锚点所需；薄版 task_list 保留供工具面，两版并存）。
    fn task_list_full(&self) -> Vec<crate::board::TeamTask>;

    /// 名册观测（五点规格⑤）：本表元数据+hub members() 在线标志合成。
    fn roster(&self) -> Vec<RosterEntry>;
}

