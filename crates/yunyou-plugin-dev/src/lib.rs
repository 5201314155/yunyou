//! 插件开发工具族（K-2/K-2-①）：`plugin_contract_read` + `plugin_validate`——
//! manifest 声明面在开发时即可验证（不依赖 REPL 装载报错回环）。
//!
//! # 定位
//! 插件开发者写完 manifest.toml 后，两个问题需要立刻可答：①声明面长什么样
//! （contract_read 结构化读取）②能不能装载（validate 静态校验+目录完整性）。
//! 校验逻辑**复用内核** `yunyou::validate_manifest` 同口径（单源纪律），本 crate
//! 不重写规则只做读取与格式化。
//!
//! # 工具族
//! - [`contract_read::ContractReadTool`]：manifest 契约面结构化读取；
//! - [`validate::ValidateTool`]：静态校验+目录完整性；
//! - 77/78 同族（plugin_scaffold/plugin_test_gen）随本骨架建。
//!
//! # 注册
//! 经 [`PluginDevPlugin`]（manifest id=`yunyou-plugin-dev`）进内置清单——
//! 与 tools/retry 同款表驱动。

pub mod contract_read;
pub mod creation_mode;
pub mod deliver;
pub mod prompt_section;
pub mod scaffold;
pub mod shared;
pub mod validate;

use yunyou::{PluginLevel, PluginManifest, projection_of};
use yunyou_tools::contract::ToolDefinition;
use yunyou_tools::registry::ToolsRegistry; // task-77：registry 直路径（plugin.rs 私有 use 非 re-export）
use yunyou_tools::registry::RegistryService;
use yunyou_util::BoxFut;

/// 插件开发工具族插件（四工具族注册载体）。
#[derive(Default)]
pub struct PluginDevPlugin;

impl yunyou::Plugin for PluginDevPlugin {
    fn name(&self) -> &str {
        "yunyou-plugin-dev"
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-plugin-dev".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["tools.v1".into()],
            provides: vec![projection_of::<dyn RegistryService>()],
            inject: vec![],
            ui: vec![],
            auto_enable: false, // 开发者显式启用（非默认全员装载）
        }
    }

    fn setup(
        self: std::sync::Arc<Self>,
        ctx: yunyou::Context,
        config: serde_json::Value,
    ) -> BoxFut<'static, yunyou::Result<yunyou::FiberOutput>> {
        Box::pin(async move {
            let registry = ToolsRegistry::new();
            let service: std::sync::Arc<dyn RegistryService> = registry.clone();
            ctx.provide(yunyou::ServiceId::of::<dyn RegistryService>(), service)?;
            Ok(yunyou::FiberOutput)
        })
    }
}

/// 双工具构造（注册面：供 plugin.rs 装配/测试直调）。
pub fn plugin_dev_tools(root: std::sync::Arc<std::path::Path>) -> Vec<std::sync::Arc<dyn ToolDefinition>> {
    vec![
        std::sync::Arc::new(contract_read::ContractReadTool { root: root.clone() }),
        std::sync::Arc::new(validate::ValidateTool { root }),
    ]
}

/// 三工具构造（K-2-② 加法演进：scaffold 需要插件根注入——写闸锚点；
/// 旧调用点（双工具形态）零改动，装配侧换用本形态即得 scaffold）。
pub fn plugin_dev_tools_with_plugins(
    root: std::sync::Arc<std::path::Path>,
    plugins_root: std::sync::Arc<std::path::Path>,
) -> Vec<std::sync::Arc<dyn ToolDefinition>> {
    let mut tools = plugin_dev_tools(root);
    tools.push(std::sync::Arc::new(scaffold::ScaffoldTool { plugins_root }));
    tools
}
