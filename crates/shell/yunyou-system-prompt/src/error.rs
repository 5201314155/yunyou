//! 契约③ 段机制的错误类型（对标官方 system-prompt/src/index.ts：order 非有限抛、
//! 同层重名抛、complete 多处即装配失败、畸形变量引用抛）。

use std::fmt;

/// 段注册/装配/插值三类失败。一律**装载期或装配期显式抛出**，不静默吞掉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemPromptError {
    /// 段名为空或全空白（段名是槽位键，不能为空）。
    EmptySectionName,
    /// 段序不是有限数（对标官方 `Number.isFinite(section.order)` 检查，:479-492）。
    NonFiniteOrder { name: String },
    /// 段文本含畸形 `{{...}}` 引用（`{{}}`、含空格/大写/花括号嵌套等）。
    MalformedVariable { section: String, found: String },
    /// 互斥段（`complete`）同时有多个生效（对标官方「more than one effective complete section makes assembly fail」）。
    CompleteConflict { names: Vec<String> },
    /// 变量名不合法（对标官方 `VARIABLE_NAME = /^[a-z][a-z0-9_]*$/`，:203）。
    InvalidVariableName { name: String },
}

impl fmt::Display for SystemPromptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySectionName => write!(f, "提示段名不能为空（段名即槽位键）"),
            Self::NonFiniteOrder { name } => {
                write!(f, "提示段「{name}」的段序必须是有限数（非 NaN/非无穷）")
            }
            Self::MalformedVariable { section, found } => write!(
                f,
                "提示段「{section}」含畸形变量引用「{found}」：引用必须是完整的 {{name}} 组，\
                 且 name 匹配 ^[a-z][a-z0-9_]*$"
            ),
            Self::CompleteConflict { names } => write!(
                f,
                "互斥提示段（complete）同时生效 {} 个：{}；最多只允许一个",
                names.len(),
                names.join("、")
            ),
            Self::InvalidVariableName { name } => write!(
                f,
                "变量名「{name}」不合法：必须匹配 ^[a-z][a-z0-9_]*$（小写字母开头，可含数字与下划线）"
            ),
        }
    }
}

impl std::error::Error for SystemPromptError {}