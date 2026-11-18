//! 主循环组件（方案 §2.5，对标 TS agent-loop 的「对话一轮」主路径裁剪）。
//!
//! # 一轮对话的步骤
//! 1. 用户输入落盘（user-message）；
//! 2. 组请求：会话历史 → 消息序列（system prompt + 工具声明）；
//! 3. `llm.stream` 流式聚合：文本 / 思考 / 工具调用分片按 index 聚合；
//! 4. 无工具调用 → assistant-message 落盘，本轮结束；
//! 5. 有工具调用 → 逐个 tool-call 落盘 → 执行器跑工具 → tool-result 落盘
//!    → 回到 2（主循环无步数硬顶，task-23 对齐三家参照系：跑到模型停手，
//!    防跑飞=取消/超时/错误三路）；
//! 6. 护栏（[agent_loop] max_steps/token_budget，默认全关）触达 → 软着陆：
//!    system-message「回复『继续』无缝延续」+TurnPaused 事件+正常返回
//!    （truncated=true，上下文完整保留可续——禁腰斩截肢）。
//!
//! # 契约
//! - 依赖注入：inject = [llm, sessions, tools, systemPrompt]（对标
//!   agent-loop/src/index.ts:291，agents 一期并入 loop，方案 §2.5）；
//! - 会话是**唯一事实源**：落盘成功的事件才进入下一轮请求；
//! - 工具失败不中断本轮（tool_result is_error=true 回传模型自行决策），
//!   仅 LLM 流失败 / 会话写失败终止本轮（错误上抛调用方）。

mod aggregate;
mod cancel;
mod compaction;
mod fold;
mod gate;
mod hooks;
mod observer;
mod plugin;
mod request;
mod retry;
mod turn;
mod turn_steps;

pub use observer::{
    observer_channel, CollectingObserver, NopObserver, TurnEvent, TurnObserver, TurnPauseReason,
};
pub use plugin::{AgentLoop, AgentLoopConfig, AgentLoopPlugin};
pub use compaction::CompactionState;
pub use hooks::{
    apply_edits as apply_compose_edits, ComposeEdit, HooksRegistry, HooksRegistryService,
    PreComposeHook, PreComposeRequest,
};
pub use gate::{
    ApprovalDecision, ApprovalPresenter, ApprovalSubject, ApprovalSubjectKind, ApprovalUiService,
    GateContext, GateDecision, GateHub, GateRegistryService, GateRegistry, PreToolUseGate,
    ToolCallView,
};
pub use turn::{AgentLoopRunner, TurnOutcome};
