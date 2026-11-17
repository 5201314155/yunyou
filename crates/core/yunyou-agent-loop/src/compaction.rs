//! 上下文压缩执行体（task-27 一期；设计蓝本 docs/设计/上下文管理设计-yunyou.md
//! §7）：摘要请求 → Compaction 事件落盘 → 熔断计数。独立成件=压缩职责域
//! （摘要/模板/熔断）与回合主控（turn.rs）、分步执行体（turn_steps.rs）并列，
//! impl [`AgentLoopRunner`] 跨文件（同 crate）。
//!
//! # 时序承重墙（设计稿 §7 云枢实现硬标 S3）
//! 压缩动作在**回合头**（当轮 UserMessage 落盘之前）触发——若压缩晚于
//! UserMessage 落盘，「最后一个 compaction」分割会把本轮用户消息切进摘要区
//! （刚发的原话降级为摘要转述=丢题变体）。跨回合水位经
//! [`CompactionState::last_watermark`]（上回合末回写，Arc 跨回合共享）——
//! 实现细节修正声明：设计稿「回合头」的触发检查+压缩一体在回合头完成，
//! 触发回合的 load_all 为压缩专用读（+1 次），io_count 默认关断言不受扰。

use crate::turn::{emit, AgentLoopRunner};
use yunyou::CordisError;
use yunyou_session::SessionEvent;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio_util::sync::CancellationToken;

/// 压缩会话级状态（熔断计数；跨回合存续——Arc 挂 runner 共享，进程生命周期
/// 内有效；重启后计数归零=环境可能已修复，重新给机会）。
pub struct CompactionState {
    /// 摘要连续失败计数（成功清零；/compact 手动成功清零）。
    pub failures: AtomicU32,
    /// 熔断态（连败达 compaction_failure_limit 置位；本会话停用 auto-compact）。
    pub disabled: AtomicBool,
    /// 上回合末水位（回合末回写、回合头触发判定读——跨回合传递；std Mutex
    /// 短临界区，锁内不 await）。
    pub last_watermark: std::sync::Mutex<Option<u64>>,
}

impl CompactionState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            failures: AtomicU32::new(0),
            disabled: AtomicBool::new(false),
            last_watermark: std::sync::Mutex::new(None),
        })
    }

    /// 回合末回写水位（finish_* 消费点调用）。
    pub fn set_watermark(&self, watermark: Option<u64>) {
        *self.last_watermark.lock().expect("水位锁") = watermark;
    }

    /// 回合头读水位（快照，锁内取值即放）。
    pub fn watermark(&self) -> Option<u64> {
        *self.last_watermark.lock().expect("水位锁")
    }
}

/// 摘要指令模板（设计稿 §12 定稿；temperature：GenerateRequest 一期无该位，
/// 忠实性由模板指令承担——llm 契约加法演进候后批，如实申报不静默缺位）。
const COMPACTION_PROMPT: &str = "你是会话压缩器。将以下对话历史压缩为一份摘要，供后续对话作为唯一的前文记忆。\n\
必须保留（按优先级）：\n\
1. 用户的任务目标与最新指示（逐字保留关键指令）；\n\
2. 已完成/进行中/未开始的工作项与各自状态；\n\
3. 关键决定与理由（为什么这么做）；\n\
4. 涉及的文件路径、命令、错误信息原文（后续操作要用的精确串）；\n\
5. 未解决的问题与下一步计划。\n\
丢弃：寒暄、重复内容、已被后续版本取代的中间状态。\n\
输出为纯摘要正文（无寒暄无解释），按上述五类组织，总长不超过 2000 字。";

/// 待压缩段文本化（最后一次 Compaction 的摘要以「〔此前压缩摘要〕」形态注入
/// +其后事件——多轮链：第二摘要的输入**必须含第一摘要**（否则多轮压缩=渐进
/// 失忆，V5 断言一锁死）；首次压缩（无 Compaction）=全量。
pub(crate) fn render_history_for_summary(history: &[SessionEvent]) -> String {
    let mut out = String::new();
    let start = match history
        .iter()
        .rposition(|e| matches!(e, SessionEvent::Compaction { .. }))
    {
        Some(pos) => {
            let Some(SessionEvent::Compaction { summary, .. }) = history.get(pos) else {
                unreachable!("rposition 命中即 Compaction");
            };
            out.push_str(&format!("〔此前压缩摘要〕{summary}\n"));
            pos + 1
        }
        None => 0,
    };
    for event in &history[start..] {
        match event {
            SessionEvent::UserMessage { text } => {
                out.push_str(&format!("用户：{text}\n"));
            }
            SessionEvent::AssistantMessage { text, .. } => {
                out.push_str(&format!("助手：{text}\n"));
            }
            SessionEvent::ToolCall { name, arguments, .. } => {
                out.push_str(&format!("工具调用：{name}({arguments})\n"));
            }
            SessionEvent::ToolResult { name, content, is_error, .. } => {
                let tag = if *is_error { "失败" } else { "完成" };
                out.push_str(&format!("工具结果（{name}，{tag}）：{content}\n"));
            }
            SessionEvent::SystemMessage { text } => {
                out.push_str(&format!("系统：{text}\n"));
            }
            // 更早的 Compaction 不出现（rposition 只取最后一个；其前段已由
            // 该摘要承载——链式语义见上）。命令审计词汇不进摘要（与
            // build_request 消费面同口径）。
            _ => {}
        }
    }
    out
}

impl AgentLoopRunner {
    /// 触发线（窗口 − 摘要输出预留 − 安全余量；窗口未配 = None = 不触发）。
    pub(crate) fn compaction_trigger(&self) -> Option<u64> {
        self.context_window.map(|window| {
            window
                .saturating_sub(self.compact_reserved_output)
                .saturating_sub(self.compact_safety_margin)
        })
    }

    /// 是否应自动压缩（旋钮与/或：auto_compact_enabled ∧ 窗口在场 ∧ 未熔断 ∧
    /// 水位 ≥ 触发线；水位 None=无测量数据不触发——首个回合/usage 未报）。
    pub(crate) fn should_auto_compact(&self, watermark: Option<u64>) -> bool {
        if !self.auto_compact_enabled || self.compaction_state.disabled.load(Ordering::Relaxed) {
            return false;
        }
        let Some(trigger) = self.compaction_trigger() else { return false };
        watermark.is_some_and(|w| w >= trigger)
    }

    /// 执行压缩：待压缩段文本化 → 摘要请求（复用 stream_once——同一超时/取消
    /// 体系，observer=None 静默）→ 非空校验 → persist(Compaction) → push 账本
    /// → emit Compacted → 熔断计数清零。persist 失败上抛（事实源不能有静默
    /// 缺口——设计稿 §7 铁律：不因压缩破例）。
    pub(crate) async fn run_compaction(
        &self,
        history: &mut Vec<SessionEvent>,
        cancel: &CancellationToken,
        pre_tokens: u64,
        observer: &Option<Arc<dyn crate::observer::TurnObserver>>,
    ) -> Result<(), CordisError> {
        let rendered = render_history_for_summary(history);
        if rendered.trim().is_empty() {
            return Err(CordisError::new(
                yunyou::ErrorKind::Internal,
                "compact/待压缩段为空（无可摘要内容）",
            ));
        }
        let request = yunyou_llm::GenerateRequest {
            model: String::new(),
            messages: vec![
                yunyou_llm::Message::text(yunyou_llm::Role::System, COMPACTION_PROMPT.to_string()),
                yunyou_llm::Message::text(
                    yunyou_llm::Role::User,
                    format!("【对话历史】\n{rendered}"),
                ),
            ],
            tools: vec![],
            // 摘要显式 0.0（task-33；设计稿 §12 伴注：忠实不创造——压缩后
            // 摘要是唯一前文记忆，保真度实质）。
            temperature: Some(0.0),
        };
        let aggregated = self.stream_once(request, cancel, &None).await?;
        if aggregated.is_empty_response() {
            return Err(CordisError::new(
                yunyou::ErrorKind::Internal,
                "compact/摘要请求返回空（模型零产出）",
            ));
        }
        let summary = aggregated.text;
        let summary_len = summary.len();
        let event = SessionEvent::Compaction { summary, pre_tokens };
        self.persist(&event).await?;
        history.push(event);
        // 渲染面「〔已自动压缩〕」提示（透明性：压缩改变后续对话记忆边界，
        // 必须可见——设计稿 §7.3；走回合主 observer，None 时零开销 no-op）。
        emit(
            observer,
            crate::observer::TurnEvent::Compacted { pre_tokens, summary_len },
        )
        .await;
        self.compaction_state.failures.store(0, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_session::SessionEvent;

    /// V5 断言一（云枢裁量）：多轮链——最后一次 Compaction 之后的事件才进
    /// 摘要输入，此前摘要以「〔此前压缩摘要〕」形态链式注入（防渐进失忆）。
    #[test]
    fn 多轮链_第二摘要输入含第一摘要() {
        let history = vec![
            SessionEvent::UserMessage { text: "第一轮任务".into() },
            SessionEvent::Compaction { summary: "第一摘要：任务已完成大半".into(), pre_tokens: 100 },
            SessionEvent::UserMessage { text: "第二轮任务".into() },
        ];
        let rendered = render_history_for_summary(&history);
        assert!(rendered.contains("〔此前压缩摘要〕第一摘要：任务已完成大半"), "第一摘要链式注入: {rendered}");
        assert!(rendered.contains("第二轮任务"), "压缩点后事件正常进: {rendered}");
        assert!(!rendered.contains("第一轮任务"), "压缩点前事件不进（只认最后一个 compaction）: {rendered}");
    }

    /// V5 断言二（build_request 只认最后）：两 compaction 场景下请求只含
    /// 第二摘要前缀+其后事件，不含第一摘要原文与压缩点前消息。
    #[test]
    fn build_request_只认最后一个compaction() {
        use crate::request::compose_request;
        struct P;
        impl yunyou_system_prompt::SystemPrompt for P {
            fn compose(&self, _persona: &str, _tools: &[yunyou_llm::ToolSchema]) -> String {
                "SYS".into()
            }
            fn register_section(
                &self,
                _: yunyou_system_prompt::Section,
            ) -> Result<yunyou::Disposer, yunyou_system_prompt::SystemPromptError> {
                unreachable!("测试不触达")
            }
        }
        struct R;
        impl yunyou_tools::RegistryService for R {
            fn register_tool(self: Arc<Self>, _: Arc<dyn yunyou_tools::ToolDefinition>) -> Result<yunyou::Disposer, yunyou::CordisError> { unreachable!() }
            fn schemas(&self) -> Vec<yunyou_llm::ToolSchema> { vec![] }
            fn get(&self, _: &str) -> Option<Arc<dyn yunyou_tools::ToolDefinition>> { None }
            fn timeout_ms(&self, _: &str) -> Option<u64> { None }
            fn is_concurrency_safe(&self, _: &str) -> bool { false }
            fn presentation_meta(&self, _: &str) -> Option<yunyou_tools::contract::PresentationMeta> { None }
        }
        let history = vec![
            SessionEvent::UserMessage { text: "早期消息".into() },
            SessionEvent::Compaction { summary: "摘要一".into(), pre_tokens: 100 },
            SessionEvent::UserMessage { text: "中期消息".into() },
            SessionEvent::Compaction { summary: "摘要二".into(), pre_tokens: 200 },
            SessionEvent::UserMessage { text: "最新消息".into() },
        ];
        let req = compose_request(&R, &P, "", &history);
        assert_eq!(req.messages.len(), 3, "system+摘要注入+最新消息（压缩点前全丢）");
        assert!(req.messages[1].content.contains("摘要二"), "注入=最后一个摘要");
        assert!(!req.messages[1].content.contains("摘要一"), "第一摘要原文不进（链式内容在第二摘要内）: {:?}", req.messages[1].content);
        assert!(req.messages[2].content.contains("最新消息"));
    }
}
