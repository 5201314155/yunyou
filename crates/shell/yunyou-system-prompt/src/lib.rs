//! 系统提示词组装服务（方案 §2.5，对标 TS `system-prompt/src/index.ts` 的职责裁剪）。
//!
//! # 契约
//! - [`SystemPrompt::compose`] 是**纯函数语义**：相同输入产出相同提示词（可复现）；
//! - **契约③（S3 改造，吸收 P0-6）**：提示段走「注册即 effect、卸载即回收」的段注册表
//!   （[`SectionRegistry::register_section`]，返回内核 Disposer），compose 消费注册表
//!   按段位表（`order`）排序拼接；人设降级为一个**默认注册的段**（人设槽）；
//! - 工具描述：每个工具一行 `工具名 — 描述`，顺序由调用方传入的 schema 顺序决定；
//! - 实现必须 `Send + Sync`；注册形态：`ctx.provide(ServiceId::of::<dyn SystemPrompt>(), ...)`。
//!
//! # persona 断头路根治（本批 S3）
//! 旧实现把配置人设吃进 `with_persona` 后丢弃（`let _ = persona`），人设只能靠 CLI 装配处
//! `bridge_persona` 经 agent-loop 段转发才能生效——组件侧是一条断头路。现在人设是
//! **段位表里的一个默认注册段**（`SECTION_PERSONA_PREFIX`）：组件自带默认值，运行期由
//! `compose` 的 `persona` 参数按同名 shadow 机制替换（空＝保留默认段）。
//! CLI 侧 `bridge_persona` 桥接**保留不动**（云门辖区）；组件侧死字段已清除。

mod assemble;
mod error;
mod interpolate;
pub mod persona_md;
mod registry;
mod section;

pub use error::SystemPromptError;
pub use registry::SectionRegistry;
pub use section::{
    Section, ORDER_HARNESS_IDENTITY, ORDER_PERSONA_PREFIX, ORDER_TOOLS, SECTION_HARNESS_IDENTITY,
    SECTION_PERSONA_PREFIX, SECTION_TOOLS,
};

use yunyou::{
    ConfigValue, Context, Disposer, FiberOutput, Plugin, PluginLevel, PluginManifest, ServiceId,
    projection_of,
};
use yunyou_llm::ToolSchema;
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 系统提示词服务契约。
pub trait SystemPrompt: Send + Sync {
    /// 组装系统提示词。
    ///
    /// # 参数
    /// - `persona`：运行期人设（人设槽替换；空串＝保留组件默认注册的人设段）；
    /// - `tools`：本轮可声明的工具集（描述注入）。
    ///
    /// 签名与返回类型**冻结**（消费方 agent-loop 在辖区外）：装配失败在此降级并记
    /// `error!`（详见 [`SectionRegistry::compose_lossy`]）；要可失败版用 [`Self::try_compose`]。
    fn compose(&self, persona: &str, tools: &[ToolSchema]) -> String;

    /// 契约③：注册一个可注销的提示段（注册即 effect，返回内核 Disposer）。
    ///
    /// 插件在 `setup()` 里照内核 effect 模式接上即可（卸载逆序清理、卸载不留残尸）：
    ///
    /// ```ignore
    /// let prompt = ctx.get::<dyn SystemPrompt>()?;
    /// let disposer = prompt.register_section(Section::new("my:policy", 500.0, "……"))?;
    /// ctx.effect("systemPrompt.section", move || Ok(Some(disposer)))?;
    /// ```
    fn register_section(&self, section: Section) -> Result<Disposer, SystemPromptError>;

    /// 可失败装配：段位表排序 + complete 互斥校验 + 插值正反例全走 [`SystemPromptError`]。
    ///
    /// 默认实现转发 [`Self::compose`]（既有实现无需改动即保持编译）。
    fn try_compose(
        &self,
        persona: &str,
        tools: &[ToolSchema],
    ) -> Result<String, SystemPromptError> {
        Ok(self.compose(persona, tools))
    }
}

/// 身份自述文案（宿主默认段，段位最低，对标官方 `harness:identity`）。
///
/// 公开导出：插件可据此以同名 shadow 方式替换 identity 槽（契约③ 替换机制）。
pub const IDENTITY_TEXT: &str = "你是 yunyou 终端里的编码助手。谨慎、直接、如实验证。";

/// 默认组装实现：持段注册表，compose 消费注册表（契约③）。
#[derive(Debug)]
pub struct DefaultSystemPrompt {
    sections: Arc<SectionRegistry>,
}

impl Default for DefaultSystemPrompt {
    fn default() -> Self {
        Self::with_registry(default_registry())
    }
}

impl DefaultSystemPrompt {
    /// 携带人设构造：人设降级为一个**默认注册的段**（人设槽，非空才落表）。
    ///
    /// 人设文本含畸形 `{{...}}` 引用时装载期抛错（对标官方 renderPrompt 抛错点）。
    pub fn with_persona(persona: String) -> Result<Self, SystemPromptError> {
        let registry = Arc::new(SectionRegistry::default());
        register_default_sections(&registry, &persona)?;
        Ok(Self { sections: registry })
    }

    /// 以既有段注册表构造（组件/测试可自带注册表再追加段）。
    pub fn with_registry(sections: Arc<SectionRegistry>) -> Self {
        Self { sections }
    }

    /// 段注册表句柄：插件经它注册新段（注册即 effect，见 [`SystemPrompt::register_section`]）。
    pub fn sections(&self) -> &Arc<SectionRegistry> {
        &self.sections
    }
}

/// 宿主默认段注册（身份段段位最低，人设槽次之；对标官方 SECTION_ORDERS 前两档）。
fn register_default_sections(
    registry: &Arc<SectionRegistry>,
    persona: &str,
) -> Result<(), SystemPromptError> {
    registry.register_section(Section::new(
        SECTION_HARNESS_IDENTITY,
        ORDER_HARNESS_IDENTITY,
        IDENTITY_TEXT,
    ))?;
    registry.register_section(Section::new(
        SECTION_PERSONA_PREFIX,
        ORDER_PERSONA_PREFIX,
        persona.trim(),
    ))?;
    Ok(())
}

/// 空注册表 + 宿主默认段（供 `Default` 与既有单元测试路径使用）。
///
/// 静态文案不可能触发校验失败（无变量引用、段序有限），故 `expect` 而非吞错。
fn default_registry() -> Arc<SectionRegistry> {
    let registry = Arc::new(SectionRegistry::default());
    register_default_sections(&registry, "").expect("宿主默认段静态合法");
    registry
}

impl SystemPrompt for DefaultSystemPrompt {
    fn compose(&self, persona: &str, tools: &[ToolSchema]) -> String {
        self.sections.compose_lossy(persona, tools)
    }

    fn register_section(&self, section: Section) -> Result<Disposer, SystemPromptError> {
        self.sections.register_section(section)
    }

    fn try_compose(
        &self,
        persona: &str,
        tools: &[ToolSchema],
    ) -> Result<String, SystemPromptError> {
        self.sections.try_compose(persona, tools)
    }
}

/// 系统提示词插件配置。
// deny_unknown_fields（挑刺报告 20261117 F-4）：与 config 镜像段双轨对齐，拼错键报错。
#[derive(Debug, Clone, Default, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SystemPromptConfig {
    /// 人设文本（worker 人设注入即此字段，方案 §2.7.2）。
    ///
    /// S3 起**真正生效**：作为默认注册的人设段落表（旧实现吃进即丢，见 lib 模块文档）。
    #[serde(default)]
    pub persona: String,
}

/// 系统提示词插件：注册 `dyn SystemPrompt` 服务。
#[derive(Default)]
pub struct SystemPromptPlugin;

impl Plugin for SystemPromptPlugin {
    fn name(&self) -> &str {
        "yunyou-system-prompt"
    }

    /// 声明面（方案 §2.2）：对齐 system-prompt.v1 插槽规格、provide `dyn SystemPrompt`、无依赖。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-system-prompt".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["system-prompt.v1".into()],
            provides: vec![projection_of::<dyn SystemPrompt>()],
            inject: vec![],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(
            serde_json::to_value(schemars::schema_for!(SystemPromptConfig))
                .expect("schema 可序列化"),
        )
    }

    fn setup(
        self: Arc<Self>,
        ctx: Context,
        config: ConfigValue,
    ) -> BoxFut<'static, yunyou::Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: SystemPromptConfig = serde_json::from_value(config).map_err(|err| {
                yunyou::CordisError::new(
                    yunyou::ErrorKind::Internal,
                    format!("system-prompt/配置解析失败: {err}"),
                )
            })?;
            let service = DefaultSystemPrompt::with_persona(cfg.persona).map_err(|err| {
                yunyou::CordisError::new(
                    yunyou::ErrorKind::PluginSetup,
                    format!("system-prompt/人设段注册失败: {err}"),
                )
            })?;
            let service: Arc<dyn SystemPrompt> = Arc::new(service);
            ctx.provide(ServiceId::of::<dyn SystemPrompt>(), service)?;
            Ok(FiberOutput)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// manifest 声明面字段齐全（方案 §2.2，S1 验收）。
    #[test]
    fn manifest声明面字段齐全() {
        let m = SystemPromptPlugin.manifest();
        assert_eq!(m.id, "yunyou-system-prompt");
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(m.contracts, vec!["system-prompt.v1".to_string()]);
        assert_eq!(m.provides, vec!["dyn SystemPrompt".to_string()]);
        assert!(m.inject.is_empty() && m.ui.is_empty());
    }

    /// manifest.inject 与 trait inject() 双向一致（装载前校验零违例）。
    #[test]
    fn manifest与trait_inject双向一致() {
        let plugin = SystemPromptPlugin;
        let issues = yunyou::validate_manifest(&plugin, &plugin.manifest());
        assert!(issues.is_empty(), "校验违例: {issues:?}");
    }

    /// F-4（挑刺报告 20261117）：组件侧 Config 拼错键要报错（与 config 镜像
    /// 段 deny 双轨对齐），不静默吞。
    #[test]
    fn 拼错键报错不静默吞() {
        let err = serde_json::from_value::<SystemPromptConfig>(serde_json::json!({
            "personna": "oops"
        }))
        .expect_err("未知键 personna 必报");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    /// persona 断头路根治：配置人设落表为默认段，空运行期人设也看得到（旧实现此处为空）。
    #[test]
    fn 配置人设落表为默认段() {
        let prompt = DefaultSystemPrompt::with_persona("云悠品牌人设".into())
            .expect("静态人设合法");
        let out = prompt.compose("", &[]);
        assert!(out.contains("云悠品牌人设"), "默认人设段应生效: {out}");
    }
}