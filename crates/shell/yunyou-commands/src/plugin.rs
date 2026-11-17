//! 命令缝插件：装载即注册 `dyn CommandsService` 服务（注册表实例，任务29 C1）。
//!
//! 装载位次契约：本插件必须在一切要 register 命令的插件（壳命令/技能扫描/
//! 插件命令示范）之前就绪——消费方在 manifest `inject` 声明 `dyn CommandsService`，
//! 内核 epoch 机制等本服务 Active 再装载，清单顺序不构成正确性依赖。

use crate::{CommandRegistry, CommandsService};
use std::sync::Arc;
use yunyou::{
    ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId,
    projection_of,
};
use yunyou_util::BoxFut;

/// 命令缝插件：provide `dyn CommandsService`。
///
/// # 依赖
/// 无。# 注册
/// `ctx.provide(ServiceId::of::<dyn CommandsService>(), CommandRegistry::new())`。
#[derive(Default)]
pub struct CommandsPlugin;

impl Plugin for CommandsPlugin {
    fn name(&self) -> &str {
        "yunyou-commands"
    }

    /// 声明面：契约 commands.v1、provide `dyn CommandsService`、无依赖。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-commands".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["commands.v1".into()],
            provides: vec![projection_of::<dyn CommandsService>()],
            inject: vec![],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            // 显式擦除：泛型推断会把 P 定为具体类型 CommandRegistry，
            // 导致 get::<dyn CommandsService> 类型不符（session 插件同款）
            let service: Arc<dyn CommandsService> = Arc::new(CommandRegistry::new());
            ctx.provide(ServiceId::of::<dyn CommandsService>(), service)?;
            Ok(FiberOutput)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// manifest 声明面字段齐全（方案 §2.2 形态，沿 bundled 插件测试先例）。
    #[test]
    fn manifest声明面字段齐全() {
        let m = CommandsPlugin.manifest();
        assert_eq!(m.id, "yunyou-commands");
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(m.contracts, vec!["commands.v1".to_string()]);
        assert_eq!(m.provides, vec!["dyn CommandsService".to_string()]);
        assert!(m.inject.is_empty() && m.ui.is_empty());
    }

    /// manifest.inject 与 trait inject() 双向一致（装载前校验零违例）。
    #[test]
    fn manifest与trait_inject双向一致() {
        let plugin = CommandsPlugin;
        let issues = yunyou::validate_manifest(&plugin, &plugin.manifest());
        assert!(issues.is_empty(), "校验违例: {issues:?}");
    }
}
