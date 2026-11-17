//! `credentials` 段：凭据托管存储配置（S5，方案 §6.1「新 yunyou-credentials crate
//! ＋config 接线」）。
//!
//! 类型**自持定义**（对齐既有五段模式：`LlmConfig` 字段对齐 `OpenAiConfig` 亦为
//! config 侧复制定义）——依赖方向是组件→config（匠二 workspace 登记），反向将成环；
//! 段值经装配层透传给 `CredentialsPlugin`，深度校验由组件自身 config_schema 承担
//!（方案 §2.2）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `credentials` 段：凭据托管存储（字段对齐 yunyou-credentials::CredentialsConfig）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CredentialsConfig {
    /// 凭据存储文件路径；缺省时由 yunyou-cli 落到
    /// `<yunyou-home>/credentials.toml`（装配时补值；600 权限，见凭据 crate store 模块）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::DshConfig;

    /// credentials 段反序列化：显式路径落值、缺段落默认（None→装配补 home 值）。
    #[test]
    fn credentials_段反序列化与缺省() {
        let text = "[credentials]\nstore_path = \"/tmp/cred.toml\"\n";
        let cfg: DshConfig = toml::from_str(text).expect("credentials 已知段应可反序列化");
        assert_eq!(
            cfg.credentials.store_path.as_deref(),
            Some("/tmp/cred.toml"),
            "显式 store_path 应落值"
        );
        let default_cfg = DshConfig::default();
        assert_eq!(default_cfg.credentials.store_path, None, "缺省应落 None（装配补值）");
    }

    /// S5/S8 消费面：`llm.api_key` 的 `role=secret` schema 标注生效——redact
    /// 结构性脱敏与 settings 投影密码框标注凭此词汇自动生效（字段在 model.rs，
    /// 消费面测试随本文件域：本 crate 即标注词汇的消费方）。
    #[test]
    fn llm_api_key_schema标注role_secret() {
        let schema = schemars::schema_for!(crate::model::LlmConfig);
        let text = serde_json::to_string(&schema).expect("schema 可序列化");
        assert!(
            text.contains("\"role\":\"secret\"") || text.contains("\"role\": \"secret\""),
            "api_key 的 schema 应带 role=secret 标注：{text}"
        );
    }
}
