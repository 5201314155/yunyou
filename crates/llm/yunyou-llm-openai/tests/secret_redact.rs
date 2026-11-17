//! 投影消费断言（审二轮 P1「防线断链」回归）：`OpenAiConfig.api_key` 的
//! `role=secret` 标注必须经 credentials 的 redact/投影面消费生效——装配层注入
//! 明文真值的就是这个字段，标注缺失时 project_form 会把密钥回显进配置面。

use yunyou_credentials::{project_form, redact_secrets};

/// schema 侧：api_key 带 role=secret；值面带「已配置密钥」。
fn secret_schema_and_value() -> (serde_json::Value, serde_json::Value) {
    let schema = serde_json::to_value(schemars::schema_for!(yunyou_llm_openai::OpenAiConfig))
        .expect("OpenAiConfig schema 可序列化");
    let value = serde_json::json!({"api_key": "sk-live-abc", "model": "test-model"});
    (schema, value)
}

/// redact 消费：api_key 整字段移除 + sidecar 记槽位（配置面只见「已设置」事实）。
#[test]
fn api_key经redact整字段移除() {
    let (schema, value) = secret_schema_and_value();
    let redacted = redact_secrets(&schema, &value);
    assert!(
        redacted.value.get("api_key").is_none(),
        "api_key 必须被整字段移除（防线断链回归：标注缺失时此断言红）"
    );
    assert!(
        redacted.secrets.iter().any(|s| s.path == vec!["api_key".to_string()] && s.set),
        "sidecar 应记 api_key 槽位且 set=true：{:?}",
        redacted.secrets
    );
}

/// 投影消费：密码框标注生效、值与 default 双空（密钥不回显不预填，官方 plainSchema）。
#[test]
fn api_key投影面不回显不预填() {
    let (schema, value) = secret_schema_and_value();
    let desc = project_form("llm", &schema, &value);
    let field = desc
        .fields
        .iter()
        .find(|f| f.path.last().map(String::as_str) == Some("api_key"))
        .expect("api_key 字段应在投影中");
    assert!(field.secret, "密码框标注必须生效（防线断链回归：标注缺失时此断言红）");
    assert_eq!(field.value, None, "投影值面不得回显密钥");
    assert_eq!(field.default, None, "投影不得预填密钥");
    assert!(!field.required, "secret 字段不标 required（可留空）");
    // 面对称性：非 secret 字段照常回显（防线只对密钥，不误伤普通字段）
    let model = desc
        .fields
        .iter()
        .find(|f| f.path.last().map(String::as_str) == Some("model"))
        .expect("model 字段应在投影中");
    assert!(!model.secret && model.value == Some(serde_json::json!("test-model")), "普通字段不脱敏");
}
