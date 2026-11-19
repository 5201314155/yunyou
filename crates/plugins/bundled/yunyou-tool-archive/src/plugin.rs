//! tool-archive 插件装配：读 config→解析 store_dir（config 显式 > 会话存储
//! 投影 > 诚实禁用）→向 HooksRegistryService 注册剥离钩子。

use crate::{ToolArchiveHook, STUB_PREFIX};
use yunyou::{projection_of, ConfigValue, Context, CordisError, ErrorKind, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId};
use yunyou_agent_loop::HooksRegistryService;
use yunyou_session::SessionStore;
use yunyou_util::BoxFut;
use std::path::PathBuf;
use std::sync::Arc;

/// 插件配置（段：`[plugins.yunyou-tool-archive.config]`）。
// deny_unknown_fields（挑刺报告 20261117 F-4）：拼错键报错，不静默落默认。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolArchiveConfig {
    /// 总开关（默认 true；[tool_archive] enabled=false 可关——lead 裁）。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// side-store 根目录；缺省从会话存储推导（`<会话目录>/tool_results`）。
    /// 显式配置覆盖投影值；两者皆无=禁用+warn（诚实不猜路径）。
    #[serde(default)]
    pub store_dir: Option<String>,
    /// 新鲜窗：最近 N 条工具结果不剥（默认 2；模型很可能马上引用刚拿到的结果）。
    #[serde(default = "default_fresh_window")]
    pub fresh_window: usize,
}

fn default_enabled() -> bool {
    true
}

fn default_fresh_window() -> usize {
    2
}

/// 工具结果剥离策略插件：不 provide 任何服务（纯策略卡），setup 期向
/// `dyn HooksRegistryService`（agent-loop 提供）注册 [`ToolArchiveHook`]。
///
/// # 依赖
/// inject `dyn SessionStore`（store_dir 投影源）+`dyn HooksRegistryService`
/// （注册面）。两者均 bundled 常在；本插件经 bundled 清单装载（预置启用），
/// `[plugins.yunyou-tool-archive] enable=false` 可关。
#[derive(Default)]
pub struct ToolArchivePlugin;

impl Plugin for ToolArchivePlugin {
    fn name(&self) -> &str {
        "yunyou-tool-archive"
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-tool-archive".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["hooks.v1".into(), "session.v1".into()],
            provides: vec![],
            inject: vec![
                projection_of::<dyn HooksRegistryService>(),
                projection_of::<dyn SessionStore>(),
            ],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![
            ServiceId::of::<dyn HooksRegistryService>(),
            ServiceId::of::<dyn SessionStore>(),
        ]
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: ToolArchiveConfig = serde_json::from_value(config).map_err(|err| {
                CordisError::new(ErrorKind::Internal, format!("tool-archive/配置解析失败: {err}"))
            })?;
            if !cfg.enabled {
                tracing::info!("[tool-archive] enabled=false，钩子未注册（工具结果全量回传）");
                return Ok(FiberOutput);
            }
            // store_dir 解析序（简报 §三，lead 裁）：config 显式 > 会话存储投影
            // > 诚实禁用（不猜路径）
            let store_dir: Option<PathBuf> = match &cfg.store_dir {
                Some(dir) => Some(PathBuf::from(dir)),
                None => ctx
                    .get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>())?
                    .path()
                    .and_then(|p| p.parent().map(PathBuf::from)),
            };
            let Some(store_dir) = store_dir else {
                tracing::warn!(
                    "[tool-archive] store_dir 未配置且会话存储无路径可投影（纯 SDK/外部环境）——诚实禁用，工具结果全量回传"
                );
                return Ok(FiberOutput);
            };
            let hooks = ctx.get::<dyn HooksRegistryService>(ServiceId::of::<
                dyn HooksRegistryService,
            >())?;
            hooks.register(Arc::new(ToolArchiveHook::new(
                store_dir.clone(),
                cfg.fresh_window,
            )))?;
            tracing::info!(
                "[tool-archive] 已注册剥离钩子（store_dir={}，fresh_window={}；stub 前缀 {STUB_PREFIX:?}）",
                store_dir.display(),
                cfg.fresh_window
            );
            Ok(FiberOutput)
        })
    }
}
