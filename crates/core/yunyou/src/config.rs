//! 配置载体、intercept 合并与 schema 校验（方案 §1.7 / §2.2）。
//!
//! 载体统一为 [`serde_json::Value`]；schema 用 JSON Schema 表达——组件侧由
//! schemars 派生后转 `Value` 传入 [`crate::registry::Plugin::config_schema`]，
//! 内核用 jsonschema 引擎执行校验，错误聚合为 [`ValidationError`]
//! （对齐 fiber.ts:19-36 的 ValidationError 多问题聚合）。

use crate::error::{CordisError, ErrorKind};
use serde_json::Value;
use std::fmt;

/// 配置统一载体。
pub type ConfigValue = Value;

/// 单条校验问题：字段路径 + 说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationEntry {
    /// 字段路径（JSON Pointer 风格点分展示；根为空串）。
    pub path: String,
    /// 问题描述。
    pub message: String,
}

/// 配置校验失败：聚合全部问题（对齐 TS `invalid config:` 的多行格式）。
#[derive(Debug, Clone)]
pub struct ValidationError {
    entries: Vec<ValidationEntry>,
}

impl ValidationError {
    /// 用一批校验问题构造。
    pub fn new(entries: Vec<ValidationEntry>) -> Self {
        Self { entries }
    }

    /// 全部校验问题。
    pub fn entries(&self) -> &[ValidationEntry] {
        &self.entries
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid config:")?;
        for entry in &self.entries {
            if entry.path.is_empty() {
                write!(f, "\n  - {}", entry.message)?;
            } else {
                write!(f, "\n  - {} (at {})", entry.message, entry.path)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

impl From<ValidationError> for CordisError {
    fn from(value: ValidationError) -> Self {
        CordisError::new(ErrorKind::ConfigValidation, value.to_string())
    }
}

/// 校验 `config` 是否满足给定的 JSON Schema。
///
/// schema 自身非法也归入校验失败（带说明），避免静默放过未校验的配置。
pub fn validate_schema(schema: &Value, config: &Value) -> Result<(), ValidationError> {
    use jsonschema::ValidationError as SchemaError;
    let validator = jsonschema::validator_for(schema).map_err(|e: SchemaError| {
        ValidationError::new(vec![ValidationEntry {
            path: String::new(),
            message: format!("schema 本身无效：{e}"),
        }])
    })?;
    let entries: Vec<ValidationEntry> = validator
        .iter_errors(config)
        .map(|e| ValidationEntry {
            path: e.instance_path().to_string(),
            message: e.to_string(),
        })
        .collect();
    if entries.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::new(entries))
    }
}

/// 浅合并多段配置（对齐 TS `Object.assign({}, ...configs)`：右覆盖左，仅对象顶层）。
///
/// 方案 §2.2：语义合并（`Config.merge` 按字段可配）一期不实现，全走浅合并；
/// 非 object 段之间整体互相覆盖（TS 会把字符串按索引展开，该行为不模仿，见差异说明）。
pub fn merge_configs(configs: &[&Value]) -> Value {
    let mut out = serde_json::Map::new();
    let mut last_scalar: Option<&Value> = None;
    for config in configs {
        match config {
            Value::Object(map) => {
                // 回到对象段后，先前累积的标量段作废
                last_scalar = None;
                for (key, value) in map {
                    out.insert(key.clone(), value.clone());
                }
            }
            Value::Null => {}
            other => last_scalar = Some(other),
        }
    }
    last_scalar
        .cloned()
        .unwrap_or(Value::Object(out))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试断言惯用 unwrap/expect/expect_err；mod 级单点豁免（S-ISO-1 硬门口径）
mod tests {
    use super::{merge_configs, validate_schema, ValidationEntry, ValidationError};
    use serde_json::{json, Value};

    #[test]
    fn 浅合并右侧覆盖左侧() {
        let base = json!({"a": 1, "b": {"deep": 1}, "keep": true});
        let intercept = json!({"b": {"deep": 2}, "c": 3});
        let merged = merge_configs(&[&base, &intercept]);
        // 浅合并：对象段整体覆盖，不做字段级深合并（对齐 Object.assign）
        assert_eq!(
            merged,
            json!({"a": 1, "b": {"deep": 2}, "keep": true, "c": 3})
        );
    }

    #[test]
    fn 标量段整体覆盖且空段跳过() {
        let merged = merge_configs(&[&json!({"a": 1}), &Value::Null, &json!("raw")]);
        assert_eq!(merged, json!("raw"));
        let merged2 = merge_configs(&[&json!("raw"), &json!({"a": 1})]);
        assert_eq!(merged2, json!({"a": 1}));
        let empty = merge_configs(&[]);
        assert_eq!(empty, json!({}));
    }

    #[test]
    fn schema_校验聚合多条问题() {
        let schema = json!({
            "type": "object",
            "properties": {
                "step": {"type": "integer"},
                "name": {"type": "string", "minLength": 2}
            },
            "required": ["step", "name"]
        });
        let bad = json!({"step": "NaN", "name": ""});
        let err = validate_schema(&schema, &bad).expect_err("应校验失败");
        // 聚合：类型错 + minLength 违例 + required 缺失等，至少 2 条
        assert!(err.entries().len() >= 2, "应聚合多条问题：{err}");
        let text = err.to_string();
        assert!(text.starts_with("invalid config:"), "展示格式对齐 TS：{text}");
    }

    #[test]
    fn 校验通过与格式化路径() {
        let schema = json!({"type": "object", "properties": {"step": {"type": "integer"}}});
        assert!(validate_schema(&schema, &json!({"step": 2})).is_ok());
        let err = validate_schema(&schema, &json!({"step": "x"})).expect_err("应失败");
        let entry: &ValidationEntry = &err.entries()[0];
        assert_eq!(entry.path, "/step");
        // From 转 CordisError
        let cordis: crate::error::CordisError = ValidationError::new(err.entries().to_vec()).into();
        assert_eq!(cordis.kind(), &crate::error::ErrorKind::ConfigValidation);
    }
}
