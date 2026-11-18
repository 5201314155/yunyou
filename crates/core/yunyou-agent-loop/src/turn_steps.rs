//! 工具循环分步执行体（自 turn.rs run_turn_inner 按职责拆出，行数清偿批
//! task-22；纯搬移零行为变更）：usage 累计/assistant 落盘/单工具步/两个收尾。
//! 方法落 [`AgentLoopRunner`] 的跨文件 impl 块（同 crate，字段本就 pub）。

use crate::turn::{emit, AgentLoopRunner, TurnOutcome};
use crate::observer::TurnPauseReason;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use yunyou::CordisError;
use yunyou_llm::Usage;
use yunyou_session::SessionEvent;

type Observer = Arc<dyn crate::observer::TurnObserver>;

/// 回合 token 计量二元组（task-26 收参：usage_acc 累计流量+context_tokens 水位
/// 快照同源同消费（同出每步 usage、同进 TurnComplete），捆扎语义自然——
/// task-51⑤ ToolState 同款收参先例；finish_* 加 context 位后 8/7 触发
/// too_many_arguments，捆扎后回到 7 上限内）。
pub(crate) struct TurnTokenStats {
    /// 整场累计（任一步未报→None，云核口径）。
    pub usage_acc: Option<Usage>,
    /// 当前上下文水位（最后一步 usage.prompt；task-26）。
    pub context: Option<u64>,
}

/// token 累计（云核口径：任一步端点未报 → 整体 None，不部分估算）。
pub(crate) fn accumulate_usage(acc: Option<Usage>, step: Option<Usage>) -> Option<Usage> {
    match (acc, step) {
        (Some(acc), Some(step)) => Some(Usage {
            prompt_tokens: acc.prompt_tokens + step.prompt_tokens,
            completion_tokens: acc.completion_tokens + step.completion_tokens,
        }),
        (None, Some(step)) => Some(step),
        (_, None) => None,
    }
}

impl AgentLoopRunner {
    /// assistant 消息落盘+入账本+发事件（无工具调用收尾与工具步两处共用；
    /// 原 run_turn_inner 两段逐行同形代码收敛，字节语义等价）。
    pub(crate) async fn persist_assistant_message(
        &self,
        text: &str,
        reasoning: Option<&str>,
        history: &mut Vec<SessionEvent>,
        observer: &Option<Observer>,
    ) -> Result<(), CordisError> {
        let event = SessionEvent::AssistantMessage {
            text: text.to_string(),
            reasoning: reasoning.map(str::to_string),
        };
        self.persist(&event).await?;
        history.push(event);
        emit(
            observer,
            crate::observer::TurnEvent::AssistantMessage {
                text: text.to_string(),
                reasoning: reasoning.map(str::to_string),
            },
        )
        .await;
        Ok(())
    }

    /// 无工具调用收尾（原 run_turn_inner 收尾分支，行数清偿批 task-22 拆出）：
    /// assistant 落盘 → Outcome → TurnComplete（context_tokens=task-26 水位
    /// 快照，语义见 observer 变体 doc）。
    /// L-9（五席评审 20261117）：finish_reason=length 时补截断标记——
    /// SystemMessage 人话注记落盘（模型与用户都看得到「话没说完」）+
    /// TurnOutcome.truncated=true（程序化消费面，语义对齐护栏触达截断位）。
    #[allow(clippy::too_many_arguments)] // L-9 加参后 8 参触顶；收参重构（stats 捆扎扩位）语义不合，留注解
    pub(crate) async fn finish_assistant(
        &self,
        text: String,
        reasoning: Option<String>,
        history: &mut Vec<SessionEvent>,
        observer: &Option<Observer>,
        tool_calls_total: usize,
        stats: &TurnTokenStats,
        output_truncated: bool,
    ) -> Result<TurnOutcome, CordisError> {
        self.persist_assistant_message(&text, reasoning.as_deref(), history, observer).await?;
        if output_truncated {
            let note = SessionEvent::SystemMessage {
                text: "⚠ 上一条回复因达到输出长度上限被截断（finish_reason=length），内容可能不完整；如需继续请让模型接着说。".to_string(),
            };
            self.persist(&note).await?;
            history.push(note);
        }
        // 回合末水位回写（task-27：跨回合触发判定的供数链）
        self.compaction_state.set_watermark(stats.context);
        let outcome = TurnOutcome {
            assistant_text: text,
            tool_calls: tool_calls_total,
            truncated: output_truncated,
            auto_resume: false, // 正常完成：无护栏触达（K-16① 语义，REPL 人工模式恒 false）
            wind_down: false,
        };
        emit(
            observer,
            crate::observer::TurnEvent::TurnComplete {
                outcome: outcome.clone(),
                prompt_tokens: stats.usage_acc.as_ref().map(|u| u.prompt_tokens),
                completion_tokens: stats.usage_acc.as_ref().map(|u| u.completion_tokens),
                context_tokens: stats.context,
                context_window: self.context_window,
            },
        )
        .await;
        Ok(outcome)
    }

    /// 悬空 ToolCall 恢复兜底（L-3，五席评审 20261117）：回合起点扫描会话，
    /// 对「有 ToolCall 无配对 ToolResult」的残留（ToolResult 落盘失败/进程
    /// 两 persist 之间被杀所致）补合成 is_error ToolResult 落盘——否则下一轮
    /// 请求「assistant 声明了 tool_calls 但无对应 tool 消息」，严格端点整轮
    /// 400 拒收，会话实质卡死。合成件持久化（事实源补齐，渲染层可见）+入
    /// 账本+发 ToolResult 事件（事件层，cli 渲染零改动即呈现）。
    pub(crate) async fn heal_dangling_tool_calls(
        &self,
        history: &mut Vec<SessionEvent>,
        observer: &Option<Observer>,
    ) -> Result<(), CordisError> {
        // 先收集悬空调用（call_id 顺序=落盘顺序，输出稳定）：ToolCall 入列，
        // 同 id 的 ToolResult 出列，扫完剩下即悬空
        let mut open: Vec<(String, String)> = Vec::new();
        for event in history.iter() {
            match event {
                SessionEvent::ToolCall { call_id, name, .. } => {
                    open.push((call_id.clone(), name.clone()));
                }
                SessionEvent::ToolResult { call_id, .. } => {
                    open.retain(|(id, _)| id != call_id);
                }
                _ => {}
            }
        }
        for (call_id, name) in open {
            let content = "会话中断于该工具调用的执行（落盘失败或进程中断），未收到执行结果。请重新发起该调用。".to_string();
            let event = SessionEvent::ToolResult {
                call_id: call_id.clone(),
                name: name.clone(),
                content: content.clone(),
                is_error: true,
                parent_tool_call_id: None,
                batch_id: None,
            };
            self.persist(&event).await?;
            history.push(event);
            emit(
                observer,
                crate::observer::TurnEvent::ToolResult {
                    call_id,
                    name,
                    summary: crate::observer::summarize(&content, 160),
                    content: serde_json::json!(content),
                    exit_code: None,
                    is_error: true,
                    parent_tool_call_id: None,
                    batch_id: None,
                },
            )
            .await;
        }
        Ok(())
    }

    /// 单个工具调用全步（原 run_turn_inner for 循环体，行数清偿批 task-22 拆出）：
    /// ToolCall 落盘+发事件 → 执行 → ToolResult 落盘+canonical 发事件。
    /// persist 失败上抛（本轮失败语义不变）。
    pub(crate) async fn run_tool_step(
        &self,
        call: &yunyou_llm::ToolCallRequest,
        cancel: &CancellationToken,
        history: &mut Vec<SessionEvent>,
        observer: &Option<Observer>,
    ) -> Result<(), CordisError> {
        let event = SessionEvent::ToolCall {
            call_id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            // 一期为顺序执行：树字段与批次标识留空，并行执行落地时填充
            parent_tool_call_id: None,
            batch_id: None,
        };
        self.persist(&event).await?;
        history.push(event);
        emit(
            observer,
            crate::observer::TurnEvent::ToolCall {
                call_id: call.id.clone(),
                name: call.name.clone(),
                arguments_summary: crate::observer::summarize(&call.arguments, 120),
                parent_tool_call_id: None,
                batch_id: None,
            },
        )
        .await;
        let outcome = self.run_tool(call, cancel).await;
        let (content, is_error) = yunyou_tools::executor::outcome_parts(&outcome);
        let exit_code = match &outcome {
            yunyou_tools::ToolOutcome::Ok(value) => {
                value.get("exit_code").and_then(|code| code.as_i64())
            }
            _ => None,
        };
        let event = SessionEvent::ToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content: content.clone(),
            is_error,
            parent_tool_call_id: None,
            batch_id: None,
        };
        self.persist(&event).await?;
        history.push(event);
        // 事件侧带 canonical value（渲染适配层解析详情块）；超 8KB 降级为
        // 首 3 行预览+折叠计数（任务30 三期②，fold 模块），完整无损版本
        // 始终在会话 JSONL
        let canonical = match &outcome {
            yunyou_tools::ToolOutcome::Ok(value) => {
                if serde_json::to_vec(value).map(|bytes| bytes.len() > 8 * 1024).unwrap_or(true) {
                    serde_json::Value::String(crate::fold::fold_preview(value))
                } else {
                    value.clone()
                }
            }
            yunyou_tools::ToolOutcome::Err(err) => {
                serde_json::json!({ "code": err.code, "message": err.message })
            }
        };
        emit(
            observer,
            crate::observer::TurnEvent::ToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                summary: crate::observer::summarize(&content, 160),
                content: canonical,
                exit_code,
                is_error,
                parent_tool_call_id: None,
                batch_id: None,
            },
        )
        .await;
        Ok(())
    }

    /// 护栏软着陆收尾（task-23：原「步数腰斩」finish_truncated 统一改造，
    /// 业主实证令+云枢三点裁决）：结构化事件 TurnPaused + SystemMessage 落盘
    /// （文案「已到达本轮 X 上限，回复『继续』即可无缝延续」——腰斩文案
    /// 「已达上限…本轮终止」零残留）→ Outcome（truncated=true，语义=护栏
    /// 暂停未跑完）→ TurnComplete。上下文完整保留（SystemMessage 已入账本
    /// 落盘），用户回复「继续」=新回合从完整历史续跑，天然无缝——无特殊
    /// 续跑机制即语义诚实（上下文在，续跑天然成立）。
    pub(crate) async fn finish_paused(
        &self,
        reason: TurnPauseReason,
        assistant_text: String,
        tool_calls_total: usize,
        history: &mut Vec<SessionEvent>,
        observer: &Option<Observer>,
        stats: &TurnTokenStats,
    ) -> Result<TurnOutcome, CordisError> {
        let message = match &reason {
            TurnPauseReason::StepsExhausted => format!(
                "已到达本轮 {} 步上限，回复「继续」即可无缝延续。",
                self.max_steps.unwrap_or_default()
            ),
            TurnPauseReason::BudgetExhausted => format!(
                "已到达本轮 token 预算 {} 上限，回复「继续」即可无缝延续。",
                self.token_budget.unwrap_or_default()
            ),
        };
        let event = SessionEvent::SystemMessage { text: message.clone() };
        self.persist(&event).await?;
        history.push(event);
        emit(
            observer,
            crate::observer::TurnEvent::TurnPaused { reason, message },
        )
        .await;
        // 回合末水位回写（task-27：软着陆路径同样供数——「继续」续跑回合头可判）
        self.compaction_state.set_watermark(stats.context);
        let outcome = TurnOutcome {
            assistant_text,
            tool_calls: tool_calls_total,
            truncated: true,
            auto_resume: false, // 软着陆暂停：REPL 人工模式恒 false（K-16①），成员 worker 续跑由调用方侧驱动
            wind_down: false,
        };
        emit(
            observer,
            crate::observer::TurnEvent::TurnComplete {
                outcome: outcome.clone(),
                prompt_tokens: stats.usage_acc.as_ref().map(|u| u.prompt_tokens),
                completion_tokens: stats.usage_acc.as_ref().map(|u| u.completion_tokens),
                context_tokens: stats.context,
                context_window: self.context_window,
            },
        )
        .await;
        Ok(outcome)
    }

    /// 执行单个工具调用（取消令牌沿用本轮；自 turn.rs 搬入，语义原样）。
    async fn run_tool(
        &self,
        call: &yunyou_llm::ToolCallRequest,
        cancel: &CancellationToken,
    ) -> yunyou_tools::ToolOutcome {
        let Some(tool) = self.tools.get(&call.name) else {
            return yunyou_tools::ToolOutcome::err(
                "tool/unknown",
                format!("未知工具 \"{}\"", call.name),
            );
        };
        // 审批缝（task-16 S1）：执行前 gate 链判定（第一个非 Pass 短路；
        // 空注册零开销）。Ask→呈现器（对象类型注册制）；无呈现器/呈现拒绝=
        // fail-closed Deny。内核零规则知识：判定全在策略插件侧。
        if !self.gates.gates.is_empty() {
            let view = crate::gate::ToolCallView { name: &call.name, arguments: &call.arguments };
            let ctx = crate::gate::GateContext::default();
            match self.gates.gates.check(&view, &ctx).await {
                crate::gate::GateDecision::Pass
                | crate::gate::GateDecision::Allow { .. } => {}
                crate::gate::GateDecision::Deny { message } => {
                    return yunyou_tools::ToolOutcome::err("tool/denied", message);
                }
                crate::gate::GateDecision::Ask { .. } => {
                    match self.gates.approval.present(crate::gate::ApprovalSubject::ToolCall(&view)).await {
                        Some(crate::gate::ApprovalDecision::Approved) => {}
                        Some(crate::gate::ApprovalDecision::Denied { message }) => {
                            return yunyou_tools::ToolOutcome::err("tool/denied", message);
                        }
                        None => {
                            return yunyou_tools::ToolOutcome::err(
                                "tool/denied",
                                "工具调用待审批，但审批呈现器未注册（无头场景 fail-closed）：已拒绝执行。",
                            );
                        }
                    }
                }
            }
        }
        // L-10（五席评审 20261117）：畸形 arguments 不再静默 {} 兜底执行——
        // 那样工具收空参大概率 is_error，而原样 JSON 文本已落盘、下轮原样回发
        // 端点，严格端点 400 不重试=会话自锁。此处落人话错误 ToolResult 给模型
        // 指路（修正参数重试）；回发面由 request.rs 归一为 {}（组请求侧同批）。
        let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
            Ok(value) => value,
            Err(err) => {
                return yunyou_tools::ToolOutcome::err(
                    "tool/bad-arguments",
                    format!(
                        "工具「{}」的参数不是合法 JSON（{err}），未执行。请检查引号/括号是否配对完整后重新发起调用。",
                        call.name
                    ),
                );
            }
        };
        yunyou_tools::executor::execute_tool(tool, args, call.id.clone(), cancel.clone(), self.tool_timeout).await
    }
}
