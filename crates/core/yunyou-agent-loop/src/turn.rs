//! 一轮对话的执行体（取输入 → 组请求 → 流式 → 工具循环 → 落盘）。

use crate::aggregate::Aggregated;
use crate::cancel::merge_cancel;
use yunyou::{CordisError, ErrorKind};
use yunyou_llm::{GenerateRequest, Llm, StreamDelta};
use yunyou_session::{SessionEvent, SessionStore};
use yunyou_system_prompt::SystemPrompt;
use yunyou_tools::RegistryService;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// 一轮对话的产出。
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    /// 最终助手文本（多步时为最后一步的可见文本）。
    pub assistant_text: String,
    /// 本轮累计的工具调用次数。
    pub tool_calls: usize,
    /// 自动收尾挂起标记（K-16①：auto_winddown 模式触达护栏=true——调用方
    /// （成员 worker）见此标记自动带「继续」执行收尾回合一次；REPL 人工模式
    /// 恒 false=现语义）。
    pub auto_resume: bool,
    /// 收尾回合终态标记（K-16①：宽限回合产出=阶段总结+剩余申报→WindDown 终；
    /// 宽限耗尽仍 false 且 truncated=true=诚实截断）。
    pub wind_down: bool,
    /// 护栏软着陆暂停标记（task-23 语义改造：原「步数超限截断」——主循环
    /// 步数硬顶已删（对齐三家参照系），true = 显式拧上的护栏（max_steps/
    /// token_budget）触发的软着陆暂停，上下文完整保留、回复「继续」无缝
    /// 续跑；false = 正常收尾。渲染侧「已截断」旧文案候 cli 域联动小刀）。
    pub truncated: bool,
}

/// 主循环服务（对标 TS AgentLoop 的服务面；字段均为 Arc/值，可廉价克隆）。
#[derive(Clone)]
pub struct AgentLoopRunner {
    /// 会话存储（会话是唯一事实源）。
    pub sessions: Arc<dyn SessionStore>,
    /// LLM 服务。
    pub llm: Arc<dyn Llm>,
    /// 工具注册表。
    pub tools: Arc<dyn RegistryService>,
    /// 系统提示词服务。
    pub prompt: Arc<dyn SystemPrompt>,
    /// 人设文本（worker 人设注入点，方案 §2.7.2）。
    pub persona: String,
    /// 单工具执行超时（None = 不限时）。
    pub tool_timeout: Option<Duration>,
    /// 工具循环步数护栏（None = 无上限；语义与软着陆形态见
    /// [`crate::plugin::AgentLoopConfig::max_steps`]，默认全关=行为与无护栏一致）。
    pub max_steps: Option<usize>,
    /// token 预算护栏（None = 不设；usage 缺失时对本轮失效，诚实注记见
    /// [`crate::plugin::AgentLoopConfig::token_budget`]）。
    pub token_budget: Option<u64>,
    /// 预警段阈值（K-16①，步数与 token 独立判定；默认 0.8）。
    pub budget_warn_ratio: f64,
    /// 触达自动收尾模式（K-16①；team 成员 true/REPL false）。
    pub auto_winddown: bool,
    /// 收尾回合宽步硬顶（K-16①；默认 4）。
    pub winddown_grace_steps: usize,
    /// 模型上下文窗口（None = auto-compact 整体关；语义见
    /// [`crate::plugin::AgentLoopConfig::context_window`]，task-27）。
    pub context_window: Option<u64>,
    /// 摘要输出预留（默认 20k；语义见 plugin config）。
    pub compact_reserved_output: u64,
    /// 触发线安全余量（默认 32k；语义见 plugin config）。
    pub compact_safety_margin: u64,
    /// 自动压缩开关（默认 true；与窗口「与」关系）。
    pub auto_compact_enabled: bool,
    /// 摘要连败熔断阈值（默认 3）。
    pub compaction_failure_limit: u32,
    /// 压缩会话级状态（熔断计数；Arc 跨回合共享——进程生命周期内有效）。
    pub compaction_state: Arc<crate::compaction::CompactionState>,
    /// fiber 卸载取消令牌（方案 §5.1 R1）：与调用方令牌在 [`Self::run_turn`] 内
    /// 合并为合一令牌，任一触发即整轮收流。由插件 setup 经内核真源
    /// [`yunyou::Context::cancellation`] 注入（见 [`crate::plugin::AgentLoopPlugin`]）；
    /// 宿主直构 runner 传空令牌（不取消）。
    pub fiber_cancel: CancellationToken,
    /// 重试策略服务（S10，设计 §2）：`None` = 策略卡不在场，LLM 失败直出直连
    /// （设计验收总口径「yunyou-retry 禁用后无重试直连，fail-fast 可预期」）。
    /// 由插件 setup 期容错解析（见 [`crate::plugin::AgentLoopPlugin`] rustdoc 的
    /// inject 偏差说明）；宿主直构 runner 传 `None` 即关停重试。
    pub retry: Option<Arc<dyn yunyou_llm::RetryPolicyProvider>>,
    /// 组装前钩子注册表（task-11 缝）：请求出口受控裁剪（声明式指令+配对守卫+
    /// 错误隔离三态，机制见 [`crate::hooks`]）。与插件服务面共享同一实例——
    /// 空注册=行为与无钩子逐字节一致（零开销短路）。
    pub hooks: Arc<crate::hooks::HooksRegistry>,
    /// 工具执行前审批缝（task-16 S1）：gate 判定链+审批呈现器表（对象类型
    /// 注册制）。空注册=每工具一次 is_empty 短路，行为与无缝逐字节一致。
    pub gates: Arc<crate::gate::GateHub>,
}

impl AgentLoopRunner {
    /// 跑一轮完整对话（兼容形态：无观察者）。
    pub async fn run_turn(
        self,
        user_input: String,
        cancel: CancellationToken,
    ) -> Result<TurnOutcome, CordisError> {
        self.run_turn_inner(user_input, cancel, None).await
    }

    /// 跑一轮完整对话（观察者形态：边跑边发 [`crate::observer::TurnEvent`]）。
    ///
    /// 事件时序契约：落盘类事件（UserMessage/ToolCall/ToolResult/AssistantMessage）
    /// 在对应 JSONL 落盘**成功之后**发出；流式增量（AssistantDelta/ReasoningDelta）
    /// 是过程态、先于落盘即时发出；本轮以 TurnComplete 收尾（Err 路径不发）。
    pub async fn run_turn_observed(
        self,
        user_input: String,
        cancel: CancellationToken,
        observer: Arc<dyn crate::observer::TurnObserver>,
    ) -> Result<TurnOutcome, CordisError> {
        self.run_turn_inner(user_input, cancel, Some(observer)).await
    }

    /// 跑一轮完整对话（含工具循环）。
    ///
    /// # 取消源（方案 §5.1 R1）
    /// 调用方令牌（REPL Ctrl-C 等）与 fiber 卸载取消两源在 **UserMessage 落盘之后**
    /// 合并为合一令牌（合并点有意后置：预取消也不改变本轮落盘起点，验收项
    /// 「UserMessage 落盘逐字节不变」）：`caller.child_token()` 为合并点，fiber 取消
    /// 由桥接任务触发 child；任一触发即整轮收流（检查点=循环头与流边界两处显式
    /// 检查 + 工具执行同令牌传参，三处同源；节奏与合并前一致）。
    ///
    /// # 前置条件
    /// 内核中 llm/sessions/tools/systemPrompt 四服务已 Active。
    /// # 失败语义
    /// LLM 流失败 / 会话写失败 → `Err`（本轮已落盘的事件保留，供续跑）；
    /// 工具失败不终止（is_error 回传模型）。
    async fn run_turn_inner(
        self,
        user_input: String,
        cancel: CancellationToken,
        observer: Option<Arc<dyn crate::observer::TurnObserver>>,
    ) -> Result<TurnOutcome, CordisError> {
        let input = user_input.trim().to_string();
        if input.is_empty() {
            return Err(CordisError::new(ErrorKind::Internal, "输入为空：没有可处理的内容")); // task-59c 人话化（原 loop/空输入 内部码）
        }
        // 上下文水位治理（task-27 回合头触发——S3 承重墙：必须在当轮
        // UserMessage 落盘之前，否则「最后一个 compaction」分割会把本轮用户
        // 消息切进摘要区=丢题变体）。水位=上回合末回写（compaction_state，
        // 跨回合 Arc 共享）；触发回合 load_all 为压缩专用读（+1 次，默认关
        // 断言不受扰——io_count 锚）。失败=本回合不压缩继续旧上下文+熔断
        // 计数（连败停用），不中断对话（设计稿 §7 表）。
        if self.should_auto_compact(self.compaction_state.watermark())
            && !cancel.is_cancelled()
        {
            let watermark = self.compaction_state.watermark().unwrap_or_default();
            let mut compact_history = self.sessions.load_all().await?;
            match self
                .run_compaction(&mut compact_history, &cancel, watermark, &observer)
                .await
            {
                Ok(()) => {}
                Err(err) => {
                    let failures =
                        self.compaction_state.failures.fetch_add(1, Ordering::Relaxed) + 1;
                    eprintln!(
                        "[compact] 摘要失败（{err}）——本回合继续旧上下文（{failures}/{}）",
                        self.compaction_failure_limit
                    );
                    if failures >= self.compaction_failure_limit {
                        self.compaction_state.disabled.store(true, Ordering::Relaxed);
                        eprintln!(
                            "[compact] 自动压缩连续失败 {failures} 次已停用（/compact 可手动重试）"
                        );
                    }
                }
            }
        }
        self.persist(&SessionEvent::UserMessage { text: input.clone() }).await?;
        emit(
            &observer,
            crate::observer::TurnEvent::UserMessage { text: input },
        )
        .await;
        // 取消源合并（方案 §5.1 R1）：调用方令牌的 child token 为合并点，fiber 卸载
        // 取消经桥接任务触发；任一触发即整轮收流。合并点有意后置于 UserMessage
        // 落盘之后（预取消不改变落盘起点，落盘逐字节不变）；检查点=循环头与流
        // 边界两处显式检查 + 工具执行同令牌传参（三处同源，节奏与合并前一致）。
        // `_bridge` 存活至本轮结束：drop 即中止桥接任务，不残留跨轮监听（无孤儿）。
        let (cancel, _bridge) = merge_cancel(&cancel, &self.fiber_cancel);
        // 本地会话账本（task-15，挑刺 P1-4）：回合起点单次 load_all，此后每个
        // persist 点同步追加——工具循环内不再全量重读（原每步 load_all+全量解析
        // =O(步数×会话大小) 次 IO，16 步长回合+2MB 会话=秒级纯开销，flock 独占下
        // 重读零一致性收益）。
        // 等价性论证三层（复核人云驰验收锚点）：
        // ① flock 进程存活期独占会话文件（跨进程无并发写者）；
        // ② REPL 单线程回合串行（本进程无并发写者）；
        // ③ web 回合闸门 try_lock 拒绝并发提交（同上）；
        // → 推论「回合内无并发写者」→ 本地账本与每步全量重读逐事件等价。
        // 回改触发条件：未来引入回合中外部注入（消息热插等）须改「账本+重读
        // 脏标记」，不得静默沿用本假设（tests/io_count.rs 以 load_all=1 断言锁死
        // 回潮面——每回合恰 1 次全量读是本方案语义本体，改断言须留痕）。
        // 重试不重读：stream_once_with_retry 重发同一 request，不触账本——
        // 失败重试无刷新账本的必要（会话事实源未变）。
        // 收尾 SystemMessage（步数超限）后无 build_request 消费方，push 仅为
        // 「persist 成功即 push」的账本完整性单纪律。
        let mut history = self.sessions.load_all().await?;
        // L-3 恢复兜底（五席评审 20261117）：回合起点扫描悬空 ToolCall 并补
        // 合成错误 ToolResult（详情见 turn_steps::heal_dangling_tool_calls）。
        // 只在真有悬空时才追加写，常态会话零影响（io_count 锚不扰）。
        self.heal_dangling_tool_calls(&mut history, &observer).await?;
        let mut tool_calls_total = 0usize;
        // 当前上下文水位（task-26，语义见循环内记录点注释）：最后一步 usage.prompt。
        // 延迟初始化（task-23：loop 无循环外尾段，首次使用前必先赋值——
        // 原 for 版初值 "" 死值被 clippy 抓出，收口为延迟初始化）。
        let mut assistant_text;
        let mut stats = crate::turn_steps::TurnTokenStats { usage_acc: None, context: None };
        // 终止语义（task-23，业主实证令 2026-11-16「正常用怎么还触发16步上限，
        // 不应该一直循环到任务完成吗，只有异常才触发」+云枢三点裁决）：主循环
        // 无步数硬顶（对齐 Claude Code maxTurns 默认不传=无上限/DSH TS 零命中
        // 上限——跑到模型不再调工具为止），终=模型停手/取消/错误三路；防跑飞
        // =取消令牌贯穿（上方合并点）+三路超时+错误上浮+流式可见性。双护栏
        // 旋钮（max_steps/token_budget）默认全关=行为与无护栏完全一致；显式
        // 拧上即同意暂停语义，触及时步末统一软着陆（turn_steps::finish_paused，
        // 「回复『继续』无缝延续」），禁腰斩截肢。步末判定=工具循环完整闭合后：
        // 保证 ToolCall/ToolResult 配对完整落盘（中途断会留悬空 tool_calls 让
        // 端点拒收下一轮请求）——预算护栏滞后一步的误差 ≤ 单步 token，换配对
        // 完整性（取舍自证）。
        let mut steps_used = 0usize;
        loop {
            if cancel.is_cancelled() {
                return Err(CordisError::new(ErrorKind::Internal, "loop/cancelled"));
            }
            let mut request = crate::request::compose_request(
                self.tools.as_ref(),
                self.prompt.as_ref(),
                &self.persona,
                &history,
            );
            // task-11 组装前钩子缝（设计草图 lead 批 2026-11-18）：请求出口
            // 受控裁剪——钩子声明式指令经引擎单点合并应用（配对守卫+错误隔离
            // 三态见 hooks 模块头）。触发点在重试包装之外：重试重发同一 request
            // 不重组装，钩子不重复触发。空注册零开销短路。
            if !self.hooks.is_empty() {
                let edits = self
                    .hooks
                    .run(crate::hooks::PreComposeRequest {
                        messages: &request.messages,
                        steps_used,
                        context_tokens: stats.context,
                        context_window: self.context_window,
                    })
                    .await;
                crate::hooks::apply_edits(&mut request.messages, &edits);
            }
            // S10 消费点（设计 §1.3「等价位=stream_once 失败路径」）：重试决策与
            // 退避执行在 crate::retry（策略卡产决策，本侧管 sleep 与取消 select）。
            let aggregated =
                crate::retry::stream_once_with_retry(&self, request, &cancel, &observer).await?;
            // 当前上下文水位（task-26）：prompt=本回合发送的上下文全量——LLM
            // 计费口径=每回合全量重发历史，端点报的 prompt 即「当前上下文」的
            // 精确测量（区别于 usage_acc 的整场累计：累计是流量、水位是快照，
            // 语义不可混用——业主实证累计被误读成上下文）。取最后一步（本轮
            // 结束时的最新水位）。
            if let Some(step_usage) = &aggregated.usage {
                stats.context = Some(step_usage.prompt_tokens);
            }
            // token 累计（云核口径：任一步端点未报 → 整体 None，不部分估算）
            stats.usage_acc = crate::turn_steps::accumulate_usage(stats.usage_acc, aggregated.usage);
            let has_calls = aggregated.has_tool_calls();
            // 空回复不闭环（task-49③，判定在 Aggregated::is_empty_response）：
            // 零产出静默 Ok=落空 assistant 装死（业主实证收尾形态）；转可重
            // 试错误，重试耗尽后 Err 上浮=人话提示，绝不静默。
            if !has_calls && aggregated.is_empty_response() {
                return Err(yunyou_llm::LlmFailure::stream_interrupted(
                    "模型本轮无任何输出（正文与思考皆空）".to_string(),
                )
                .into_cordis());
            }
            // L-9（五席评审 20261117）：finish_reason=length=端点合法截断
            // （max tokens），半截正文此前当完整回复落盘（诚实性缺陷）。
            // into_message 消费聚合体，截断判定必须在其之前取出。
            let output_truncated = aggregated.finish_reason.as_deref() == Some("length");
            let message = aggregated.into_message();
            assistant_text = message.content.clone();
            if !has_calls {
                // 收尾分支抽 turn_steps::finish_assistant（行数清偿批 task-22）；
                // output_truncated=L-9 截断标记入参
                return self
                    .finish_assistant(
                        message.content,
                        message.reasoning,
                        &mut history,
                        &observer,
                        tool_calls_total,
                        &stats,
                        output_truncated,
                    )
                    .await;
            }
            // 工具步：先落盘 assistant（含调用声明），再逐个执行与落盘
            // （persist/run_tool_step 抽 turn_steps，行数清偿批 task-22，零行为变更）
            self.persist_assistant_message(&message.content, message.reasoning.as_deref(), &mut history, &observer).await?;
            for call in &message.tool_calls {
                if call.name.is_empty() {
                    continue;
                }
                self.run_tool_step(call, &cancel, &mut history, &observer).await?;
                tool_calls_total += 1;
            }
            // 步末护栏判定（步计数在完整闭合后递增——配对完整性，见循环头注释）
            steps_used += 1;
            // K-16① 预警段（先触先警：步数与 token 独立判定；一次性注入但
            // 模型未收尾则每步末重复提示——每次注入均落盘=上下文可见）
            let step_ratio = self
                .max_steps
                .map(|limit| steps_used as f64 / limit as f64)
                .unwrap_or(0.0);
            let token_ratio = match (self.token_budget, &stats.usage_acc) {
                (Some(budget), Some(usage)) => {
                    (usage.prompt_tokens + usage.completion_tokens) as f64 / budget as f64
                }
                _ => 0.0,
            };
            let warned_ratio = self.budget_warn_ratio.clamp(0.0, 1.0);
            let in_warn = step_ratio >= warned_ratio || token_ratio >= warned_ratio;
            let reached = step_ratio >= 1.0 || token_ratio >= 1.0;
            if in_warn && !reached {
                let remain = ((1.0 - step_ratio.max(token_ratio)) * 100.0) as u64;
                let warn = SessionEvent::SystemMessage {
                    text: format!(
                        "预算余约 {remain}%——请开始收尾：增量落盘已完成成果（写文件）、输出阶段总结、申报剩余工作（未完成项列清单）。"
                    ),
                };
                self.persist(&warn).await?;
                history.push(warn);
            }
            // 触达段：auto_winddown=注入收尾指令+auto_resume 标记（调用方自动续
            // 收尾回合）；人工模式=现软着陆语义不变
            if reached {
                if self.auto_winddown {
                    let winddown = SessionEvent::SystemMessage {
                        text: "预算触达——请立即收尾：输出阶段总结+申报剩余工作（未完成项列清单）。系统将自动续一个收尾回合（宽限步硬顶）。".into(),
                    };
                    self.persist(&winddown).await?;
                    history.push(winddown);
                    let outcome = TurnOutcome {
                        assistant_text,
                        tool_calls: tool_calls_total,
                        auto_resume: true,
                        wind_down: false,
                        truncated: true,
                    };
                    emit(
                        &observer,
                        crate::observer::TurnEvent::TurnComplete {
                            outcome: outcome.clone(),
                            prompt_tokens: stats.usage_acc.as_ref().map(|u| u.prompt_tokens),
                            completion_tokens: stats.usage_acc.as_ref().map(|u| u.completion_tokens),
                            context_tokens: stats.context,
                            context_window: self.context_window, // task-27 透传（K-16 融合补位）
                        },
                    )
                    .await;
                    return Ok(outcome);
                }
                // 触达段 reason 判别（回归合并分支不得丢——79f29da M 件半成品
                // 收口）：步数先触=StepsExhausted；预算先触=BudgetExhausted。
                let reason = if step_ratio >= 1.0 {
                    crate::observer::TurnPauseReason::StepsExhausted
                } else {
                    crate::observer::TurnPauseReason::BudgetExhausted
                };
                return self
                    .finish_paused(
                        reason,
                        assistant_text,
                        tool_calls_total,
                        &mut history,
                        &observer,
                        &stats,
                    )
                    .await;
            }
        }
    }

    /// 一次模型调用：流式聚合（思考增量不丢弃，进会话）；
    /// 观察者在场时逐增量即时转发 AssistantDelta/ReasoningDelta（过程态，先于落盘）。
    /// pub(crate)：重试包装 [`crate::retry::stream_once_with_retry`] 以此为重发单元。
    pub(crate) async fn stream_once(
        &self,
        request: GenerateRequest,
        cancel: &CancellationToken,
        observer: &Option<Arc<dyn crate::observer::TurnObserver>>,
    ) -> Result<Aggregated, CordisError> {
        let mut stream = self.llm.stream_cancellable(request, cancel.clone()).await?;
        let mut aggregated = Aggregated::default();
        loop {
            let delta = tokio::select! {
                next = stream.next() => next?,
                _ = cancel.cancelled() => {
                    return Err(CordisError::new(ErrorKind::Internal, "loop/cancelled"));
                }
            };
            match delta {
                Some(delta) => {
                    match &delta {
                        StreamDelta::Text(piece) => emit(
                            observer,
                            crate::observer::TurnEvent::AssistantDelta { text: piece.clone() },
                        )
                        .await,
                        StreamDelta::Reasoning(piece) => emit(
                            observer,
                            crate::observer::TurnEvent::ReasoningDelta { text: piece.clone() },
                        )
                        .await,
                        _ => {}
                    }
                    aggregated = aggregated.fold(delta);
                }
                None => return Ok(aggregated),
            }
        }
    }

    /// 落盘（会话写失败即本轮失败——事实源不能有静默缺口）。
    /// pub(crate)：turn_steps.rs 分步方法共用（行数清偿批 task-22）。
    pub(crate) async fn persist(&self, event: &SessionEvent) -> Result<(), CordisError> {
        self.sessions.append(event).await
    }
}

/// 事件转发（观察者缺席时零开销 no-op）。
///
/// 在 async 上下文内直接 await 观察者 future：**顺序严格保证**（与对话流程
/// 同一任务、同一次序）；观察者契约要求快速返回，重活自行 spawn。
pub(crate) async fn emit(
    observer: &Option<Arc<dyn crate::observer::TurnObserver>>,
    event: crate::observer::TurnEvent,
) {
    if let Some(obs) = observer {
        obs.on_event(event).await;
    }
}
