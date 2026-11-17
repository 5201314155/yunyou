//! 凭据插件：装载即注册 `dyn CredentialService`（宿主托管存储）与
//! `dyn SettingsProjection`（S8 投影）两服务（样版对齐 yunyou-session::plugin）。
//!
//! 装配接线注记（S2 收口合入用）：`assemble` 序列在五插件后追加
//! `root.plugin(CredentialsPlugin, segment(&raw, "credentials"))`；
//! `credentials.store_path` 缺省相对路径，装配层补 `<yunyou-home>/credentials.toml`
//! （同 `session.path` 的 with_session_default 模式）——本 crate 不依赖 yunyou-cli。
//!
//! # 存储双实例注记（审二轮 P2）
//!
//! 宿主（装配层 `$REF` 解析与启动迁移）与本插件 setup 各 open 一个
//! [`crate::HostCredentialStore`]——双实例读同一文件，互不共享内存表。约束：
//! **运行期严禁任何 set/unset**（双实例下另一实例内存表会失同步），写侧仅启动期
//! 迁移路径（migrate_plaintext_key，此时两实例行为以文件为准可接受）；管理面写
//! 操作（面板改密钥）二期 provide 前复用单实例后再放开。

use crate::settings::{SettingsProjection, SettingsProjector};
use crate::store::HostCredentialStore;
use crate::CredentialService;
use yunyou::{projection_of, ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId};
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 凭据插件配置（段：`credentials`）。
// deny_unknown_fields（挑刺报告 20261117 F-4）：与 config 镜像段双轨对齐，拼错键报错
// （误写进未声明键的明文 key 不再原样进配置面投影，与 F-8 叠加面一并收口）。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialsConfig {
    /// 凭据存储文件路径；缺省相对路径，装配层补 `<yunyou-home>/credentials.toml`
    /// （600 权限，见 [`crate::store`]）。
    #[serde(default = "default_store_path")]
    pub store_path: String,
}

fn default_store_path() -> String {
    "credentials.toml".into()
}

impl Default for CredentialsConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::Value::Object(serde_json::Map::new()))
            .expect("全默认值构造必然成功")
    }
}

/// 凭据插件：注册 `dyn CredentialService` + `dyn SettingsProjection`。
///
/// # 依赖
/// 无（托管存储自带文件路径；env 兜底读进程环境）。
/// # 注册
/// `ctx.provide(ServiceId::of::<dyn CredentialService>(), HostCredentialStore)`；
/// `ctx.provide(ServiceId::of::<dyn SettingsProjection>(), SettingsProjector)`。
#[derive(Default)]
pub struct CredentialsPlugin;

impl Plugin for CredentialsPlugin {
    fn name(&self) -> &str {
        "yunyou-credentials"
    }

    /// 声明面（方案 §2.2）：对齐 credentials.v1 + settings.v1 两个插槽规格、
    /// provide 双服务、无依赖；ui 条目示范「设置表单段」指向自身 config 段（S8）。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-credentials".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["credentials.v1".into(), "settings.v1".into()],
            provides: vec![
                projection_of::<dyn CredentialService>(),
                projection_of::<dyn SettingsProjection>(),
            ],
            inject: vec![],
            ui: vec![yunyou::UiEntry::Settings { section: "credentials".into() }],
            auto_enable: true,
        }
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(serde_json::to_value(schemars::schema_for!(CredentialsConfig)).expect("schema 可序列化"))
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: CredentialsConfig = serde_json::from_value(config).map_err(|err| {
                yunyou::CordisError::new(
                    yunyou::ErrorKind::Internal,
                    format!("credentials/配置解析失败: {err}"),
                )
            })?;
            let store = HostCredentialStore::open(cfg.store_path.into())?;
            // 显式擦除：泛型推断会把 P 定为具体类型，导致 get::<dyn Trait> 类型不符
            let service: Arc<dyn CredentialService> = Arc::new(store);
            ctx.provide(ServiceId::of::<dyn CredentialService>(), service)?;
            let projector: Arc<dyn SettingsProjection> = Arc::new(SettingsProjector);
            ctx.provide(ServiceId::of::<dyn SettingsProjection>(), projector)?;
            Ok(FiberOutput)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// manifest 声明面：契约/服务键/ui 条目齐全（S1 验收口径，方案 §2.2）。
    #[test]
    fn manifest声明面字段齐全() {
        let m = CredentialsPlugin.manifest();
        assert_eq!(m.id, "yunyou-credentials");
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(m.contracts, vec!["credentials.v1".to_string(), "settings.v1".to_string()]);
        assert!(m.provides.contains(&"dyn CredentialService".to_string()), "凭据服务键应登记");
        assert!(m.provides.contains(&"dyn SettingsProjection".to_string()), "投影服务键应登记");
        assert!(m.inject.is_empty(), "无依赖");
        assert_eq!(
            m.ui,
            vec![yunyou::UiEntry::Settings { section: "credentials".into() }],
            "ui 条目应示范 settings 表单段（S8 冻结面）"
        );
    }

    /// F-4（挑刺报告 20261117）：组件侧 Config 拼错键要报错（与 config 镜像
    /// 段 deny 双轨对齐），不静默吞。
    #[test]
    fn 拼错键报错不静默吞() {
        let err = serde_json::from_value::<CredentialsConfig>(serde_json::json!({
            "store_patth": "x.toml"
        }))
        .expect_err("未知键 store_patth 必报");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    /// ui 条目经 validate_manifest 全绿（page 拒装等校验联动，S8）。
    #[test]
    fn manifest过装载前校验() {
        struct Probe;
        impl Plugin for Probe {
            fn name(&self) -> &str { "probe" }
            fn manifest(&self) -> PluginManifest { CredentialsPlugin.manifest() }
            fn inject(&self) -> Vec<ServiceId> { vec![] }
            fn setup(self: Arc<Self>, _ctx: Context, _cfg: ConfigValue)
                -> BoxFut<'static, Result<FiberOutput>> {
                Box::pin(async { Ok(FiberOutput) })
            }
        }
        // 同一类型参数下生成投影键，保证 trait inject 与 manifest 双向一致
        let mut m = CredentialsPlugin.manifest();
        m.provides = vec![
            projection_of::<dyn CredentialService>(),
            projection_of::<dyn SettingsProjection>(),
        ];
        assert!(
            yunyou::validate_manifest(&Probe, &m).is_empty(),
            "凭据插件 manifest 应通过装载前三条校验"
        );
    }

    /// config schema：store_path 有默认值（schema 驱动的表单可预填，S8 投影消费面）。
    #[test]
    fn config_schema含默认路径() {
        let schema = schemars::schema_for!(CredentialsConfig);
        let text = serde_json::to_string(&schema).expect("schema 可序列化");
        assert!(text.contains("store_path"), "字段名应在 schema 中");
        assert!(text.contains("credentials.toml"), "默认值应预填进 schema（default 面）");
    }
}
