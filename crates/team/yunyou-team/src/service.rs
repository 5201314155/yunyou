//! TeamService：工具面唯一代理（身份、并发治理与名册/任务板的集中点）。
//!
//! 身份模型（防伪造定案在插件面的延续）：lead 侧全部 hub 操作经本服务执行，
//! 身份=本插件 fiber（setup 内 register "lead" 的自发成员）；成员侧发消息走
//! member.rs 的身份化工具（成员自己的 fiber 上下文），不经本服务——两条身份
//! 通道都收在 fiber 上下文里，模型与工具实现都无法自报发送者。

use crate::board::{BoardOpError, TaskBoard, TeamTaskView};
use crate::task_types::{ContextMode, TeamConfig, TeamService, TeammateSpawned, TeammateSpec};
use crate::member::MemberPlugin;
use crate::roster::{Roster, RosterEntry};
use crate::tools;
use yunyou::hub::{Address, DeliveredMessage, MessageKind, MessageHub, OutgoingMessage, SentReceipt};
use yunyou::{Context, CordisError, ErrorKind, FiberHandle, Result, WeakContext};
use yunyou_session::SessionStore;
use yunyou_util::BoxFut;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;


/// 团队共享簿记（service 与成员回合循环共用：busy 置位/成员 fiber 句柄）。
pub(crate) struct TeamCore {
    pub board: Mutex<TaskBoard>,
    pub roster: Mutex<Roster>,
    /// 在册成员 fiber 句柄（kick=dispose；成员离场后句柄留存至显式移除）。
    pub fibers: Mutex<BTreeMap<String, FiberHandle>>,
    /// 成员回合并发闸（裁③ max_concurrency）。
    pub permits: Arc<Semaphore>,
    /// 自动领取策略（K-17；默认 off=防过度自动化首闸）。
    pub auto_claim: crate::auto::AutoClaimConfig,
    /// 任务板解锁唤醒信标（K-18②：approve→notify_waiters 广播空闲成员；
    /// 与 hub 订阅唤醒同构=事件驱动非轮询，Notify permit 合并安全）。
    pub board_wakeup: Arc<tokio::sync::Notify>,
    /// 自动续跑上限（K-16②配置面透传）。
    pub resume_max: u32,
    /// 中断上报开关（K-16③配置面透传）。
    pub resume_notify: bool,
    /// 成员工具单次调用超时秒数（0=不限时；云垒 P2-8 解围闸，task-30③）。
    pub tool_timeout_secs: u64,
}

/// 服务实现（TeamPlugin setup 构造并 provide）。
pub struct TeamServiceState {
    /// 团队插件 fiber 弱句柄：lead 身份与 spawn 挂靠点（强句柄纪律——setup 落定
    /// 后只许 Weak；团队禁用=upgrade 失败=fail-loud）。
    ctx: WeakContext,
    /// hub 共享实例（members() 投影用；身份操作一律走 ctx 身份入口非本句柄）。
    hub: Arc<dyn MessageHub>,
    /// lead 会话存储（fork 摘要数据源）。
    sessions: Arc<dyn SessionStore>,
    /// lead 会话目录（成员自有会话 teammate-\<名\>.jsonl 的派生源）。
    session_dir: PathBuf,
    /// 共享簿记。
    core: Arc<TeamCore>,
}

impl TeamServiceState {
    /// 构造（plugin.rs setup 调用）。
    pub(crate) fn new(
        ctx: &Context,
        hub: Arc<dyn MessageHub>,
        sessions: Arc<dyn SessionStore>,
        config: &TeamConfig,
    ) -> Self {
        // 成员会话目录=config.path 同源派生（teammate-\<名\>.jsonl；config 单源，
        // ⑤改判：store trait 无路径口，目录取自 [session].path 同 key 配置值）
        let session_dir = std::path::Path::new(&config.path)
            .parent()
            .map(PathBuf::from)
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            ctx: ctx.downgrade(),
            hub,
            sessions,
            session_dir,
            core: Arc::new(TeamCore {
                board: Mutex::new(TaskBoard::new()),
                roster: Mutex::new(Roster::new()),
                fibers: Mutex::new(BTreeMap::new()),
                permits: Arc::new(Semaphore::new(config.max_concurrency as usize)),
                auto_claim: config.auto_claim.clone(),
                board_wakeup: Arc::new(tokio::sync::Notify::new()),
                resume_max: config.resume_max,
                resume_notify: config.resume_notify,
                tool_timeout_secs: config.tool_timeout_secs,
            }),
        }
    }

    /// 升级团队上下文（禁用/卸载后 fail-loud）。
    fn ctx(&self) -> Result<Context> {
        self.ctx.upgrade().ok_or_else(|| {
            CordisError::new(ErrorKind::EffectInactive, "团队插件已卸载，无法执行团队操作")
        })
    }

    /// 名册在册校验（任务板 actor 必须是真实成员或 lead）。
    fn ensure_actor(&self, actor: &str) -> Result<()> {
        if actor == "lead" || self.core.roster.lock().expect("名册锁").contains(actor) {
            return Ok(());
        }
        Err(CordisError::new(
            ErrorKind::HubUnknownMember,
            format!("\"{actor}\" 不在名册：任务板操作者必须是 lead 或在册成员"),
        ))
    }

    /// BoardOpError → 内核错误（工具面统一走 Result；fail-loud 不吞错）。
    fn board_err(err: BoardOpError) -> CordisError {
        CordisError::new(ErrorKind::HubInvalidName, err.to_string())
    }
}

impl TeamService for TeamServiceState {
    fn spawn_teammate(&self, spec: TeammateSpec) -> BoxFut<'static, Result<TeammateSpawned>> {
        // 借用改自有值预捕获（'static future 不携 &self）：ctx/sessions/core/dir
        let ctx = match self.ctx() {
            Ok(ctx) => ctx,
            Err(err) => return Box::pin(async move { Err(err) }),
        };
        let sessions = Arc::clone(&self.sessions);
        let core = Arc::clone(&self.core);
        let hub = Arc::clone(&self.hub);
        let session_dir = self.session_dir.clone();
        Box::pin(async move {
            // ① fork 摘要（裁②一期降级）：lead 会话尾部压缩为人设段文本
            let persona = match spec.context {
                ContextMode::Fresh => spec.prompt.clone(),
                ContextMode::Fork => {
                    let summary = crate::member::lead_session_summary(&*sessions).await;
                    format!("{}\n\n【lead 对话摘要（fork 一期降级注入）】\n{summary}", spec.prompt)
                }
            };
            // ② 预注册信箱（spawn 窗口：空白名/Online 重名 fail-loud）。Pending
            // 在册=离线成员重拉：跳过 open（成员 setup 的 register 直接认领原
            // 信箱=hub 续投语义，测试矩阵 hub 例⑧），不视为冲突
            let mailbox = match ctx.open_mailbox(&spec.name) {
                Ok(handle) => Some(handle),
                Err(err) => {
                    let is_pending_respawn = err.kind() == &ErrorKind::HubDuplicateMember
                        && !hub.members().iter().any(|m| m.name == spec.name && m.online);
                    if is_pending_respawn { None } else { return Err(err) }
                }
            };
            // ③ 建成员 fiber（挂团队 fiber；失败则 dispose 回收名字=hub 锚点语义）
            let session_path = session_dir.join(format!("teammate-{}.jsonl", spec.name));
            // await_ready 语义保留（K-7 行为：spawn 返回=成员 Active+信箱就绪）。
            // await_ready 用 rx.changed().await（真让出 runtime，worker task 可被
            // 调度）；此前 fire-and-forget 试验=误诊（诊断链在另一分支，回滚）。
            let spawn = ctx.plugin(
                MemberPlugin::new(spec.name.clone(), persona.clone(), session_path, Arc::clone(&core)),
                serde_json::json!({}),
            );
            match spawn.await_ready().await {
                Ok(()) => {}
                Err(err) => {
                    if let Some(mailbox) = mailbox {
                        mailbox.dispose().await;
                    }
                    return Err(err);
                }
            }
            // ④ 名册登记+句柄留存；开箱 effect 显式回收（已 Online=reclaim 无操作，
            // 仅摘 effect 条目防团队 fiber 上 effect 积累）
            core.roster
                .lock()
                .expect("名册锁")
                .register(&spec.name, spec.description.clone());
            core.fibers
                .lock()
                .expect("fiber 表锁")
                .insert(spec.name.clone(), spawn);
            // 已 Online 后开箱 effect 的 reclaim 无操作：显式 dispose 仅摘 effect 条目
            if let Some(mailbox) = mailbox {
                mailbox.dispose().await;
            }
            Ok(TeammateSpawned {
                name: spec.name,
                description: spec.description,
                note: "信箱已预注册并上线：拉起窗口内的消息将在其首次消费时取净".into(),
            })
        })
    }

    fn send_message(&self, to: Address, kind: MessageKind, body: String) -> Result<SentReceipt> {
        // 身份=团队 fiber（"lead"）：ctx 身份入口→hub 派发，from 不可自报
        self.ctx()?.hub_send(to, OutgoingMessage { kind, body })
    }

    fn task_create(&self, title: String, description: String) -> Result<u64> {
        self.task_create_full(title, description, String::new(), vec![])
    }

    fn task_create_full(
        &self,
        title: String,
        description: String,
        capability: String,
        blocked_by: Vec<u64>,
    ) -> Result<u64> {
        Ok(self
            .core
            .board
            .lock()
            .expect("任务板锁")
            .create(title, description, "lead", capability, blocked_by))
    }

    fn task_claim(&self, task_id: u64, actor: &str) -> Result<()> {
        self.ensure_actor(actor)?;
        self.core
            .board
            .lock()
            .expect("任务板锁")
            .claim(task_id, actor)
            .map_err(Self::board_err)
    }

    fn task_submit(&self, task_id: u64, actor: &str) -> Result<()> {
        self.ensure_actor(actor)?;
        self.core
            .board
            .lock()
            .expect("任务板锁")
            .submit(task_id, actor)
            .map_err(Self::board_err)?;
        crate::service_notify::notify_review(
            &self.core, &self.ctx, task_id,
        );
        Ok(())
    }

    fn task_review(&self, task_id: u64, actor: &str, approve: bool) -> Result<()> {
        self.ensure_actor(actor)?;
        self.core
            .board
            .lock()
            .expect("任务板锁")
            .review(task_id, actor, approve)
            .map_err(Self::board_err)?;
        crate::service_notify::notify_unlock(
            &self.core, &self.ctx, task_id, approve,
        );
        Ok(())
    }

    fn task_reassign(&self, task_id: u64, new_owner: &str) -> Result<()> {
        // 改派权=派工权：v1 仅 lead（dsh team_task_update 同语义；成员请求改派走
        // 消息面向 lead 提请）
        self.core
            .board
            .lock()
            .expect("任务板锁")
            .reassign(task_id, new_owner, "lead")
            .map_err(Self::board_err)
    }

    fn kick(&self, name: &str) -> BoxFut<'static, Result<()>> {
        let fiber = self.core.fibers.lock().expect("fiber 表锁").get(name).cloned();
        let core = Arc::clone(&self.core);
        let name = name.to_string();
        Box::pin(async move {
            let Some(fiber) = fiber else {
                return Err(CordisError::new(
                    ErrorKind::HubUnknownMember,
                    format!("\"{name}\" 不在册或已离场"),
                ));
            };
            fiber.dispose().await?; // hub 条目降回 Pending（信箱保留）
            core.roster.lock().expect("名册锁").remove(&name);
            core.fibers.lock().expect("fiber 表锁").remove(&name);
            Ok(())
        })
    }

    fn drain_lead_inbox(&self) -> Result<Vec<DeliveredMessage>> {
        // 与 send_message 同一身份通道（团队 fiber="lead"）：drain 的身份派发
        // 语义天然防越权（非 lead fiber 取不到此信箱）
        self.ctx()?.drain_inbox()
    }

    fn task_list(&self) -> Vec<TeamTaskView> {
        self.core.board.lock().expect("任务板锁").list()
    }

    fn roster(&self) -> Vec<RosterEntry> {
        // 在线标志来自 hub members()（消息侧唯一真源），本表只供元数据合成
        let online: BTreeMap<String, bool> = self
            .hub
            .members()
            .into_iter()
            .map(|info| (info.name, info.online))
            .collect();
        self.core
            .roster
            .lock()
            .expect("名册锁")
            .snapshot()
            .into_iter()
            .map(|(name, meta)| RosterEntry {
                busy: meta.busy,
                busy_since: meta.busy_since,
                task: meta.task,
                online: online.get(&name).copied().unwrap_or(false),
                description: meta.description,
                name,
            })
            .collect()
    }
}

/// 工具注册（plugin.rs setup 调用）：lead 侧工具族进全局注册表。
pub(crate) fn register_lead_tools(ctx: &Context, service: Arc<dyn TeamService>) -> Result<()> {
    let registry = ctx.get::<dyn yunyou_tools::RegistryService>(
        yunyou::ServiceId::of::<dyn yunyou_tools::RegistryService>(),
    )?;
    for tool in tools::lead_tools(Arc::clone(&service)) {
        let name = tool.name().to_string();
        // 注册即 effect（方案 §2.1.4）：团队 fiber 卸载时工具随撤。
        // 每轮克隆 registry Arc（trait register 接收器按值取 Arc，循环内不可吞）
        let disposer = Arc::clone(&registry).register_tool(tool)?;
        ctx.effect(format!("team.tool({name})"), move || Ok(Some(disposer)))?;
    }
    Ok(())
}
