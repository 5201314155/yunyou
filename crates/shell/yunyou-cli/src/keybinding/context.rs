//! 键与 context（交互矩阵 A1 第二层+A2 解析器）。

/// 一条键组合（修饰键+主键；字符串解析单一入口，平台差异收常量见 reserved）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// 主键（小写归一：'c'、'tab'、'escape'、'enter'、'up'…）。
    pub key: String,
}

impl KeyCombo {
    /// 解析 "ctrl+shift+tab" 形态（claude 键名字面同源；未知名原样保留
    /// fail-safe——解析失败=查不到绑定，不 panic）。
    pub fn parse(spec: &str) -> Self {
        let (mut ctrl, mut shift, mut alt) = (false, false, false);
        let mut key = String::new();
        for part in spec.split('+') {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" | "meta" => alt = true,
                other => key = other.to_string(),
            }
        }
        Self { ctrl, shift, alt, key }
    }
}

/// 键位活跃 context（A2：同一键在不同 context 行为不同而互不误触）。
/// claude 13 context 全集锚=〈claude-交互键位行为提取〉§一 1；本批落四态
/// （与 yunyou 现有四态对齐，盘点 §0），其余 context 随域演进只加变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// 兜底层（claude Global）——任何态都生效的最小集。
    Global,
    /// 编辑态（提示符输入）。
    Chat,
    /// 面板开（斜杠补全候选）。
    Panel,
    /// 确认对话框（批 7 权限）。
    Confirmation,
}
