//! 一轮对话的过程事件外露（task-10 A 线）：观察者 trait + mpsc 桥。
//!
//! # 通道形态（定案，2026-11-13 报池卡）
//! **观察者 trait（callback）为主通道，`observer_channel()` 提供 mpsc 桥**：
//! - CLI 线性流式渲染（云鉴规范：ANSI 直出顺序打印）直接实现 trait，在回调里
//!   写终端，无需管理 channel 生命周期；
//! - 需要多消费方或任务化消费的场景，用 [`observer_channel`] 一行拿到
//!   `(Arc<dyn TurnObserver>, UnboundedReceiver<TurnEvent>)` 桥接。
//!
//! # 事件与 JSONL 的同源关系
//! - [`TurnEvent::UserMessage`] / [`TurnEvent::ToolCall`] / [`TurnEvent::ToolResult`] /
//!   [`TurnEvent::AssistantMessage`] 与落盘的 [`yunyou_session::SessionEvent`] 一一对应，
//!   且**在落盘成功之后才发出**（渲染看到的事件必然已持久化）；
//! - [`TurnEvent::AssistantDelta`] / [`TurnEvent::ReasoningDelta`] 是流式**过程态**
//!   （JSONL 不落盘，落盘的是聚合后的 AssistantMessage），在聚合过程中即时发出；
//! - ToolCall/ToolResult 事件携带树归属字段（parent_tool_call_id/batch_id，
//!   与 SessionEvent 同源；一期顺序执行为 None，渲染端组树接口就绪）。
//!
//! # 契约
//! - [`TurnObserver::on_event`] 会被**同步 await**：实现应快速返回
//!   （重活自行 spawn），否则拖慢对话流；
//! - 事件按时间序发出；观察者不会收到乱序或重放；
//! - trait object 安全；实现必须 Send + Sync。

use yunyou_util::BoxFut;
use std::sync::{Arc, Mutex};

/// 一轮对话的过程事件。
#[derive(Debug, Clone)]
pub enum TurnEvent {
    /// 用户输入已受理并落盘。
    UserMessage { text: String },
    /// 助手可见文本增量（流式过程态；完整文本由 AssistantMessage 事件承载）。
    AssistantDelta { text: String },
    /// 思考增量（推理模型 reasoning_content；渲染端可折叠显示）。
    ReasoningDelta { text: String },
    /// 模型发起工具调用（分片聚合完成后发一次；带树归属）。
    ToolCall {
        call_id: String,
        name: String,
        /// 参数摘要（JSON 文本截断至 120 字符，防刷屏；完整参数看会话 JSONL）。
        arguments_summary: String,
        parent_tool_call_id: Option<String>,
        batch_id: Option<String>,
    },
    /// 工具执行结果（带树归属）。
    ToolResult {
        call_id: String,
        name: String,
        /// 内容摘要（截断至 160 字符；完成卡用；详情渲染用 [`TurnEvent::content`]）。
        summary: String,
        /// 工具 canonical value 原样（渲染适配层解析详情块，如 Bash{exit_code,output}、
        /// Read{path,lines}、Diff{added,removed}）。**上限 8KB**（序列化体积，超限降级为
        /// String 提示）——完整无损版本看会话 JSONL；DetailKind 解析属 cli 适配层辖区。
        content: serde_json::Value,
        /// 退出码（工具 canonical value 携带 exit_code 时提取；无则 None）。
        exit_code: Option<i64>,
        is_error: bool,
        parent_tool_call_id: Option<String>,
        batch_id: Option<String>,
    },
    /// 助手消息已落盘（每步结束；流式增量的聚合终态）。
    AssistantMessage {
        text: String,
        reasoning: Option<String>,
    },
    /// 本轮完成（outcome 与返回值同源）。
    TurnComplete {
        outcome: super::turn::TurnOutcome,
        /// prompt token 累计（多步求和；任一步端点未报则 None——渲染端状态落款行用）。
        prompt_tokens: Option<u64>,
        /// completion token 累计（语义同上）。耗时由 cli 接线层自行计时，A 线不背。
        completion_tokens: Option<u64>,
        /// 当前上下文水位（task-26，加法演进）：最后一步 API usage 的 prompt
        /// 值——LLM 计费口径=每回合全量重发历史，prompt 即「当前上下文 token
        /// 数」的端点侧测量（区别于 prompt_tokens 的整场累计——累计是
        /// 流量、水位是快照，两者语义不可混用；业主实证：累计被误读成上下文）。
        /// 精度边界：水位快照=**发送时点**实测，未含本回合 assistant 回复的
        /// completion（其下回合才计入历史）——即略低于回合收尾真实水位一条
        /// 回复的量（与 Claude Code 口径一致）。usage 未报时 None（诚实不估
        /// 算）。「距压缩阈值百分比」形态候 task-27 auto-compact 落地（有阈
        /// 值才有分母）。
        context_tokens: Option<u64>,
        /// 模型上下文窗口（task-27 加法演进：[agent_loop] context_window 透传，
        /// 供落款「X% until auto-compact」联动——None=未配置窗口，渲染面不显
        /// 示百分比）。供数侧=runner 配置直传（非测量值）。
        context_window: Option<u64>,
    },
    /// 上下文压缩完成（task-27 一期，加法演进）：渲染面「〔已自动压缩〕」提示
    /// +程序化消费。压缩改变后续对话的记忆边界，必须可见（透明性）。手动
    /// /compact 与 auto 触发共用本事件。
    Compacted {
        /// 压缩前水位（该次压缩时的上下文 token 数）。
        pre_tokens: u64,
        /// 摘要字符数（观测：摘要体量级）。
        summary_len: usize,
    },
    /// 护栏软着陆暂停（task-23，加法演进；终态语义见 [`TurnPauseReason`]）：
    /// 上下文完整保留（暂停 SystemMessage 已落盘），用户回复「继续」即以完整
    /// 历史无缝续跑——非腰斩、非失败。渲染端可静默（SystemMessage 事件已承载
    /// 人话文案，本变体供程序化消费方结构化读取）。
    TurnPaused {
        /// 触发的护栏类别。
        reason: TurnPauseReason,
        /// 人话文案（与落盘 SystemMessage 同文；「已到达本轮 X 上限，回复
        /// 『继续』即可无缝延续」形态）。
        message: String,
    },
}

/// 护栏触发原因（task-23 终止语义对齐三家参照系：Claude Code 主循环 maxTurns
/// 可选默认不传=无上限、Codex 预算面、DSH TS 零命中上限——主循环默认无步数
/// 硬顶，终=模型停手/取消/错误三路；护栏两旋钮默认全关，显式拧上即同意暂停
/// 语义，触及时统一软着陆可续，禁腰斩截肢）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnPauseReason {
    /// 步数护栏（`[agent_loop] max_steps`，API 面对齐 Claude Code maxTurns）。
    StepsExhausted,
    /// token 预算护栏（`[agent_loop] token_budget`，对齐 Codex 预算面；
    /// usage 任一步未报→整体 None 时本护栏对本轮失效——诚实不估算）。
    BudgetExhausted,
}

/// 事件观察者 trait（渲染端实现；对象安全）。
pub trait TurnObserver: Send + Sync {
    /// 处理一个事件。会被同步 await——快速返回，重活自行 spawn。
    fn on_event(&self, event: TurnEvent) -> BoxFut<'_, ()>;

    /// LLM 重试调度通知（任务18 超时重试，加法演进默认 no-op——不实现即
    /// 零感知；attempt 从 1 起，cause 为 llm/<code> 失败类别，D10 裁决：
    /// observer/日志层不落 JSONL）。
    /// 【恢复留痕】本方法由云绘二任按 retry.rs 调用面与 tests/retry.rs
    /// override 签名补回（2026-11-14 15:5x 四期同步 cp 误覆盖在途改动，
    /// 事故已报 lead），请任务18 承办席复核完整性。
    fn on_retry_scheduled(&self, _attempt: u32, _delay_ms: u64, _cause: &str) -> BoxFut<'_, ()> {
        Box::pin(async {})
    }
}

/// 无操作观察者（占位实现）。
pub struct NopObserver;

impl TurnObserver for NopObserver {
    fn on_event(&self, _event: TurnEvent) -> BoxFut<'_, ()> {
        Box::pin(async {})
    }
}

/// mpsc 桥：把事件流入通道的观察者。
struct ChannelObserver {
    sender: tokio::sync::mpsc::UnboundedSender<TurnEvent>,
}

impl TurnObserver for ChannelObserver {
    fn on_event(&self, event: TurnEvent) -> BoxFut<'_, ()> {
        // 无界通道 + 发送失败仅意味着接收端已弃（不影响对话主流程）
        let _ = self.sender.send(event);
        Box::pin(async {})
    }
}

/// 建一对「观察者 + 无界接收端」：需要 channel 形态消费时的桥。
///
/// 接收端关闭（drop）后事件安全丢弃——观察者桥不因消费方退出而打断对话。
pub fn observer_channel() -> (Arc<dyn TurnObserver>, tokio::sync::mpsc::UnboundedReceiver<TurnEvent>) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    (Arc::new(ChannelObserver { sender }), receiver)
}

/// 事件收集器（测试与调试用）：把事件按序存入共享缓冲。
#[derive(Default)]
pub struct CollectingObserver {
    events: Mutex<Vec<TurnEvent>>,
}

impl CollectingObserver {
    /// 建收集器（返回 Arc 供观察者与读取两侧使用）。
    pub fn new() -> Arc<Self> {
        Arc::new(Self { events: Mutex::new(Vec::new()) })
    }

    /// 取快照（按到达顺序）。
    pub fn snapshot(&self) -> Vec<TurnEvent> {
        self.events.lock().expect("事件缓冲锁").clone()
    }
}

impl TurnObserver for CollectingObserver {
    fn on_event(&self, event: TurnEvent) -> BoxFut<'_, ()> {
        self.events.lock().expect("事件缓冲锁").push(event);
        Box::pin(async {})
    }
}

/// 文本摘要：截断至 `max_chars` 字符（字符边界安全），超限补省略标记。
/// 摘要恒单行（任务30 四期 D-B）：先压平连续空白——render_outcome 落盘
/// 形态为 pretty 多行 JSON（contract.rs 人读友好），LLM 参数也可能带换行；
/// 多行文本进完成卡即糊屏（业主截图 `{ "count": 200, …` 多行倾泻）。压平
/// 只影响摘要预览，无损版始终在会话 JSONL。
pub(crate) fn summarize(text: &str, max_chars: usize) -> String {
    let mut flat = String::with_capacity(text.len());
    let mut prev_blank = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_blank {
                flat.push(' ');
                prev_blank = true;
            }
        } else {
            flat.push(ch);
            prev_blank = false;
        }
    }
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let cut: String = flat.chars().take(max_chars).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 摘要恒单行（任务30 四期 D-B）：pretty 多行 JSON 压平为单行，
    /// 完成卡不再被 `{ "count": 200, …` 多行倾泻糊屏。
    #[test]
    fn 摘要压平多行json() {
        let pretty = "{\n  \"count\": 200,\n  \"matches\": [\n    \"a.rs\"\n  ]\n}";
        let out = summarize(pretty, 160);
        assert!(!out.contains('\n'), "摘要不得含换行：{out:?}");
        assert!(out.starts_with("{ \"count\": 200, \"matches\": ["), "压平保序：{out:?}");
    }

    /// 摘要截断的字符边界安全（多字节不裂开）与省略标记。
    #[test]
    fn 摘要截断安全() {
        assert_eq!(summarize("短文本", 10), "短文本");
        let long = "中".repeat(50);
        let out = summarize(&long, 10);
        assert!(out.starts_with(&"中".repeat(10)));
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), 11);
    }

    /// 收集器观察者按序收集。
    #[tokio::test]
    async fn 收集器按序() {
        let collector = CollectingObserver::new();
        collector
            .on_event(TurnEvent::UserMessage { text: "hi".into() })
            .await;
        collector.on_event(TurnEvent::AssistantDelta { text: "你".into() }).await;
        let snapshot = collector.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert!(matches!(snapshot[0], TurnEvent::UserMessage { .. }));
        assert!(matches!(snapshot[1], TurnEvent::AssistantDelta { .. }));
    }

    /// mpsc 桥：观察者发送 → 接收端按序收到。
    #[tokio::test]
    async fn 桥接通道() {
        let (observer, mut receiver) = observer_channel();
        observer.on_event(TurnEvent::UserMessage { text: "a".into() }).await;
        observer.on_event(TurnEvent::AssistantDelta { text: "b".into() }).await;
        drop(observer);
        let first = receiver.recv().await.unwrap();
        assert!(matches!(first, TurnEvent::UserMessage { .. }));
        let second = receiver.recv().await.unwrap();
        assert!(matches!(second, TurnEvent::AssistantDelta { .. }));
        assert!(receiver.recv().await.is_none(), "发送端关闭后通道收束");
    }
}
