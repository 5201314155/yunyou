//! 动作名注册表（交互矩阵 A1 第一层，lead 批 2026-11-19）。
//!
//! 键位=「动作名→键」映射而非「键→if 分支」（claude schema.ts:236 同构）：
//! 动作名是一等公民，新增面板=注册新动作+挂 context，不改任何既有分支。
//! 全集锚=〈claude-交互键位行为提取-20261119〉§一/二；本批只落注册面
//! （动作枚举+默认绑定+解析器），**不接线任何既有键处理路径**（零行为）——
//! lineedit/repl 既有分支归编辑行批（yun-ya），迁移另行立票。

/// 用户可触发的交互动作（动作名=注册表唯一键；演进只加不改）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // —— Global 层（claude defaultBindings Global 块）——
    /// 中断当前（ctrl+c 单击；双击语义由调用方计时，见 reserved 纪律）。
    Interrupt,
    /// 退出（ctrl+c/ctrl+d 双击）。
    Exit,
    /// 重绘（ctrl+l，终端花屏自救）。
    Redraw,
    /// transcript 全屏只读视图（ctrl+o；q 退出）。
    ToggleTranscript,
    /// 历史搜索（ctrl+r）。
    HistorySearch,

    // —— Chat/编辑层 ——
    /// 提交（enter）。
    Submit,
    /// 中断/取消（escape）。
    Cancel,
    /// 双击 Esc（空输入）=回合回滚选择器（rewind，对接 task-26 rewind 批）。
    RewindPicker,
    /// 权限模式循环（shift+tab，前进不回退）。
    CycleMode,
    /// 历史 up/down。
    HistoryPrev,
    HistoryNext,
    /// 输入暂存（ctrl+s，切话题不丢草稿；【超】候选）。
    Stash,

    // —— Panel（斜杠补全面板）——
    /// 补全循环（tab）。
    CompleteNext,
    /// 面板候选上下移（up/down）。
    PanelPrev,
    PanelNext,
    /// 执行选中项（enter）。
    PanelExec,
    /// 收面板（escape）。
    PanelClose,

    // —— Confirmation 层（批 7 权限对话框直连）——
    /// 是（y/enter）。
    ConfirmYes,
    /// 否（n/escape）。
    ConfirmNo,
    /// 永远允许（a；经 suggestions 档位）。
    ConfirmAlways,
    /// 对话框内循环模式（shift+tab；【超】审批疲劳自救，键位缝先注册）。
    ConfirmCycleMode,
    /// 多建议项选择（tab=nextField、space=toggle）。
    ConfirmNextField,
    ConfirmToggle,
}

impl Action {
    /// 动作名（注册表唯一键；字符串形态供覆盖文件/日志/调试）。
    pub fn name(&self) -> &'static str {
        match self {
            Action::Interrupt => "app:interrupt",
            Action::Exit => "app:exit",
            Action::Redraw => "app:redraw",
            Action::ToggleTranscript => "app:toggleTranscript",
            Action::HistorySearch => "history:search",
            Action::Submit => "chat:submit",
            Action::Cancel => "chat:cancel",
            Action::RewindPicker => "chat:rewindPicker",
            Action::CycleMode => "chat:cycleMode",
            Action::HistoryPrev => "history:previous",
            Action::HistoryNext => "history:next",
            Action::Stash => "chat:stash",
            Action::CompleteNext => "panel:completeNext",
            Action::PanelPrev => "panel:previous",
            Action::PanelNext => "panel:next",
            Action::PanelExec => "panel:exec",
            Action::PanelClose => "panel:close",
            Action::ConfirmYes => "confirm:yes",
            Action::ConfirmNo => "confirm:no",
            Action::ConfirmAlways => "confirm:always",
            Action::ConfirmCycleMode => "confirm:cycleMode",
            Action::ConfirmNextField => "confirm:nextField",
            Action::ConfirmToggle => "confirm:toggle",
        }
    }
}
