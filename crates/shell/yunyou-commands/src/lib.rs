//! # yunyou-commands —— 命令注册缝（任务29 C1，主板加强项）
//!
//! 命令体系三层边界（docs/架构/命令体系设计.md §一）中的**主板面**：
//! 注册表/目录查询/分发入口。命令注册是宿主 UI 缝基础设施——任何 UI 适配器
//!（CLI 补全面板/未来 App 面板）都要「列目录+分发」，与 Context 服务注入、
//! 事件五分发同级；它不承载任何业务语义（业务在各命令的 handler 里），
//! 故属主板加强项而非业务功能。
//!
//! ## 三条实现纪律（设计 §一/§九硬边界）
//! 1. **命令面零进模型**：handler 在宿主执行，参数与结果不进模型请求、
//!    不占工具白名单（本 crate 与 agent-loop 无依赖边，结构上不可达）；
//! 2. **注册即 effect**：[`CommandsService::register`] 返回 [`yunyou::Disposer`]，
//!    注册方应将其登记为本 fiber 的 effect（与 ctx.effect 同语义）——
//!    fiber 卸载即从目录除名，热插拔不留残尸；
//! 3. **同名 fail-loud**：同名重复注册返回 `Err(ServiceDuplicate)` 并点名命令名，
//!    严禁静默覆盖（防注册面互相遮蔽）。
//!
//! ## 消费方式
//! 实现经 [`CommandsPlugin`] 以 `dyn CommandsService` 服务注入；要 register
//! 命令的插件在 manifest `inject` 声明该服务键，内核 epoch 机制保证本服务
//! Active 后再装载（清单顺序不构成正确性依赖）。

pub mod plugin;
pub mod registry;

pub use plugin::CommandsPlugin;
pub use registry::CommandRegistry;

use std::sync::Arc;
use yunyou::Result;
use yunyou_util::BoxFut;

/// 命令二分（设计 §二【决策点 D1】已裁）：
/// - [`CommandKind::Action`]：handler 在宿主执行、结果文本上屏（/help /quit /new 类）；
/// - [`CommandKind::Expand`]：handler 不执行副作用，产出「输入框回填文本」，
///   由用户确认后作为普通用户消息发送——技能命令归此类
///   （否则「技能进命令」与「命令不进模型」两约束相抵）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// 动作类：执行即生效，结果文本上屏。
    Action,
    /// 展开类：零副作用，产出回填文本交用户确认。
    Expand,
}

impl CommandKind {
    /// 展示名（面板/日志用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Expand => "expand",
        }
    }
}

/// 命令调用入参（handler 的宿主执行环境）。
///
/// 一期最小面：只带原始输入行。command_id 的铸造与 command-run/command-done
/// 落盘由执行方（CLI 分发层）负责——会话日志是纯配对审计（设计 §五），
/// 不归 handler 职责。
#[derive(Debug, Clone)]
pub struct CommandInvocation {
    /// 用户原始输入行（含斜杠前缀与参数部分）。
    pub raw_input: String,
}

/// 命令结算结果（会话日志 command-done 的 kind 语义，设计 §五）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResultKind {
    /// 成功：动作类 text 上屏；展开类 text 回填输入框待确认。
    Success,
    /// 失败：text 为人可读错误说明。
    Error,
}

/// handler 返回：结算结果 + 文本（上屏或回填，用途由命令 kind 决定）。
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// 结算结果。
    pub kind: CommandResultKind,
    /// 动作类=上屏文本；展开类=回填文本；失败=错误说明。
    pub text: String,
}

impl CommandOutput {
    /// 成功结果。
    pub fn success(text: impl Into<String>) -> Self {
        Self { kind: CommandResultKind::Success, text: text.into() }
    }

    /// 失败结果。
    pub fn error(text: impl Into<String>) -> Self {
        Self { kind: CommandResultKind::Error, text: text.into() }
    }
}

/// 命令 handler：宿主侧执行（不进模型），异步闭包。
pub type CommandHandler =
    Arc<dyn Fn(CommandInvocation) -> BoxFut<'static, yunyou::Result<CommandOutput>> + Send + Sync>;

/// 命令定义（注册入参，设计 §二）。
///
/// `name` 不带斜杠；`aliases` 支持中文名（/帮助 /退出 /新建，设计 §三）。
#[derive(Clone)]
pub struct CommandDefinition {
    /// 命令名（不带斜杠，如 `help`；精确名，解析优先级最高）。
    pub name: String,
    /// 别名（可含中文；解析顺序=精确 name → aliases，别名不遮蔽任何精确名）。
    pub aliases: Vec<String>,
    /// 面板/帮助文案里的描述（一行）。
    pub description: String,
    /// 动作类/展开类（D1 二分）。
    pub kind: CommandKind,
    /// 是否接受参数（task-57，D1 契约 v1.1 加法演进）：false=Action 类多余参数
    /// 触发 38⑤「未接受参数已忽略」提示；true=参数整体传 handler（如 /sessions <序号>）。
    /// 展开类（技能）参数属回填载荷，此位不影响其语义。
    pub accepts_args: bool,
    /// 宿主执行的 handler。
    pub handler: CommandHandler,
}

impl std::fmt::Debug for CommandDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandDefinition")
            .field("name", &self.name)
            .field("aliases", &self.aliases)
            .field("description", &self.description)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// 命令目录条目（list 的只读投影，面板消费；**不含 handler**——目录是数据面）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDescriptor {
    /// 命令名（不带斜杠）。
    pub name: String,
    /// 别名列表。
    pub aliases: Vec<String>,
    /// 一行描述。
    pub description: String,
    /// 动作类/展开类。
    pub kind: CommandKind,
    /// 是否接受参数（task-57；register 时自 Definition 拷贝，38⑤ 提示层豁免开关）。
    pub accepts_args: bool,
}

/// 命令注册缝服务（object-safe，经 ctx 服务注入，设计 §二）。
///
/// 方法面=注册（register）+目录（list/find）+分发（handler_of）：
/// 设计 §一「注册表/目录查询/分发」三职责的兑现——find 给面板数据面，
/// handler_of 给执行方（同一解析规则），CommandDescriptor 保持无 handler 纯投影。
pub trait CommandsService: Send + Sync {
    /// 注册命令；同名已存在 → `Err(ServiceDuplicate)` 点名命令名（fail-loud）。
    ///
    /// 返回的 Disposer 执行即从目录除名（幂等）；注册方应将其登记为本 fiber
    /// 的 effect（fiber 卸载自动除名，与 ctx.effect 同语义）。
    fn register(&self, def: CommandDefinition) -> Result<yunyou::Disposer>;

    /// 全量目录：name 字典序稳定输出（提示词/面板可复现，设计 §二）。
    fn list(&self) -> Vec<CommandDescriptor>;

    /// 解析入口（设计 §三别名规则）：精确 name 优先，其次 aliases；
    /// 别名不遮蔽任何精确名。
    fn find(&self, token: &str) -> Option<CommandDescriptor>;

    /// 分发入口：按 find 同款解析规则取 handler（设计 §一表格「分发」职责）。
    /// 未知命令/别名 → None（调用方提示，不进任何日志——官方语义：语法/未知名
    /// 未进 handler 即不落日志）。
    fn handler_of(&self, token: &str) -> Option<CommandHandler>;
}
