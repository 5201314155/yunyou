//! 主循环插件：inject 四服务（llm/sessions/tools/systemPrompt），装载即注册
//! `dyn AgentLoop`（对标 agent-loop/src/index.ts:291 的依赖声明）。
//!
//! # S10 重试策略的消费形态（对设计 D11 的偏差声明，待云枢 retro 裁认）
//!
//! 设计 §2/D11 裁「agent-loop inject 强依赖 RetryPolicyProvider」，前提是 bundled
//! 恒在不可卸载；S2 落地后 `[plugins]` 可禁用 bundled（只可禁用不可卸载两级语义），
//! 前提失效——若仍 inject 强依赖，禁用 retry 会让本插件 Pending「依赖未就绪」，
//! 整个 agent 瘫痪，与设计 §7 验收总口径「yunyou-retry 禁用后 agent-loop 无重试
//! 直连（fail-fast 行为可预期）」直接冲突。故实现取：**不进 inject，setup 期容错
//! get（缺服务即 None=直连）**。就绪时序论证：retry 零依赖注册即装载（微秒级），
//! 本插件须等 llm/sessions/tools/prompt 四依赖全 Active 才 setup（内核 epoch 机
//! 制），bundled 清单注册序 retry 先于本插件——三重前置于生产路径恒成立；集成
//! 测试锁定该时序（tests/retry.rs）。

use crate::turn::AgentLoopRunner;
use yunyou::{ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId, projection_of};
use yunyou_llm::Llm;
use yunyou_session::SessionStore;
use yunyou_system_prompt::SystemPrompt;
use yunyou_tools::RegistryService;
use yunyou_util::BoxFut;
use std::sync::Arc;
use std::time::Duration;

/// 主循环插件配置。
// deny_unknown_fields（挑刺报告 20261117 F-4）：与 config 镜像段双轨对齐，拼错键报错。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentLoopConfig {
    /// 人设文本（转发给 SystemPrompt::compose；worker 人设注入点）。
    #[serde(default)]
    pub persona: String,
    /// 单工具执行超时秒数（0 = 不限时；默认 300）。
    #[serde(default = "default_tool_timeout")]
    pub tool_timeout_secs: u64,
    /// 工具循环步数护栏（默认 None = 无上限）。终态语义对齐三家参照系
    /// （Claude Code 主循环 maxTurns 可选且默认不传=无上限、DSH TS 零命中
    /// 上限）：主循环跑到模型不再调工具为止，终 = 模型停手/取消/错误三路，
    /// 防跑飞 = 取消令牌贯穿 + 三路超时 + 错误上浮 + 流式可见性。显式设值 =
    /// 同意软着陆暂停语义：跑满该步数后注入「回复『继续』即可无缝延续」
    /// 暂停并正常返回（非腰斩截肢——业主实证令 2026-11-16：正常任务不应
    /// 触发上限截肢）；对无人值守 team worker 可拧上作运营成本闸。
    #[serde(default)]
    pub max_steps: Option<usize>,
    /// 本轮 token 预算护栏（默认 None = 不设；对齐 Codex 预算面口径）。显式
    /// 设值 = 同意暂停语义：累计 prompt+completion 达到预算即软着陆暂停（可
    /// 「继续」续跑）。诚实注记：usage 口径「任一步未报 → 整体 None」——
    /// 端点未报 usage 时本护栏对本轮失效（不部分估算，诚实让位于护栏）。
    #[serde(default)]
    pub token_budget: Option<u64>,
    /// 模型上下文窗口（token；默认 None = auto-compact 机制整体关）。诚实
    /// 声明：一期无模型注册表，窗口不知道就不猜——显式配置即启用（task-27）。
    #[serde(default)]
    pub context_window: Option<u64>,
    /// 摘要输出预留（token；默认 20_000——对齐 CC MAX_OUTPUT_TOKENS_FOR_SUMMARY，
    /// 按 compact 摘要输出 p99.99 定标）。触发线 = 窗口 − 本值 − 安全余量。
    #[serde(default = "default_compact_reserved_output")]
    pub compact_reserved_output: u64,
    /// 触发线安全余量（token；默认 32_000——覆盖水位精度边界=差一条回复的
    /// 量 + 请求组装误差）。
    #[serde(default = "default_compact_safety_margin")]
    pub compact_safety_margin: u64,
    /// 自动压缩开关（默认 true，对齐 CC config.ts:594；context_window=None
    /// 时整体关——开关与窗口是「与」关系）。
    #[serde(default = "default_auto_compact_enabled")]
    pub auto_compact_enabled: bool,
    /// 摘要连续失败熔断阈值（默认 3，对齐 CC 连败口径；/compact 手动成功清零）。
    #[serde(default = "default_compaction_failure_limit")]
    pub compaction_failure_limit: u32,
}

fn default_tool_timeout() -> u64 {
    300
}

fn default_compact_reserved_output() -> u64 {
    20_000
}

fn default_compact_safety_margin() -> u64 {
    32_000
}

fn default_auto_compact_enabled() -> bool {
    true
}

fn default_compaction_failure_limit() -> u32 {
    3
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            persona: String::new(),
            tool_timeout_secs: default_tool_timeout(),
            max_steps: None,
            token_budget: None,
            context_window: None,
            compact_reserved_output: default_compact_reserved_output(),
            compact_safety_margin: default_compact_safety_margin(),
            auto_compact_enabled: default_auto_compact_enabled(),
            compaction_failure_limit: default_compaction_failure_limit(),
        }
    }
}

/// 主循环服务契约（宿主/CLI 调用面）。
///
/// # 取消语义（方案 §5.1 R1）
/// `cancel` 是**调用方令牌**（对话轮取消源，如 REPL Ctrl-C）；**fiber 卸载取消源**
/// 由插件装载时注入 runner，两源在 `run_turn*` 内部合并为合一令牌
/// （调用方令牌的 child token 为合并点）：任一触发即整轮收流，工具执行同一令牌。
pub trait AgentLoop: Send + Sync {
    /// 跑一轮完整对话（用户输入 → 回复；工具循环内含；无事件外露）。
    fn run_turn(
        &self,
        user_input: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> BoxFut<'static, Result<crate::turn::TurnOutcome>>;

    /// 手动压缩（task-27 /compact，一期）：立即摘要压缩当前会话（不等水位
    /// 阈值）——用户主动管理上下文。与 auto 触发共用 run_compaction（成功清
    /// 熔断计数）。返回人话回执。水位未知（usage 不持久化）时 pre_tokens=0
    /// （观测注记：手动路径无水位测量）。
    fn compact_manual(
        &self,
        cancel: tokio_util::sync::CancellationToken,
    ) -> BoxFut<'static, Result<String>>;

    /// 观察者形态：边跑边发 [`crate::observer::TurnEvent`]（task-10 A 线）。
    ///
    /// # 事件时序契约
    /// 落盘类事件在 JSONL 落盘成功后发出；流式增量即时发出；
    /// 事件按时间序、与本 future 同任务顺序派发（观察者被逐个 await）。
    fn run_turn_observed(
        &self,
        user_input: &str,
        cancel: tokio_util::sync::CancellationToken,
        observer: Arc<dyn crate::observer::TurnObserver>,
    ) -> BoxFut<'static, Result<crate::turn::TurnOutcome>>;
}

/// 主循环插件。
#[derive(Default)]
pub struct AgentLoopPlugin;

impl Plugin for AgentLoopPlugin {
    fn name(&self) -> &str {
        "yunyou-agent-loop"
    }

    /// 声明面（方案 §2.2）：消费 llm/session/tools/system-prompt 四契约并
    /// provide agent-loop.v1；inject 与 [`Plugin::inject`] 同源（同一类型参数清单）。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-agent-loop".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec![
                "llm.v1".into(),
                "session.v1".into(),
                "tools.v1".into(),
                "system-prompt.v1".into(),
                "agent-loop.v1".into(),
                "hooks.v1".into(),
            ],
            provides: vec![
                projection_of::<dyn AgentLoop>(),
                projection_of::<dyn crate::hooks::HooksRegistryService>(),
            ],
            inject: vec![
                projection_of::<dyn Llm>(),
                projection_of::<dyn SessionStore>(),
                projection_of::<dyn RegistryService>(),
                projection_of::<dyn SystemPrompt>(),
            ],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![
            ServiceId::of::<dyn Llm>(),
            ServiceId::of::<dyn SessionStore>(),
            ServiceId::of::<dyn RegistryService>(),
            ServiceId::of::<dyn SystemPrompt>(),
        ]
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(
            serde_json::to_value(schemars::schema_for!(AgentLoopConfig)).expect("schema 可序列化"),
        )
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: AgentLoopConfig = serde_json::from_value(config)
                .map_err(|err| yunyou::CordisError::new(yunyou::ErrorKind::Internal, format!("loop/配置解析失败: {err}")))?;
            // 依赖在 setup 时必然 Active（epoch 已就绪），get 直接成功
            let sessions = ctx.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>())?;
            let llm = ctx.get::<dyn Llm>(ServiceId::of::<dyn Llm>())?;
            let tools = ctx.get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())?;
            let prompt = ctx.get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>())?;
            // 重试策略服务（S10）：**可选消费，不进 inject**——inject 是强依赖
            // （缺任一即 Inactive），声明会令「禁用 retry」连带瘫痪主循环，与设计
            // 验收总口径「yunyou-retry 禁用后无重试直连」冲突；此为对 D11
            // 「inject 强依赖」的偏差，理由与时序论证见模块 rustdoc，待云枢 retro。
            // 就绪前提：bundled 清单注册序（retry 先于本插件）+ retry 零依赖先装载。
            let retry = ctx
                .get::<dyn yunyou_llm::RetryPolicyProvider>(ServiceId::of::<
                    dyn yunyou_llm::RetryPolicyProvider,
                >())
                .ok();
            let tool_timeout = if cfg.tool_timeout_secs == 0 {
                None
            } else {
                Some(Duration::from_secs(cfg.tool_timeout_secs))
            };
            // fiber 取消令牌（方案 §5.1 R1）：真源=内核 Context::cancellation()
            // 克隆 FiberShared.cancel（卸载/处置即触发），与调用方令牌在 run_turn
            // 内合并——插件被禁用时在途轮不再「残尸」。
            let fiber_cancel = ctx.cancellation();
            // 组装前钩子注册表（task-11 缝）：本插件 provide 服务面（钩子提供方
            // 插件 inject dyn HooksRegistryService 注册），runner 持同一实例
            // 每轮快照——注册先后无关，运行期注册即下轮生效。
            let hooks: Arc<crate::hooks::HooksRegistry> = Default::default();
            ctx.provide(
                ServiceId::of::<dyn crate::hooks::HooksRegistryService>(),
                hooks.clone() as Arc<dyn crate::hooks::HooksRegistryService>,
            )?;
            // 审批缝服务面（task-16 S1）：策略插件 inject GateRegistryService 注册
            // gate 链；UI 侧/模式插件 inject ApprovalUiService 注册呈现器（对象
            // 类型注册制）。runner 持同一实例；空注册=行为与无缝逐字节一致。
            let gates: Arc<crate::gate::GateHub> = Default::default();
            ctx.provide(
                ServiceId::of::<dyn crate::gate::GateRegistryService>(),
                gates.clone() as Arc<dyn crate::gate::GateRegistryService>,
            )?;
            ctx.provide(
                ServiceId::of::<dyn crate::gate::ApprovalUiService>(),
                gates.clone() as Arc<dyn crate::gate::ApprovalUiService>,
            )?;
            // 模式会话态服务面由权限插件提供（ModeGate=hub+提示段刷新+切换
            // SystemMessage 通知三合一；本插件只供 GateHubHandle 裸句柄——
            // set_mode 的提示段/通知副作用归策略侧，内聚单点）
            ctx.provide(
                ServiceId::of::<dyn crate::gate::GateHubHandle>(),
                Arc::new(crate::gate::GateHubHandleWrapper(Arc::clone(&gates)))
                    as Arc<dyn crate::gate::GateHubHandle>,
            )?;
            let runner: Arc<dyn AgentLoop> = Arc::new(LoopService {
                runner: AgentLoopRunner {
                    // K-16① 字段缺省（79f29da TurnOutcome 演进接缝补件）：
                    // REPL 人工模式 auto_winddown=false；预警比 0.8；宽步 4。
                    budget_warn_ratio: 0.8,
                    auto_winddown: false,
                    winddown_grace_steps: 4,
                    sessions,
                    llm,
                    tools,
                    prompt,
                    persona: cfg.persona,
                    tool_timeout,
                    max_steps: cfg.max_steps,
                    token_budget: cfg.token_budget,
                    context_window: cfg.context_window,
                    compact_reserved_output: cfg.compact_reserved_output,
                    compact_safety_margin: cfg.compact_safety_margin,
                    auto_compact_enabled: cfg.auto_compact_enabled,
                    compaction_failure_limit: cfg.compaction_failure_limit,
                    compaction_state: crate::compaction::CompactionState::new(),
                    fiber_cancel,
                    retry,
                    hooks,
                    // task-16 S1（yun-bo 在途 gate.rs）编译性补参：GateHub::default()=
                    // 无缝全空=行为与无 gate 逐字节一致（gate.rs:253 注释自证）；
                    // 语义接线归 task-16 承办席
                    gates: Default::default(),
                },
            });
            ctx.provide(ServiceId::of::<dyn AgentLoop>(), runner)?;
            Ok(FiberOutput)
        })
    }
}

/// [`AgentLoop`] trait 的薄包装（持有 runner）。
struct LoopService {
    runner: AgentLoopRunner,
}

impl AgentLoop for LoopService {
    fn compact_manual(
        &self,
        cancel: tokio_util::sync::CancellationToken,
    ) -> BoxFut<'static, Result<String>> {
        let runner = self.runner.clone();
        Box::pin(async move {
            let mut history = runner
                .sessions
                .load_all()
                .await
                .map_err(|err| yunyou::CordisError::new(
                    yunyou::ErrorKind::Internal,
                    format!("compact/会话读取失败: {err}"),
                ))?;
            let pre_tokens = 0_u64; // 手动路径无水位测量（usage 不持久化，观测注记）
            runner.run_compaction(&mut history, &cancel, pre_tokens, &None).await?;
            Ok("已压缩：前文对话已摘要，后续对话以摘要为前文记忆（完整历史仍在会话文件可回查）。".into())
        })
    }

    fn run_turn(
        &self,
        user_input: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> BoxFut<'static, Result<crate::turn::TurnOutcome>> {
        // runner 无自身状态（四服务均为 Arc），克隆进 'static future
        let input = user_input.to_string();
        let runner = self.runner.clone();
        Box::pin(runner.run_turn(input, cancel))
    }

    fn run_turn_observed(
        &self,
        user_input: &str,
        cancel: tokio_util::sync::CancellationToken,
        observer: Arc<dyn crate::observer::TurnObserver>,
    ) -> BoxFut<'static, Result<crate::turn::TurnOutcome>> {
        let input = user_input.to_string();
        let runner = self.runner.clone();
        Box::pin(runner.run_turn_observed(input, cancel, observer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// manifest 声明面字段齐全（方案 §2.2，S1 验收）：四依赖契约＋自供 agent-loop.v1。
    #[test]
    fn manifest声明面字段齐全() {
        let m = AgentLoopPlugin.manifest();
        assert_eq!(m.id, "yunyou-agent-loop");
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(
            m.provides,
            vec!["dyn AgentLoop".to_string(), "dyn HooksRegistryService".to_string()]
        );
        assert_eq!(
            m.inject,
            vec!["dyn Llm", "dyn SessionStore", "dyn RegistryService", "dyn SystemPrompt"]
        );
        assert_eq!(m.contracts.len(), 6);
        assert!(m.ui.is_empty());
    }

    /// manifest.inject 与 trait inject() 双向一致（装载前校验零违例）。
    #[test]
    fn manifest与trait_inject双向一致() {
        let plugin = AgentLoopPlugin;
        let issues = yunyou::validate_manifest(&plugin, &plugin.manifest());
        assert!(issues.is_empty(), "校验违例: {issues:?}");
    }

    /// L-1（挑刺报告 20261117）：yunyou.toml `[agent_loop]` 段值经宿主段透传
    /// （assemble merge_segment 原样 JSON）反序列化进本插件 Config，护栏两旋钮
    /// **生效直达 runner**——runner 消费路径另有 tests/limits.rs 用例②③端到端锁定。
    #[test]
    fn 配置段值护栏旋钮直达插件config() {
        let cfg: AgentLoopConfig = serde_json::from_value(json!({
            "max_steps": 5,
            "token_budget": 100_000u64,
        }))
        .expect("段值应能反序列化（L-1 缺席即本测试红：unknown field 拒装）");
        assert_eq!(cfg.max_steps, Some(5));
        assert_eq!(cfg.token_budget, Some(100_000));
        // 缺省段=护栏全关（与 limits.rs 用例①默认 None 语义一致）
        let cfg: AgentLoopConfig = serde_json::from_value(json!({})).expect("空段应落默认值");
        assert_eq!(cfg.max_steps, None);
        assert_eq!(cfg.token_budget, None);
    }

    /// F-4（挑刺报告 20261117）：组件侧 Config 拼错键要报错（与 config 镜像
    /// 段 deny 双轨对齐），不静默吞。
    #[test]
    fn 拼错键报错不静默吞() {
        let err = serde_json::from_value::<AgentLoopConfig>(serde_json::json!({
            "max_stepz": 5
        }))
        .expect_err("未知键 max_stepz 必报");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }
}
