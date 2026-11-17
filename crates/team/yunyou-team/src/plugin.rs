//! 团队插件（bundled 第 9 卡，K-7）：lead 身份源+TeamService provide+工具族注册。
//!
//! 装配序（bundled.rs）：本卡列在 agent-loop 之后——工具注册消费 RegistryService
//! 与 SessionStore，epoch 机制保证依赖就绪才装载（位次是优化非正确性依赖）。
//! 禁用语义：[plugins] 表禁用本卡=无团队工具/无 lead 身份/无成员（子 fiber 全收），
//! 但内核 MessageHub 缝不受影响（缝由内核供，K-6 定案 f 行）。

use crate::task_types::{TeamConfig, TeamService};
use crate::service::TeamServiceState;
use yunyou::hub::MessageHub;
use yunyou_session::SessionStore;
use yunyou_tools::RegistryService;
use yunyou::{ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId, projection_of};
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 团队插件。
#[derive(Default)]
pub struct TeamPlugin;

impl Plugin for TeamPlugin {
    fn name(&self) -> &str {
        "yunyou-team"
    }

    /// 声明面：consume message.v1（hub 缝）+session.v1（会话派生），provide
    /// team.v1（TeamService——名册/任务板/消息的工具面聚合服务）。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-team".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["message.v1".into(), "session.v1".into(), "team.v1".into()],
            provides: vec![projection_of::<dyn TeamService>()],
            inject: vec![
                projection_of::<dyn MessageHub>(),
                projection_of::<dyn RegistryService>(),
                projection_of::<dyn SessionStore>(),
            ],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![
            ServiceId::of::<dyn MessageHub>(),
            ServiceId::of::<dyn RegistryService>(),
            ServiceId::of::<dyn SessionStore>(),
        ]
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(serde_json::to_value(schemars::schema_for!(TeamConfig)).expect("schema 可序列化"))
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            // 配置解析：空/Null 走默认（max_concurrency=4，裁③）
            let team_config: TeamConfig = serde_json::from_value(config).unwrap_or_default();
            let hub = ctx.get::<dyn MessageHub>(ServiceId::of::<dyn MessageHub>())?;
            let sessions = ctx.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>())?;
            // lead 身份源：自发成员语义——无条目 register 直接 Online，绑定本 fiber；
            // 本插件 fiber 是常驻 bundled fiber，lead 身份随其存活（禁用=身份随撤，
            // hub 缝不受影响）
            ctx.register_member("lead")?;
            let state = TeamServiceState::new(&ctx, hub, sessions, &team_config);
            let service: Arc<dyn TeamService> = Arc::new(state);
            ctx.provide(ServiceId::of::<dyn TeamService>(), Arc::clone(&service))?;
            // lead 侧工具族进全局注册表（成员侧覆写在成员 setup 内，member_tools.rs）
            crate::service::register_lead_tools(&ctx, service)?;
            Ok(FiberOutput)
        })
    }
}
