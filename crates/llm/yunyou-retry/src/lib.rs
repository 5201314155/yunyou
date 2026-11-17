//! 重试策略插件（S10，任务18 第一张实战卡；《超时重试插件设计》§2 终裁）。
//!
//! 职责单一（策略自足，设计 §2「消费: 无」）：解析 `[retry]` 段配置 → 构造
//! [`yunyou_llm::RetryPolicy`] 纯类型 → provide `dyn RetryPolicyProvider` 服务。
//! 退避执行（sleep 与取消 select）在消费侧 `yunyou-agent-loop`（设计 §2
//! 「策略归调用侧」）；本 crate 零网络、零状态、零凭据（设计 §5「凭据无关」）。
//!
//! # 配置语义（设计 §5，default 即产品）
//! - `retry_on_timeout`（默认 false，裁决 D8 安全默认律）：true 时白名单自动
//!   追加 "timeout"——opt-in 便利字段，与手动写进 `retryable_codes` 等价；
//! - 插件启停由 yunyou.toml `[plugins.yunyou-retry] enable` 接管（S2 机制），
//!   设计 §5 的 `enabled` 字段不在此重复；禁用后消费方取不到服务＝失败直出
//!   直连（验收总口径「无重试直连，fail-fast 行为可预期」）。

use yunyou_llm::{RetryPolicy, RetryPolicyProvider};
use yunyou::{
    ConfigValue, Context, CordisError, ErrorKind, FiberOutput, Plugin, PluginLevel, PluginManifest,
    Result, ServiceId, projection_of,
};
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 插件配置（`[retry]` 段镜像，字段对齐 `yunyou-config::RetryConfig`；
/// 组件自带 Config 做 config_schema 投影是既有惯例，如 AgentLoopConfig）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RetryPluginConfig {
    /// 最多重试次数（默认 2 = 最多 3 次尝试）。
    pub max_retries: u32,
    /// 可重试类别码白名单（超时显式排除，安全默认律 D8）。
    pub retryable_codes: Vec<String>,
    /// 退避基数（毫秒）。
    pub initial_delay_ms: u64,
    /// 退避封顶（毫秒）。
    pub max_delay_ms: u64,
    /// 抖动比例（0..1）。
    pub jitter_ratio: f64,
    /// 超时是否重试（默认 false；true → 白名单追加 "timeout"）。
    pub retry_on_timeout: bool,
}

impl Default for RetryPluginConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            retryable_codes: vec![
                "connection".into(),
                "http_5xx".into(),
                "http_429".into(),
                "stream_interrupted".into(),
            ],
            initial_delay_ms: 500,
            max_delay_ms: 8000,
            jitter_ratio: 0.1,
            retry_on_timeout: false,
        }
    }
}

impl From<RetryPluginConfig> for RetryPolicy {
    fn from(cfg: RetryPluginConfig) -> Self {
        let mut codes = cfg.retryable_codes;
        if cfg.retry_on_timeout && !codes.iter().any(|c| c == "timeout") {
            codes.push("timeout".into());
        }
        Self {
            mode: yunyou_llm::RetryMode::Whitelist,
            max_retries: cfg.max_retries,
            retryable_codes: codes,
            initial_delay_ms: cfg.initial_delay_ms,
            max_delay_ms: cfg.max_delay_ms,
            jitter_ratio: cfg.jitter_ratio,
        }
    }
}

/// 重试策略插件（bundled，随框架分发；一期策略卡恒在，禁用走 `[plugins]`）。
#[derive(Default)]
pub struct RetryPlugin;

impl Plugin for RetryPlugin {
    fn name(&self) -> &str {
        "yunyou-retry"
    }

    /// 声明面（方案 §2.2）：零消费（策略自足），provide 重试策略缝。
    /// contracts=llm.v1：`RetryPolicy`/`LlmFailure` 类型真源在 yunyou-llm 契约
    /// crate，插槽规格随 llm.v1 版本化。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-retry".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["llm.v1".into()],
            provides: vec![projection_of::<dyn RetryPolicyProvider>()],
            inject: vec![],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![]
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(
            serde_json::to_value(schemars::schema_for!(RetryPluginConfig))
                .expect("schema 可序列化"),
        )
    }

    fn setup(
        self: Arc<Self>,
        ctx: Context,
        config: ConfigValue,
    ) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: RetryPluginConfig = serde_json::from_value(config).map_err(|err| {
                CordisError::new(ErrorKind::Internal, format!("retry/配置解析失败: {err}"))
            })?;
            // 显式标注 dyn 强转：provide 泛型按 value 推断 P，不标注会把具体类型
            // 存入服务表导致 get::<dyn _> downcast 失败（plugin.rs 同款惯例）。
            let policy: Arc<dyn RetryPolicyProvider> = Arc::new(RetryPolicy::from(cfg));
            ctx.provide(ServiceId::of::<dyn RetryPolicyProvider>(), policy)?;
            Ok(FiberOutput)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_llm::{LlmFailure, LlmFailureCategory};

    /// 按归一化类别码构造失败（测试便捷）。
    fn failure(code: &str) -> LlmFailure {
        let category =
            LlmFailureCategory::from_code(code).unwrap_or_else(|| panic!("合法类别码 {code}"));
        LlmFailure::new(category, "测试")
    }

    /// default 即产品：零配置构造的策略，默认白名单四类可重试、超时不在场。
    #[test]
    fn 默认配置零手填行为正确() {
        let policy = RetryPolicy::from(RetryPluginConfig::default());
        for code in ["connection", "http_5xx", "http_429", "stream_interrupted"] {
            assert!(policy.classify(&failure(code), 1).is_retry(), "{code} 默认应重试");
        }
        assert!(!policy.classify(&failure("timeout"), 1).is_retry(), "超时默认不重试（D8）");
    }

    /// retry_on_timeout=true 展开为白名单追加 "timeout"（opt-in 语义）。
    #[test]
    fn retry_on_timeout展开为白名单() {
        let cfg = RetryPluginConfig { retry_on_timeout: true, ..Default::default() };
        let policy = RetryPolicy::from(cfg);
        assert!(policy.classify(&failure("timeout"), 1).is_retry(), "opt-in 后超时应重试");
        // 幂等：重复展开不产生重复码
        let cfg2 = RetryPluginConfig {
            retry_on_timeout: true,
            retryable_codes: vec!["timeout".into()],
            ..Default::default()
        };
        assert_eq!(RetryPolicy::from(cfg2).retryable_codes, vec!["timeout"]);
    }

    /// manifest 声明面：零 inject、自供重试缝、contracts 挂 llm.v1、bundled。
    #[test]
    fn manifest声明面字段齐全() {
        let m = RetryPlugin.manifest();
        assert_eq!(m.id, "yunyou-retry");
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(m.provides, vec!["dyn RetryPolicyProvider".to_string()]);
        assert!(m.inject.is_empty(), "策略自足零消费");
        assert_eq!(m.contracts, vec!["llm.v1".to_string()]);
        assert!(m.ui.is_empty());
        let issues = yunyou::validate_manifest(&RetryPlugin, &m);
        assert!(issues.is_empty(), "manifest 校验违例: {issues:?}");
    }

    /// config_schema 可序列化且含 default（官方「装好即预置」律的投影前提）。
    #[test]
    fn config_schema含默认预填() {
        let schema = RetryPlugin.config_schema().expect("应有 schema");
        let text = serde_json::to_string(&schema).expect("可序列化");
        assert!(text.contains("retry_on_timeout"), "schema 应含 opt-in 字段");
        assert!(text.contains("retryable_codes"), "schema 应含白名单字段");
    }
}
