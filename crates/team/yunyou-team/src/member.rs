//! 成员插件：每 teammate 一个独立 fiber（团队 fiber 的子 fiber），自带会话、
//! 人设与回合循环——「teammate=共享 Context 独立 fiber（自己的 Session+
//! AgentLoop）」（任务26 v1 范围原文）。
//!
//! ## 回合循环
//!
//! - **唤醒**：subscribe_messages 只作唤醒信号（观测面推=尽力而为），消费仍走
//!   drain（双通道定案不破：拉=消费、推=观测）；tokio Notify 的 permit 在无
//!   等待者时持久保存——**提醒不丢**（连发合并为单 permit 也安全：醒来 drain
//!   取净信箱即可），空闲兜底轮询仅防极端竞态（订阅回调注册前入站/回调链路
//!   故障，周期见 [`IDLE_BACKSTOP_INTERVAL`]）——信箱是事实源，推送只是节拍器；
//! - **消费**：drain_inbox 取净→格式化进轮输入（偏差申报 a，lead 已批：消息是
//!   会话输入非系统策略；人设走 persona 槽=runner 既有成员注入点）；
//! - **完工自动汇报**（五点规格③）：run_turn 终结（Ok/Err）由成员 fiber 自身经
//!   ctx.hub_send 发 Report 给 lead——谁终结谁发送；在别处代发会被 hub 身份
//!   派发拦截（融合终版 §1.4 Report 联动细则，集成测试锁定）；
//! - **取消/离场**：fiber 卸载令牌贯穿 runner（合一取消）→ 循环退出 → effect
//!   清理 → hub 条目降回 Pending（信箱与未取净消息保留，同名重拉续投）。

use crate::service::TeamCore;
use yunyou::hub::{Address, MessageKind, OutgoingMessage};
use yunyou_session::{SessionEvent, SessionStore};
use yunyou::{ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId, projection_of};
use yunyou_agent_loop::AgentLoopRunner;
use yunyou_llm::Llm;
use yunyou_session::JsonlStore;
use yunyou_system_prompt::SystemPrompt;
use yunyou_tools::RegistryService;
use yunyou_util::BoxFut;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// 空闲兜底轮询周期（task-20①，挑刺 P2-1）：唤醒主通道=Notify（订阅回调
/// notify_one），兜底只防极端竞态（订阅回调注册前的入站由 hub 补投兜底+回调
/// 链路故障）——Notify permit 持久不丢，兜底周期取卡面 1-5s 口径上界即可：
/// 空转 drain 自 100ms 纯轮询时代的 10 次/s/成员降至 0.2 次/s/成员（50 倍降幅），
/// hub 全局锁空转占用同步下降；消息到达的响应性由 Notify 即时唤醒承担
///（非本周期决定，tests/wakeup.rs 锁功能活性上界）。
const IDLE_BACKSTOP_INTERVAL: Duration = Duration::from_secs(5);

/// 成员插件（spawn 时构造；每实例一 fiber；字段即元数据单一真源）。
pub(crate) struct MemberPlugin {
    /// 成员名（=插件名=hub 在册名）。
    name: String,
    /// 人设全文（spawn 规格的 prompt，fork 时含 lead 对话摘要）。
    persona: String,
    /// 成员自有会话路径（与 lead 会话同目录 teammate-\<名\>.jsonl）。
    session_path: PathBuf,
    /// 团队共享簿记（busy 置位/并发闸）。
    core: Arc<TeamCore>,
}

impl MemberPlugin {
    pub(crate) fn new(
        name: String,
        persona: String,
        session_path: PathBuf,
        core: Arc<TeamCore>,
    ) -> Self {
        Self { name, persona, session_path, core }
    }
}

impl Plugin for MemberPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: format!("teammate-{}", self.name),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec![
                "message.v1".into(),
                "llm.v1".into(),
                "session.v1".into(),
                "tools.v1".into(),
                "system-prompt.v1".into(),
            ],
            provides: vec![],
            inject: vec![
                projection_of::<dyn yunyou::hub::MessageHub>(),
                projection_of::<dyn Llm>(),
                projection_of::<dyn RegistryService>(),
                projection_of::<dyn SystemPrompt>(),
            ],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![
            ServiceId::of::<dyn yunyou::hub::MessageHub>(),
            ServiceId::of::<dyn Llm>(),
            ServiceId::of::<dyn RegistryService>(),
            ServiceId::of::<dyn SystemPrompt>(),
        ]
    }

    fn setup(
        self: Arc<Self>,
        ctx: Context,
        _config: ConfigValue,
    ) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            // ① 身份认领：Pending 预注册信箱→Online（补投就绪）；无条目=直接 Online
            ctx.register_member(&self.name)?;
            // ② 依赖句柄（持 Arc<dyn Trait>=强句柄纪律允许形态）
            let llm = ctx.get::<dyn Llm>(ServiceId::of::<dyn Llm>())?;
            let tools = ctx.get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())?;
            let prompt = ctx.get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>())?;
            let retry = ctx
                .get::<dyn yunyou_llm::RetryPolicyProvider>(ServiceId::of::<
                    dyn yunyou_llm::RetryPolicyProvider,
                >())
                .ok();
            // ③ 成员自有会话（会话是各自的事实源；路径=lead 会话同目录派生）
            let sessions: Arc<dyn SessionStore> = JsonlStore::new(self.session_path.clone());
            // ④ 成员工具面：allowlist 裁剪+身份化覆写（发消息=自己身份；lead 专属
            //    工具不可见——见 member_tools.rs）
            let member_tools =
                crate::member_tools::member_registry(&ctx, &self.name, Arc::clone(&tools))?;
            // ⑤ 唤醒信标（观测订阅只作节拍器，消费在 drain）
            let wakeup = Arc::new(Notify::new());
            let mirror = Arc::clone(&wakeup);
            // 订阅本身就是 fiber effect（注册即 effect）：句柄存续与否不影响
            // 订阅随 fiber 卸载自动注销，丢弃句柄仅表示不显式提前撤销
            let _subscription = ctx.subscribe_messages(Arc::new(
                move |_message: &yunyou::hub::DeliveredMessage| {
                    let mirror = Arc::clone(&mirror);
                    Box::pin(async move {
                        mirror.notify_one();
                        Ok(())
                    })
                },
            ))?;
            // ⑥ 回合循环任务（持 WeakContext：强句柄仅限 setup 窗口）
            let worker = MemberWorker {
                name: self.name.clone(),
                persona: self.persona.clone(),
                ctx: ctx.downgrade(),
                sessions,
                llm,
                tools: member_tools,
                prompt,
                retry,
                core: Arc::clone(&self.core),
                wakeup,
                cancel: ctx.cancellation(),
            };
            tokio::spawn(worker.run());
            Ok(FiberOutput)
        })
    }
}

/// 成员回合循环（独立任务；fiber 卸载令牌触发即静止退出）。
struct MemberWorker {
    name: String,
    persona: String,
    ctx: yunyou::WeakContext,
    sessions: Arc<dyn SessionStore>,
    llm: Arc<dyn Llm>,
    tools: Arc<dyn RegistryService>,
    prompt: Arc<dyn SystemPrompt>,
    retry: Option<Arc<dyn yunyou_llm::RetryPolicyProvider>>,
    core: Arc<TeamCore>,
    wakeup: Arc<Notify>,
    cancel: CancellationToken,
}

impl MemberWorker {
    /// K-16② 重启续跑检测（尾部扫描三类：软着陆 SystemMessage/工具无配对/
    /// 孤儿 user——设计 §3.2）：命中=自动发「继续」（round<2）+ResumeMark 落盘；
    /// round≥2=升级不自动续（hub 报 lead 人工介入）。
    async fn startup_resume_check(&self, ctx: &Context) {
        let Ok(events) = self.sessions.load_all().await else { return };
        let Some(last) = events.last() else { return };
        let reason = match last {
            SessionEvent::SystemMessage { text } if text.contains("回复「继续」") => "软着陆中断",
            SessionEvent::ToolCall { .. } => "工具执行中断",
            SessionEvent::UserMessage { .. } => "孤儿输入",
            _ => return,
        };
        let round = events
            .iter()
            .rev()
            .find_map(|e| match e {
                SessionEvent::ResumeMark { round, .. } => Some(*round),
                _ => None,
            })
            .unwrap_or(0);
        if round >= self.core.resume_max {
            let _ = ctx.hub_send(
                Address::Member("lead".into()),
                OutgoingMessage {
                    kind: MessageKind::System,
                    body: format!("成员「{}」重启检测：未完回合续跑已达上限（round={round}），请人工介入（reason={reason}）。", self.name),
                },
            );
            return;
        }
        let mark = SessionEvent::ResumeMark { round: round + 1, reason: reason.to_string() };
        let _ = self.sessions.append(&mark).await;
        let _ = ctx.hub_send(
            Address::Member(self.name.clone()),
            OutgoingMessage {
                kind: MessageKind::System,
                body: format!("【自动续跑】检测到未完回合（{reason}，round={round}）——自动继续推进。"),
            },
        );
        eprintln!("[k16] 成员 {} 重启续跑 round={}", self.name, round + 1);
    }

    async fn run(self) {
        // K-16② 启动续跑检测（首回合前——设计 §3.2 检测段位置）
        if let Some(ctx) = self.ctx.upgrade() {
            self.startup_resume_check(&ctx).await;
        }
        loop {
            if self.cancel.is_cancelled() { return; }
            // 醒来即消费（含首圈，task-20①）：等待臂置于循环尾——register 认领
            // Pending 信箱的续投消息**不触发 Notify**（订阅回调只在 send 时
            // notify_one，认领时已落信箱的历史消息无信号），worker 首圈必须
            // 立即 drain 才能消费续投；先 drain 后等待的结构同时消掉「醒来先
            // 再等一拍」的空转圈（首圈与每次唤醒都直达消费）。
            let Some(ctx) = self.ctx.upgrade() else { return };
            let Ok(inbox) = ctx.drain_inbox() else { return };
            if inbox.is_empty() {
                // K-17 自动领取检查（idle+ready+capability 匹配，lead 批①）
                if let Some(task_input) = self.try_auto_claim() {
                    let _permit = self.core.permits.acquire().await;
                    self.core.roster.lock().expect("名册锁").set_busy(&self.name, true);
                    self.run_one(&ctx, task_input).await;
                    self.core.roster.lock().expect("名册锁").set_busy(&self.name, false);
                }
            } else {
                let _permit = self.core.permits.acquire().await;
                self.core.roster.lock().expect("名册锁").set_busy(&self.name, true);
                for message in inbox {
                    if self.cancel.is_cancelled() { break; }
                    let input = format!("【来自 {}】{}", message.from, message.body);
                    self.run_one(&ctx, input).await;
                }
                self.core.roster.lock().expect("名册锁").set_busy(&self.name, false);
            }
            // 等待臂（循环尾）：Notify 唤醒即到即消费；兜底周期仅防极端竞态
            // （Notify permit 持久不丢，连发合并安全——见 IDLE_BACKSTOP_INTERVAL）。
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = self.wakeup.notified() => {}
                // K-18②：任务板解锁事件（approve→notify_waiters 广播）——
                // 醒来→循环顶 drain→空则 try_auto_claim（K-17 衔接=唤醒后
                // 首查可领任务）
                _ = self.core.board_wakeup.notified() => {}
                _ = tokio::time::sleep(IDLE_BACKSTOP_INTERVAL) => {}
            }
        }
    }


    /// 自动领取（K-17①）：idle 时查板领活→claim→组装回合输入。
    /// 策略三闸全走（enabled/capability 匹配/硬顶），领不到=None 静默（约束⑤）。
    fn try_auto_claim(&self) -> Option<String> {
        let config = &self.core.auto_claim;
        if !config.enabled {
            return None;
        }
        let board = self.core.board.lock().expect("任务板锁");
        let pending = board.pending_matching(&config.capability);
        let active = board.active_claimed_count(&self.name);
        drop(board);
        let candidates = crate::auto::evaluate_claims(
            config,
            &pending,
            &[(self.name.clone(), config.capability.clone())],
            &[(self.name.clone(), active)],
        );
        let candidate = candidates.first()?;
        // claim 走既有闸（非 pending/已领拒原样=SelfReview 同效纪律②）
        let (title, description) = {
            let mut board = self.core.board.lock().expect("任务板锁");
            board.claim(candidate.task_id, &self.name).ok()?;
            board.get_task_info(candidate.task_id)?
        };
        Some(format!(
            "【自动分配】任务 #{}「{}」\n{}",
            candidate.task_id, title, description
        ))
    }

    /// 跑一轮并按终结形态发 Report（Ok/Err 两路都汇报=「做完了会给你发消息」是
    /// 机制不是习惯）。发送点=成员 fiber 上下文（谁终结谁发送）。取消形态不发：
    /// 取消即 fiber 正在退场，Report 随信箱语义留待续投/静默，不阻断退场。
    async fn run_one(&self, ctx: &Context, input: String) {
        let runner = AgentLoopRunner {
                    // K-16① team 成员=auto_winddown true（自动续跑语义）；
                    // 预警比 0.8；宽步 4（与 plugin.rs REPL 侧缺省同源）。
                    budget_warn_ratio: 0.8,
                    auto_winddown: true,
                    winddown_grace_steps: 4,
            sessions: Arc::clone(&self.sessions),
            llm: Arc::clone(&self.llm),
            tools: Arc::clone(&self.tools),
            prompt: Arc::clone(&self.prompt),
            persona: self.persona.clone(),
            // task-30③（云垒 P2-8 解围闸）：成员工具超时对齐 [agent_loop]
            // 语义——0=不限时，非 0=Some(Duration)（AgentLoopRunner 消费侧同构
            // agent-loop/plugin.rs:163）。默认 300s：挂死工具超时熔断，permit
            // 释放给其余成员（4 permit 全队停摆的根除点）。宽限 runner 经
            // runner.tool_timeout 继承同值（K-16 形态，宽限阶段同样熔断）。
            tool_timeout: if self.core.tool_timeout_secs == 0 {
                None
            } else {
                Some(std::time::Duration::from_secs(self.core.tool_timeout_secs))
            },
            // 护栏两旋钮默认全关（task-23）：成员回合与主循环同语义——无步数
            // 硬顶跑到模型停手；运营成本闸候 [agent_loop] 配置面按需拧。
            max_steps: None,
            token_budget: None,
            context_window: None,
            compact_reserved_output: 20_000,
            compact_safety_margin: 32_000,
            auto_compact_enabled: true,
            compaction_failure_limit: 3,
            compaction_state: yunyou_agent_loop::CompactionState::new(),
            fiber_cancel: self.cancel.child_token(),
            retry: self.retry.clone(),
            hooks: Default::default(),
        };
        // K-16① 触达自动收尾：宽限 runner 预铸（run_turn 消费 self——宽限参数
        // 与字段克隆在消费前完成）
        let grace_runner = AgentLoopRunner {
            max_steps: Some(runner.winddown_grace_steps),
            auto_winddown: false,
            context_window: runner.context_window,
            compact_reserved_output: runner.compact_reserved_output,
            compact_safety_margin: runner.compact_safety_margin,
            auto_compact_enabled: runner.auto_compact_enabled,
            compaction_failure_limit: runner.compaction_failure_limit,
            compaction_state: yunyou_agent_loop::CompactionState::new(),
            sessions: Arc::clone(&runner.sessions),
            llm: Arc::clone(&runner.llm),
            tools: Arc::clone(&runner.tools),
            prompt: Arc::clone(&runner.prompt),
            persona: runner.persona.clone(),
            tool_timeout: runner.tool_timeout,
            token_budget: runner.token_budget,
            budget_warn_ratio: runner.budget_warn_ratio,
            winddown_grace_steps: runner.winddown_grace_steps,
            fiber_cancel: runner.fiber_cancel.child_token(),
            retry: runner.retry.clone(),
            hooks: runner.hooks.clone(),
        };
        let outcome = runner.run_turn(input, self.cancel.clone()).await;
        let outcome = if let Ok(done) = &outcome {
            if done.auto_resume {
                eprintln!("[k16] 触达护栏——自动续收尾回合（宽限 {} 步）", grace_runner.winddown_grace_steps);
                grace_runner.run_turn("继续".into(), self.cancel.child_token()).await
            } else {
                outcome
            }
        } else {
            outcome
        };
        let body = match &outcome {
            Ok(done) => {
                let truncated = if done.truncated { "（步数超限截断）" } else { "" };
                let wind = if done.wind_down { "（收尾完成）" } else { "" };
                format!("任务回合完成{truncated}{wind}：{}", done.assistant_text)
            }
            Err(err) => format!("任务回合失败：{err}"),
        };
        let _ = ctx.hub_send(
            Address::Member("lead".into()),
            OutgoingMessage { kind: MessageKind::Report, body },
        );
        // K-16③ 中断上报（Interrupted 态→hub 投 lead——复用 K-7 Report 通道
        // 反向事件零新通道；节流=同回合仅此一报；开关=resume_notify 配置）
        if outcome.is_err() && self.core.resume_notify {
            let reason = outcome.as_ref().err().map(|e| e.to_string()).unwrap_or_default();
            let _ = ctx.hub_send(
                Address::Member("lead".into()),
                OutgoingMessage {
                    kind: MessageKind::System,
                    body: format!("【中断上报】成员「{}」回合中断：{reason}", self.name),
                },
            );
        }
    }
}

/// lead 会话尾部压缩（fork 一期降级，裁②）：最近事件的「角色: 摘要」行，截断
/// 防膨胀——真 fork 随二期会话投影，届时换实现不换 spawn 参数面。
pub(crate) async fn lead_session_summary(sessions: &dyn SessionStore) -> String {
    let Ok(events) = sessions.load_all().await else {
        return "（lead 会话不可读，按空白上下文行事）".into();
    };
    let lines: Vec<String> = events
        .iter()
        .rev()
        .take(20)
        .map(summarize_event)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    lines.join("\n")
}

/// 单事件摘要（会话事实源的最小投影；未知类型行已被 store 层跳过）。
fn summarize_event(event: &yunyou_session::SessionEvent) -> String {
    use yunyou_session::SessionEvent as E;
    const MAX: usize = 120;
    let clip = |s: &str| s.chars().take(MAX).collect::<String>();
    match event {
        E::UserMessage { text } => format!("lead: {}", clip(text)),
        E::AssistantMessage { text, .. } => format!("assistant: {}", clip(text)),
        E::ToolCall { name, .. } => format!("tool: {name} 调用"),
        E::ToolResult { name, is_error, .. } => {
            format!("tool: {name} {}", if *is_error { "失败" } else { "完成" })
        }
        E::SystemMessage { text } => format!("system: {}", clip(text)),
        _other => "event（未展开）".to_string(),
    }
}
