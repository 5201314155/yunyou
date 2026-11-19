//! web-search 插件装配（过渡期 bundled 内置；network 缝⑨开闸后转 market——
//! market 插件 network=false 恒禁，本插件是该缝开闸后的第一个客户，稿中明示）：
//! 读 config→构造 provider 链→向 `dyn RegistryService` 注册 web_search 工具。

use crate::tool::WebSearchTool;
use crate::{WebSearchTool as _ToolAlias};
use yunyou::{projection_of, ConfigValue, Context, CordisError, ErrorKind, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId};
use yunyou_tools::RegistryService;
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 插件配置（段：`[web_search]` 顶层段——cli bundled segment 对齐；
/// deny_unknown_fields：拼错键报错不静默，挑刺 F-4 同律）。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WebSearchConfig {
    /// 总开关（默认 true；[web_search] enabled=false 可关——注册面消失=真关）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 后端显式指定（"duckduckgo"|"searxng"|"brave"|"tavily"；缺省=表序取首个
    /// 可用者，零配置即 DDG）。切换后端只改这一行（验收 2）。
    #[serde(default)]
    pub provider: Option<String>,
    /// SearXNG 实例地址（自建最优；公共实例稳定性参差）。
    #[serde(default = "default_searxng_base")]
    pub searxng_base_url: String,
    /// 单次返回结果条数上限（≤8 封顶，纪律 §四.3）。
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// Brave API key（缺省读环境变量 BRAVE_API_KEY；凭据缝⑨开闸后迁移）。
    #[serde(default)]
    pub brave_api_key: Option<String>,
    /// Tavily API key（缺省读环境变量 TAVILY_API_KEY）。
    #[serde(default)]
    pub tavily_api_key: Option<String>,
}

fn default_true() -> bool {
    true
}
fn default_searxng_base() -> String {
    "https://searx.be".into()
}
fn default_max_results() -> usize {
    crate::MAX_RESULTS
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: None,
            searxng_base_url: default_searxng_base(),
            max_results: default_max_results(),
            brave_api_key: None,
            tavily_api_key: None,
        }
    }
}

/// web_search 插件：不 provide 服务（纯策略卡），setup 期向 `dyn RegistryService`
/// 注册 web_search（注册即 effect，fiber 卸载随撤）。
///
/// # 依赖
/// inject `dyn RegistryService`（bundled 常在，tools 插件 provide）。
#[derive(Default)]
pub struct WebSearchPlugin;

impl Plugin for WebSearchPlugin {
    fn name(&self) -> &str {
        "yunyou-web-search"
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-web-search".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["tools.v1".into()],
            provides: vec![],
            inject: vec![projection_of::<dyn RegistryService>()],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![ServiceId::of::<dyn RegistryService>()]
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: WebSearchConfig = serde_json::from_value(config)
                .map_err(|err| CordisError::new(ErrorKind::Internal, format!("web-search/配置解析失败: {err}")))?;
            if !cfg.enabled {
                tracing::info!("[web-search] enabled=false：web_search 不注册（真关，模型面不可见）");
                return Ok(FiberOutput);
            }
            let registry = ctx.get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
                .map_err(|err| CordisError::new(ErrorKind::Internal, format!("web-search/注册表不可见: {err}")))?;
            let tool: Arc<dyn yunyou_tools::ToolDefinition> = Arc::new(WebSearchTool { config: Arc::new(cfg) });
            let name = tool.name().to_string();
            let disposer = registry.register_tool(tool)?;
            ctx.effect(format!("web-search.tool({name})"), move || Ok(Some(disposer)))?;
            Ok(FiberOutput)
        })
    }
}

// 抑制未使用别名告警（导出路径单一来源：lib.rs re-export tool::WebSearchTool）
#[allow(unused_imports)]
use _ToolAlias as _;
