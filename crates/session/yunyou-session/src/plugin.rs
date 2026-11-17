//! 会话插件：装载即注册 `SessionStore` 服务（JSONL 实现，路径来自配置）。

use crate::jsonl::JsonlStore;
use crate::SessionStore;
use yunyou::{ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId, projection_of};
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 会话插件配置。
// deny_unknown_fields（挑刺报告 20261117 F-4）：与 config 镜像段双轨对齐，拼错键报错。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    /// 会话 JSONL 文件路径（父目录自动创建）。
    #[serde(default = "default_path")]
    pub path: String,
}

fn default_path() -> String {
    "sessions/default.jsonl".into()
}

impl Default for SessionConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::Value::Object(serde_json::Map::new()))
            .expect("全默认值构造必然成功")
    }
}

/// 会话插件：注册 `dyn SessionStore` 服务。
///
/// # 依赖
/// 无。# 注册
/// `ctx.provide(ServiceId::of::<dyn SessionStore>(), JsonlStore::new(path))`。
/// 一次装载一个 store；多会话并行由调用方各建 JsonlStore 实例（不同路径）。
#[derive(Default)]
pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn name(&self) -> &str {
        "yunyou-session"
    }

    /// 声明面（方案 §2.2）：对齐 session.v1 插槽规格、provide `dyn SessionStore`、无依赖。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-session".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["session.v1".into()],
            provides: vec![projection_of::<dyn SessionStore>()],
            inject: vec![],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(serde_json::to_value(schemars::schema_for!(SessionConfig)).expect("schema 可序列化"))
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: SessionConfig = serde_json::from_value(config)
                .map_err(|err| yunyou::CordisError::new(yunyou::ErrorKind::Internal, format!("session/配置解析失败: {err}")))?;
            let store = JsonlStore::new(cfg.path);
            // 跨进程排他锁（task-36① 方案 A）：装配期抢锁——双实例并发写同一会话
            // 在启动期被拦下（人话报错上抛=装配失败），flock 随进程死自动释放零尸锁
            store.try_lock_exclusive()?;
            // 显式擦除：泛型推断会把 P 定为具体类型 JsonlStore，导致 get::<dyn SessionStore> 类型不符
            let service: Arc<dyn SessionStore> = store;
            ctx.provide(ServiceId::of::<dyn SessionStore>(), service)?;
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
        let m = SessionPlugin.manifest();
        assert_eq!(m.id, "yunyou-session");
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(m.contracts, vec!["session.v1".to_string()]);
        assert_eq!(m.provides, vec!["dyn SessionStore".to_string()]);
        assert!(m.inject.is_empty() && m.ui.is_empty());
    }

    /// manifest.inject 与 trait inject() 双向一致（装载前校验零违例）。
    #[test]
    fn manifest与trait_inject双向一致() {
        let plugin = SessionPlugin;
        let issues = yunyou::validate_manifest(&plugin, &plugin.manifest());
        assert!(issues.is_empty(), "校验违例: {issues:?}");
    }

    /// F-4（挑刺报告 20261117）：组件侧 Config 拼错键要报错（与 config 镜像
    /// 段 deny 双轨对齐），不静默吞。
    #[test]
    fn 拼错键报错不静默吞() {
        let err = serde_json::from_value::<SessionConfig>(serde_json::json!({
            "patth": "x.jsonl"
        }))
        .expect_err("未知键 patth 必报");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }
}
