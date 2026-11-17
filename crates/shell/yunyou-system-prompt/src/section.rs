//! 提示段数据模型：段名、段序、段文本、互斥标记（契约③，方案 §3③）。
//!
//! 对标官方 `core/system-prompt/src/index.ts`：`PromptSection`（:56-79）与
//! `SECTION_ORDERS` 段位全表（:136-160）。

use crate::error::SystemPromptError;
use crate::interpolate::scan_malformed;

/// 宿主自有段名：身份自述（对标官方 `harness:identity`，:457-461）。
pub const SECTION_HARNESS_IDENTITY: &str = "harness:identity";
/// 段位表唯一的人设槽：宿主默认注册、运行期可被同名替换（对标官方
/// `PERSONA_PREFIX_SECTION = 'deployment:persona-prefix'`，:189-200——官方注释明说
/// 「同名正是替换而非重复」的机制依据）。
pub const SECTION_PERSONA_PREFIX: &str = "deployment:persona-prefix";
/// 工具动态段名：宿主持有、每轮由 assemble 合成（不进注册表，见 assemble.rs）。
pub const SECTION_TOOLS: &str = "harness:tools";

/// 段位（段位表，对标官方 `SECTION_ORDERS`，:136-160）：集中分配的排序位，
/// 组件注册新段应选取档位之间的空隙（如 150＝自定义策略段），不得挤占他人档位。
pub const ORDER_HARNESS_IDENTITY: f64 = -1000.0;
/// 人设槽档位。
pub const ORDER_PERSONA_PREFIX: f64 = 0.0;
/// 工具段档位（官方 TOOL_* 自 1000 起；工具段为宿主动态段，不进注册表，见 assemble.rs）。
pub const ORDER_TOOLS: f64 = 1000.0;

/// 一个系统提示段。
///
/// 段名是**槽位键**：同名注册＝替换（shadow）而非重复，见 `crate::registry` 的语义注释。
///
/// 与官方 `PromptSection`（index.ts:53-79）的一期裁剪（注记备查）：`interpolate?:
/// boolean`（整段关插值）与 provider 型 text（`(context) => string`）未纳入——
/// 一期文本恒静态、恒插值；需要字面 `{{` 时写成无配对 `}}` 的孤立散文（见
/// `crate::interpolate` 模块文档）。二期扩字段不破段名槽位语义。
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// 段名（槽位键），非空。
    pub name: String,
    /// 段序：升序拼接的依据；必须是有限数，相等时按段名字典序（对标官方 :58-61）。
    pub order: f64,
    /// 段文本，可含 `{{variable}}` 引用（未注册变量＝空串，见 `crate::interpolate`）。
    pub text: String,
    /// 互斥标记：本段即**完整**系统提示词，其余段让位；同时多个生效＝装配失败
    /// （对标官方 `complete?: boolean`，:70-74）。
    pub complete: bool,
}

impl Section {
    /// 普通段构造（非互斥）。
    pub fn new(name: impl Into<String>, order: f64, text: impl Into<String>) -> Self {
        Self { name: name.into(), order, text: text.into(), complete: false }
    }

    /// 标记为互斥段（`complete`）。
    pub fn complete(mut self) -> Self {
        self.complete = true;
        self
    }

    /// 装载前/注册前校验：段名非空、段序有限、文本无畸形变量引用（对标官方 :479-492）。
    pub fn validate(&self) -> Result<(), SystemPromptError> {
        if self.name.trim().is_empty() {
            return Err(SystemPromptError::EmptySectionName);
        }
        if !self.order.is_finite() {
            return Err(SystemPromptError::NonFiniteOrder { name: self.name.clone() });
        }
        scan_malformed(&self.text)
            .map_err(|found| SystemPromptError::MalformedVariable { section: self.name.clone(), found })
    }
}