//! 渲染层的事件对接面（任务 task-10 B 线）。
//!
//! 形状与设计规范 §4 映射表、会话事件（yunyou-session）及 A 线外露事件
//! （yunyou-agent-loop，云织接口先行中）对齐：通道落地后由适配层把
//! 云织事件映射为本枚举，渲染层只认这里（适配隔离，通道形态无关）。

use std::time::Duration;

/// 一轮对话中 agent-loop 外露的渲染事件（B 线只消费不产生）。
#[derive(Debug, Clone)]
pub enum TurnEvent {
    /// assistant 文本增量（规范 §4.5 流式回复）。
    TextDelta { text: String },
    /// 思考内容增量（reasoning，规范外补块——弱化灰调，见 stream.rs）。
    ReasoningDelta { text: String },
    /// 工具调用开始（规范 §4.6；树字段见 §4.8，缺省为平铺兼容）。
    ToolCall {
        call_id: String,
        name: String,
        /// 参数摘要（已按工具规则裁好，烟灰 48 列内）。
        summary: String,
        parent_tool_call_id: Option<String>,
        batch_id: Option<String>,
    },
    /// 工具结果（规范 §4.7 完成卡；detail 按 [`DetailKind`] 展开渲染）。
    ToolResult {
        call_id: String,
        /// 工具名（完成卡粗体）。
        name: String,
        ok: bool,
        /// 结果摘要（成功：行数/项数/耗时；失败：退出码+错误首行）。
        summary: String,
        /// 结果详情（折叠块数据；None = 只落定行不展开）。
        detail: Option<DetailKind>,
        parent_tool_call_id: Option<String>,
    },
    /// 回合完结（规范 §4.12 状态落款，本地聚合）。
    TurnDone {
        elapsed: Duration,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        /// 当前上下文水位（task-26：最后一步 usage.prompt——端点侧测量，区别
        /// 于 prompt_tokens 累计流量；None=usage 未报。语义见 agent-loop
        /// observer::TurnEvent::TurnComplete::context_tokens）。
        context_tokens: Option<u64>,
        /// 模型上下文窗口（task-27 联动：落款百分比/距压缩显示——None=未配置
        /// 窗口，维持 task-26 形态）。
        context_window: Option<u64>,
        model: String,
    },
    /// 回合错误（对运维可见，绯红行）。
    Error { message: String },
}

/// tool_result 详情的展示形态（规范 §4.9–§4.11 分流）。
#[derive(Debug, Clone)]
pub enum DetailKind {
    /// bash 输出（exit 0 → 折叠摘要；非 0 → stderr 尾部 ≤5 行直接展示）。
    Bash { exit_code: Option<i32>, output: String },
    /// 文件读取（≤10 行全量带行号；>10 行首 3 行 + … +N 行）。
    Read { path: String, lines: Vec<String>, from: usize },
    /// edit/write 的 diff（+/− 着色，上下文折叠）。
    Diff { added: usize, removed: usize, hunks: Vec<DiffHunk> },
    /// 其余工具：逐行纯文本（最多 10 行）。
    Plain { lines: Vec<String> },
}

/// 一个 diff 块（规范 §4.11：上下文 ±2 行折叠）。
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// 文件归属（批 2-T2a：apply_patch 多文件补丁按 hunk 标路径，渲染为
    /// hunk 前的 ▎ path 行；edit 单文件语义=None 不加行）。
    pub path: Option<String>,
    /// 变更区旧文起始行号（1-based，批 5 gutter 数据面；None=旧 canonical
    /// 无行号→gutter 退位，窄终端防御同源）。
    pub old_start: Option<usize>,
    /// 增行（含 `+` 前缀渲染由渲染层加）。
    pub added: Vec<String>,
    /// 删行。
    pub removed: Vec<String>,
    /// 上下文行数（渲染为 ⋮ 上下文 N 行）。
    pub context_lines: usize,
}
