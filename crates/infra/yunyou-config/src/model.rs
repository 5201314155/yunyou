//! yunyou.toml 的强类型模型、默认值与环境变量覆盖。
//!
//! 校验链（loader.rs）：TOML 文本 → JSON 值 → schema 校验 → 反序列化为本模型。
//! schema 由本模型的 `JsonSchema` derive 同源生成，杜绝「校验器与模型漂移」。
//! 各段字段与 3A 组件的 Config 对齐（yunyou-llm-openai/yunyou-session/yunyou-tools/
//! yunyou-system-prompt/yunyou-agent-loop），段值原样透传给 `ctx.plugin`。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use crate::credentials::CredentialsConfig;

/// LLM 段的环境变量覆盖键（联调验收约定，key 由 lead 经 env 注入，绝不入库）。
pub const ENV_LLM_BASE_URL: &str = "DSH_TEST_LLM_BASE_URL";
pub const ENV_LLM_API_KEY: &str = "DSH_TEST_LLM_API_KEY";
pub const ENV_LLM_MODEL: &str = "DSH_TEST_LLM_MODEL";

/// yunyou.toml 顶层配置（对标 cordis.yml Entry 树的一期静态裁剪，方案 §2.5）。
///
/// 每个段对应 §2.5 序列中的一个组件；yunyou-cli 装配时把段值透传给
/// `ctx.plugin(组件, 段配置)`，组件自身 schema 做深度校验。
///
/// [plugins] 段（S2，方案 §2.4；裁决 D2）是**已知段**不是未知段：
/// `deny_unknown_fields` 保留（拼写防呆不放松），插件启用表经
/// [`DshConfig::plugins`] 容器化透传——配置面不再把插件清单锁死在编译期。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DshConfig {
    /// LLM 提供商连接配置（`yunyou-llm-openai` 组件）。
    pub llm: LlmConfig,
    /// 会话存储配置（`yunyou-session` 组件）。
    pub session: SessionConfig,
    /// 系统提示词配置（`yunyou-system-prompt` 组件）。
    pub system_prompt: SystemPromptConfig,
    /// 工具配置（`yunyou-tools` 组件）。
    pub tools: ToolsConfig,
    /// 工作区根配置（task-71①：root 显式指定=git 根检测的覆盖；缺省=自动链）。
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// 主循环配置（`yunyou-agent-loop` 组件）。
    pub agent_loop: AgentLoopConfig,
    /// 重试策略配置（`[retry]` 段，任务18/S10；消费方 `yunyou-retry` 插件，
    /// 字段对齐 yunyou_llm::RetryPolicy 纯类型，设计 §5「装好即预置」）。
    pub retry: RetryConfig,
    /// 凭据托管存储配置（`yunyou-credentials` 组件，S5；类型定义见
    /// [`crate::credentials`]，段值经装配层透传）。
    pub credentials: CredentialsConfig,
    /// 沙箱配置（`[sandbox]` 段，task-24 接线；字段对齐
    /// yunyou_sandbox::SandboxPluginConfig，设计 §2 config 面）。
    #[serde(default)]
    pub sandbox: SandboxConfig,
    /// 插件启用表（`[plugins]` 段，方案 §2.4）：键=插件 id（bundled 用 crate 名）。
    /// 缺省空表=全部插件走各自 manifest.auto_enable（存量五段配置零改动可用）。
    pub plugins: PluginsConfig,
}

/// `[sandbox]` 段：沙箱开关与语义（task-24；字段对齐
/// yunyou_sandbox::SandboxPluginConfig——config 镜像惯例同 session 段）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxConfig {
    /// 沙箱总开关（false=显式降级直跑，warn 留痕）。
    #[serde(default = "default_sandbox_enabled")]
    pub enabled: bool,
    /// /tmp 与 TMPDIR 并入可写根（默认 true——bash pwd 捕获与编译器临时文件合法面）。
    #[serde(default = "default_sandbox_writable_tmp")]
    pub writable_tmp: bool,
    /// 网络禁断（默认 true；false=显式放开，重启生效非热调——设计 §2 config 面）。
    #[serde(default = "default_sandbox_network_denied")]
    pub network_denied: bool,
}

fn default_sandbox_enabled() -> bool {
    true
}
fn default_sandbox_writable_tmp() -> bool {
    true
}
fn default_sandbox_network_denied() -> bool {
    true
}

/// `[plugins]` 段条目：单插件的启用开关与段值（方案 §2.4 第 2 步：
/// 每插件 { enable: bool = manifest.auto_enable 默认, config: 段值 }）。
///
/// TOML 形态：
///
/// ```toml
/// [plugins.yunyou-tools]
/// enable = false
///
/// [plugins.yunyou-tools.config]
/// root_dir = "/tmp/workdir"
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PluginEntry {
    /// 是否启用；缺省（未写 enable 键）回落该插件 manifest.auto_enable（装配时读）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable: Option<bool>,
    /// 透传给插件的段值配置（深度校验由组件自身 config_schema 承担，方案 §2.2）。
    /// 与顶层已知段浅合并、本键优先——新机制插件（无顶层段）的唯一配置入口；
    /// 存量五段插件不写本键时行为与现状完全一致（迁移兼容，S2 验收口径）。
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub config: Value,
}

/// `[plugins]` 段：bundled 插件启用表（方案 §2.4；裁决 D2：deny_unknown_fields
/// 保留＋[plugins] 作为已知段加入 DshConfig——插件段值容器化透传）。
///
/// 段内按插件 id flatten 承载任意插件键（TOML 写作 `[plugins.<插件id>]`）；
/// 条目自身 [`PluginEntry`] deny_unknown_fields（enable/config 之外的键报错），
/// 未列出的插件按其 manifest.auto_enable 处理。未知的**插件 id**不报错：
/// 一期 market 级未实现，多写的键留待装配日志提示（不破坏存量文件）。
///
/// **本段不得加 deny_unknown_fields**（task-18 修 sed 盲扫误伤留痕）：entries
/// 用 `#[serde(flatten)]` 承载任意插件 id，flatten 与 deny 互斥——加 deny 即把
/// `[plugins.<任意id>]` 全数当未知键拒，插件启用表失效（既有用例
/// plugins_段_enable与config反序列化 实证）。条目内防呆由 PluginEntry 自身
/// deny 承担（enable/config 之外的键报错）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PluginsConfig {
    /// 插件条目表：flatten 使键即插件 id（如 "yunyou-tools"）。
    #[serde(flatten)]
    pub entries: HashMap<String, PluginEntry>,
}

/// `llm` 段：OpenAI 兼容端点连接参数（字段对齐 yunyou-llm-openai::OpenAiConfig）。
///
/// task-34⑤：deny_unknown_fields 与顶层/`[plugins]` 段对齐（拼写防呆不放松）——
/// 嵌套段此前静默忽略未知键（实测 `timeout_seconds = 0` 零警告：用户以为设了
/// 不限时，实际落回默认 300s），报错即教育；serde 报错自动列合法键。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LlmConfig {
    /// OpenAI 兼容 base_url（不含 `/chat/completions`）。
    /// 默认取台账环境事实中的本机中转端点。
    pub base_url: String,
    /// API key；留空表示端点不要求鉴权。S5 起（方案 §7 风险 7）：**推荐写凭据
    /// 引用** `$YUNYOU_LLM_API_KEY`（经凭据托管存储解析，值不落配置文件）；
    /// 明文写法为迁移期兼容——启动迁移自动入托管存储并定点改写为引用。
    /// `role=secret` 标注（schemars extend）：redact 结构性脱敏与 settings 投影
    /// 密码框标注对本字段自动生效（S8 消费面）。
    #[schemars(extend("role" = "secret"))]
    pub api_key: String,
    /// 模型名。默认取台账环境事实中的测试模型（推理模型，注意 reasoning_content 容忍）。
    pub model: String,
    /// 单次请求超时秒数（0 = 不限时）。默认 300。
    /// 语义（D-19-1 起）：仅约束**连接+收头**段（首字节前）——读流段不受它
    /// 约束（推理时长不可预算），由 `stream_idle_timeout_ms` 空闲看门狗管。
    /// 默认与 OpenAiConfig（插件 schema 面）对齐=300（D2 双 default 对齐：
    /// 旧值 120 与「默认 300 不变兼容」注释矛盾，无 [llm] 段时静默变档）。
    pub timeout_secs: u64,
    /// 流式读取空闲超时（毫秒；0 = 不设看门狗）。默认 30_000。
    /// D-19-1：chunk 间无字节超过该时长即判流死（总预算对流式响应是错误
    /// 超时语义——推理时长不可预算）。透传链：LlmConfig 序列化 → 插件 config
    /// 分发 → OpenAiConfig 同名字段（双侧 default 对齐，断言锁死）。
    pub stream_idle_timeout_ms: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8317/v1".to_string(),
            api_key: String::new(),
            model: "z-ai/glm-5.3-flash".to_string(),
            timeout_secs: 300,
            stream_idle_timeout_ms: 30_000,
        }
    }
}

/// `session` 段：会话 JSONL 文件（字段对齐 yunyou-session::SessionConfig）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// 会话 JSONL 文件路径；缺省时由 yunyou-cli 落到
    /// `<yunyou-home>/sessions/default.jsonl`（装配时补值）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// `[workspace]` 段（task-71①）：工作区根显式配置（覆盖 git 根自动检测）。
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// 工作区根显式指定（绝对或相对进程 cwd；非法=stderr 警告+回退自动链）。
    #[serde(default)]
    pub root: Option<String>,
}

/// `system_prompt` 段：提示词组装参数（字段对齐 yunyou-system-prompt::SystemPromptConfig）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SystemPromptConfig {
    /// 人设文本（worker 人设注入点，方案 §2.7.2）；缺省用云悠品牌预设
    /// （task-10：M2 口径「yunyou 编码助手」由模型自由发挥，统一为云悠自我介绍；2026-11-13 随目录改名批次统一品牌）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
}

impl Default for SystemPromptConfig {
    fn default() -> Self {
        Self { persona: Some(default_persona().to_string()) }
    }
}

/// 云悠品牌人设预设（一处字符串：改品牌口径只改这里）。
pub fn default_persona() -> &'static str {
    "你是云悠（YunYou）——终端里的编码助手，为用户把活干完。回答直接、简洁、用中文；需要时主动使用工具验证。"
}

/// `tools` 段：工具执行参数（字段对齐 yunyou-tools::ToolsConfig）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// 文件类工具的根目录（相对 cwd 或绝对路径）。
    pub root_dir: String,
    /// 内置工具开关（工具名→bool，池 task-7 业主令「工具读配置开关」）：
    /// `false` = 该工具不注册、不进模型面 schemas()（LLM 看不见=真关，非仅禁执行）。
    /// 缺省（空表）= 全开；未知工具名键由 yunyou-tools 装载期 fail-loud 报错
    /// （拼错防呆在插件侧按已知工具名清单校验，map 键无法用 deny_unknown_fields）。
    #[serde(default)]
    pub builtin: std::collections::BTreeMap<String, bool>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self { root_dir: ".".to_string(), builtin: std::collections::BTreeMap::new() }
    }
}

/// `agent_loop` 段：主循环参数（字段对齐 yunyou-agent-loop::AgentLoopConfig）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentLoopConfig {
    /// 人设文本（经 agent-loop 转发给 SystemPrompt::compose）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    /// 单工具执行超时秒数（0 = 不限时）。
    pub tool_timeout_secs: u64,
    /// 工具循环步数护栏（默认 None = 无上限；字段对齐
    /// yunyou-agent-loop::AgentLoopConfig.max_steps，挑刺报告 20261117 L-1：
    /// 文档口径的两旋钮缺席且本段 deny_unknown_fields，按文档配置即启动硬失败）。
    /// 显式设值 = 同意软着陆暂停语义（跑满即暂停可续，非腰斩），详见插件侧注释。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<usize>,
    /// 本轮 token 预算护栏（默认 None = 不设；对齐
    /// yunyou-agent-loop::AgentLoopConfig.token_budget）。显式设值 = 同意软着陆
    /// 暂停语义（累计 prompt+completion 达标即暂停可「继续」续跑）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self { persona: None, tool_timeout_secs: 300, max_steps: None, token_budget: None }
    }
}

/// `retry` 段：重试策略参数（消费方 `yunyou-retry` 插件，S10；字段对齐
/// yunyou_llm::RetryPolicy，设计 §5）。default 即产品：零配置装上＝连接失败/
/// 5xx/429/流中断自动重试 2 次、指数退避 0.5s→8s 带抖动、超时与鉴权 4xx 不重试
/// （安全默认律，裁决 D8）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RetryConfig {
    /// 最多重试次数（默认 2 = 最多 3 次尝试；预算永不放宽以接受劣化）。
    pub max_retries: u32,
    /// 可重试类别码白名单（按 LlmFailure 归一化类别码匹配；超时显式排除——
    /// 「不确定结果不自动重试」安全默认律）。
    pub retryable_codes: Vec<String>,
    /// 退避基数（毫秒）：`initial × 2^(attempt-1)` 封顶 `max_delay_ms`。
    pub initial_delay_ms: u64,
    /// 退避封顶（毫秒）。
    pub max_delay_ms: u64,
    /// 抖动比例（0..1，防惊群；sample 由消费侧提供）。
    pub jitter_ratio: f64,
    /// 超时是否重试（默认 false）：true 时白名单自动追加 "timeout"（opt-in 便利
    /// 字段，语义与手动把 "timeout" 写进 retryable_codes 等价——重试=可能双计费，
    /// 由用户显式选择承担）。
    pub retry_on_timeout: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            retryable_codes: vec![
                "connection".into(),
                "http_5xx".into(),
                "http_429".into(),
                "stream_interrupted".into(),
            ],
            initial_delay_ms: 500,
            max_delay_ms: 8000,
            jitter_ratio: 0.1,
            retry_on_timeout: false,
        }
    }
}

/// 应用环境变量覆盖（注入式 getter：生产传真实 env，测试传闭包，不碰进程全局态）。
///
/// 目前只覆盖 `llm` 段三键（联调验收的最小必要集）：env 存在即覆盖，缺省不动。
pub fn apply_env_overrides(config: &mut DshConfig, get: &dyn Fn(&str) -> Option<String>) {
    if let Some(value) = get(ENV_LLM_BASE_URL) {
        config.llm.base_url = value;
    }
    if let Some(value) = get(ENV_LLM_API_KEY) {
        config.llm.api_key = value;
    }
    if let Some(value) = get(ENV_LLM_MODEL) {
        config.llm.model = value;
    }
}

/// 生产用覆盖源：读进程环境变量。
pub fn default_env_overrides(config: &mut DshConfig) {
    apply_env_overrides(config, &|key: &str| std::env::var(key).ok());
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
