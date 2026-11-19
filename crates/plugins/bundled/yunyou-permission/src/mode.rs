//! 模式层（task-16 S3，设计稿 §四交互矩阵）：模式不改规则匹配，只变换
//! 「无命中/Ask」的去向（claude 后置变换器同律，:502-513/:604-647）。
//!
//! # 词汇（对齐 yun-bo 模式体系骨架附录 SessionMode 四态）
//! `default` / `accept_edits`（兼容 `acceptEdits` 驼峰）/ `plan` / `bypass`。
//! GateContext.mode 为等价字符串字段（附录第 1 条「不改 trait 形状」口径）；
//! SessionMode crate 落地后换型（加法演进点注记）。

/// 权限模式（矩阵列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 缺省：写类区内免审/区外审；执行类审；读类放行。
    Default,
    /// 编辑放行：写类区内免审（与 default 同格但语义位保留给「更宽编辑面」
    /// 演进——v1 矩阵中与 default 只差执行类外视野，矩阵不变式锁）。
    AcceptEdits,
    /// 计划：恒只读——写/执行一律问。
    Plan,
    /// 旁路：不问我（deny 规则/L0 免疫不豁免——矩阵不变式 3）。
    Bypass,
}

impl Mode {
    /// 解析（kebab/驼峰兼容；未知=None→调用方按 Default 处理+warn）。
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "accept_edits" | "acceptEdits" => Some(Self::AcceptEdits),
            "plan" => Some(Self::Plan),
            "bypass" => Some(Self::Bypass),
            _ => None,
        }
    }
}

/// 工具类别（矩阵行分类；v1 按工具名单源——新工具随注册面演进在此处加行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    /// 只读（read/glob/grep/web_fetch/skill_read 等）。
    Read,
    /// 写类（write/edit 系——落盘副作用）。
    Write,
    /// 执行类（bash 系——任意副作用面）。
    Exec,
}

/// 工具名→类别（单源表；未知工具=Read 保守？不——未知工具按 Exec 审最保守：
/// 看不见副作用面的工具宁可多问）。
pub(crate) fn tool_class(tool: &str) -> ToolClass {
    match tool {
        "write" | "edit" | "multi_edit" => ToolClass::Write,
        "bash" | "shell" => ToolClass::Exec,
        // read/glob/grep/web_fetch/skill_read/todo_write/team_* 等已知只读/中性
        "read" | "glob" | "grep" | "web_fetch" | "web_search" | "skill_read" | "todo_write" => {
            ToolClass::Read
        }
        // 未知工具=执行类保守审（缺省 Ask，任何模式非 bypass 都会问）
        _ => ToolClass::Exec,
    }
}

/// 模式×无命中判定（矩阵「无命中」三行；规则命中态由 gate.rs 前置处理——
/// 本函数只答规则未命中时的去向）。
///
/// 入参：模式/工具类别/工作区内（write 类传入路径判定结果；root 未知=区外
/// 保守——「工作区外要审批」在 root 不可知时宁问勿放）。
pub(crate) fn decide_unmatched(mode: Mode, class: ToolClass, in_workspace: Option<bool>) -> Unmatched {
    match mode {
        Mode::Bypass => Unmatched::Allow,
        Mode::Plan => match class {
            ToolClass::Read => Unmatched::Allow,
            ToolClass::Write | ToolClass::Exec => Unmatched::Ask,
        },
        Mode::AcceptEdits | Mode::Default => match class {
            ToolClass::Read => Unmatched::Allow,
            ToolClass::Write => match in_workspace {
                Some(true) => Unmatched::Allow,   // 区内写免审（业主定死语义）
                Some(false) | None => Unmatched::Ask, // 区外/root 未知=审
            },
            ToolClass::Exec => Unmatched::Ask,
        },
    }
}

/// 无命中去向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmatched {
    /// 放行（不产生决策——gate 返回 Pass/Allow）。
    Allow,
    /// 问人。
    Ask,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 词汇解析：kebab/驼峰/未知。
    #[test]
    fn 模式解析() {
        assert_eq!(Mode::parse("default"), Some(Mode::Default));
        assert_eq!(Mode::parse("accept_edits"), Some(Mode::AcceptEdits));
        assert_eq!(Mode::parse("acceptEdits"), Some(Mode::AcceptEdits));
        assert_eq!(Mode::parse("plan"), Some(Mode::Plan));
        assert_eq!(Mode::parse("bypass"), Some(Mode::Bypass));
        assert_eq!(Mode::parse("yolo"), None);
    }

    /// 工具分类：写/执行/读/未知保守。
    #[test]
    fn 工具分类() {
        assert_eq!(tool_class("write"), ToolClass::Write);
        assert_eq!(tool_class("edit"), ToolClass::Write);
        assert_eq!(tool_class("bash"), ToolClass::Exec);
        assert_eq!(tool_class("read"), ToolClass::Read);
        assert_eq!(tool_class("web_fetch"), ToolClass::Read);
        assert_eq!(tool_class("神奇新工具"), ToolClass::Exec, "未知工具保守审");
    }

    /// 矩阵「无命中」三行逐格（设计稿 §四表尾三行）。
    #[test]
    fn 无命中矩阵逐格() {
        use ToolClass::*;
        // default：区内写 Allow/区外写 Ask/执行 Ask/读 Allow
        assert_eq!(decide_unmatched(Mode::Default, Write, Some(true)), Unmatched::Allow);
        assert_eq!(decide_unmatched(Mode::Default, Write, Some(false)), Unmatched::Ask);
        assert_eq!(decide_unmatched(Mode::Default, Exec, None), Unmatched::Ask);
        assert_eq!(decide_unmatched(Mode::Default, Read, None), Unmatched::Allow);
        // acceptEdits：同 default 格（区内写 Allow/区外 Ask/执行 Ask）
        assert_eq!(decide_unmatched(Mode::AcceptEdits, Write, Some(true)), Unmatched::Allow);
        assert_eq!(decide_unmatched(Mode::AcceptEdits, Write, Some(false)), Unmatched::Ask);
        assert_eq!(decide_unmatched(Mode::AcceptEdits, Exec, None), Unmatched::Ask);
        // plan：恒只读——写/执行 Ask、读 Allow
        assert_eq!(decide_unmatched(Mode::Plan, Write, Some(true)), Unmatched::Ask, "plan 区内写也问");
        assert_eq!(decide_unmatched(Mode::Plan, Exec, None), Unmatched::Ask);
        assert_eq!(decide_unmatched(Mode::Plan, Read, None), Unmatched::Allow);
        // bypass：全 Allow
        assert_eq!(decide_unmatched(Mode::Bypass, Write, Some(false)), Unmatched::Allow);
        assert_eq!(decide_unmatched(Mode::Bypass, Exec, None), Unmatched::Allow);
        // root 未知：写类宁问勿放（业主「工作区外要审批」的保守读法）
        assert_eq!(decide_unmatched(Mode::Default, Write, None), Unmatched::Ask);
    }
}
