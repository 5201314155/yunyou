//! permission 插件装配：加载 settings [permissions] →编译规则集→向
//! GateRegistryService 注册 [`PermissionGate`]（形态对齐 tool-archive 纯策略卡）。

use crate::gate::PermissionGate;
use crate::rules::RuleSet;
use std::path::PathBuf;
use std::sync::Arc;
use yunyou::Result;
use yunyou::projection_of;
use yunyou::ConfigValue;
use yunyou::Context;
use yunyou::CordisError;
use yunyou::ErrorKind;
use yunyou::FiberOutput;
use yunyou::Plugin;
use yunyou::PluginLevel;
use yunyou::PluginManifest;
use yunyou::ServiceId;
use yunyou_agent_loop::GateRegistryService;
use yunyou_util::BoxFut;

/// 插件配置（段：`[plugins.yunyou-permission.config]`）。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PermissionConfig {
    /// 总开关（默认 true；false=不注册 gate，全部工具照跑）。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// settings.json 路径（缺省 `~/.yunyou/settings.json`；测试/嵌入显式注入）。
    #[serde(default)]
    pub settings_path: Option<String>,
    /// settings 缺失/解析失败时是否 fail-loud（默认 false=缺文件空规则集诚实
    /// 放行；true=装配失败拒启——公网/强管制部署可拧）。
    #[serde(default)]
    pub strict: bool,
    /// 工作区根（缺省=进程 cwd 投影；acceptEdits/default「区内写免审」判定面，
    /// S4 经 GateContext.workspace_root 透传——运行期真源候 workspace 服务接线）。
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// L4 沙箱穿越开关（默认 false；true=愿进沙箱的命令穿越 ask 规则自动
    /// 放行——claude autoAllowBashIfSandboxed 同律，设计稿 §一 L4）。
    #[serde(default)]
    pub auto_allow_sandboxed: bool,
}

fn default_enabled() -> bool {
    true
}

/// 缺省 settings 路径（home 投影；取不到=None 走空规则集）。
fn default_settings_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".yunyou").join("settings.json"))
}

/// 权限审批策略插件：不 provide 服务（纯策略卡），setup 期向
/// `dyn GateRegistryService`（agent-loop 提供，S1 缝）注册规则 gate。
#[derive(Default)]
pub struct PermissionPlugin;

impl Plugin for PermissionPlugin {
    fn name(&self) -> &str {
        "yunyou-permission"
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-permission".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["gate.v1".into()],
            provides: vec![],
            inject: vec![projection_of::<dyn GateRegistryService>()],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![ServiceId::of::<dyn GateRegistryService>()]
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: PermissionConfig = serde_json::from_value(config)
                .map_err(|err| CordisError::new(ErrorKind::Internal, format!("permission/配置解析失败: {err}")))?;
            if !cfg.enabled {
                return Ok(FiberOutput); // 显式关停：不注册 gate（诚实零行为）
            }
            let path = cfg
                .settings_path
                .as_ref()
                .map(PathBuf::from)
                .or_else(default_settings_path);
            let rules = match path {
                Some(path) => match RuleSet::load_user_settings(&path) {
                    Ok(rules) => rules,
                    Err(err) if cfg.strict => {
                        return Err(CordisError::new(ErrorKind::Internal, format!("permission/{err}")))
                    }
                    Err(err) => {
                        eprintln!("[permission] 规则集加载失败（strict=false 降级空规则集）：{err}");
                        RuleSet::default()
                    }
                },
                // home 不可得：诚实空规则集（denylist 内置层仍生效）
                None => RuleSet::default(),
            };
            let registry = ctx
                .get::<dyn GateRegistryService>(ServiceId::of::<dyn GateRegistryService>())
                .map_err(|err| CordisError::new(ErrorKind::Internal, format!("permission/审批缝不可见: {err}")))?;
            let mut gate = PermissionGate::new(rules);
            gate.set_auto_allow_sandboxed(cfg.auto_allow_sandboxed);
            registry.register(Arc::new(gate))?;
            // workspace_root 投影（S4）：config 显式 > cwd；经 GateContext 透传的
            // 接线在 runner 侧（GateContext::default 现不带 root——执行面接线候
            // workspace 服务注入，此处落配置面）。
            if cfg.workspace_root.is_some() {
                // 值随 cfg 生命周期存 GateHub 外无状态——S4 注记：接线点在此，
                // runner 侧 GateContext 组装候 workspace 服务提供真源
            }
            Ok(FiberOutput)
        })
    }
}
