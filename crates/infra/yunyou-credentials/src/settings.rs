//! settings 表单投影（契约④，方案 §3-④/§4.1；并入本 crate 的裁决理由见 lib.rs 顶注）。
//!
//! 官方对照（`packages/settings/settings/src/schema.ts`，2026-11-14 实读）：
//! - 投影输入=schemars 派生的 JSON Schema＋当前配置值，输出=表单描述（一期无 UI 面，
//!   投影数据面就位即达标，方案 §4.1「零界面代码」；CLI 侧启动时展示即可）；
//! - `plainSchema` 语义：`role('secret')` 字段**删 default 与 required**（密码框只写型：
//!   永不回显预填，可留空）——本实现同构；
//! - `volatileForm`/`projectForm`/`isVolatilePath`（volatile 热编辑面）一期不做：
//!   volatile 底座已在内核（S6），投影侧接入随热编辑面立项（注释即预留位）。
//!
//! 值面与脱敏联动：`FormField.value` 不含 secret 值（密码框标注+sidecar 见
//! [`crate::redact`]），同一 schema 走 [`crate::redact::redact_secrets`] 产出
//! `secrets` 清单供配置面渲染「槽位存在但已脱敏」。

use crate::redact::{redact_secrets, RedactedSecret};
use serde_json::Value;

/// 表单字段类型（schemars type → 表单控件类型的一期映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFieldType {
    /// 字符串（secret 时宿主渲染为密码框）。
    String,
    /// 浮点数。
    Number,
    /// 整数。
    Integer,
    /// 布尔。
    Boolean,
    /// 数组（元素级投影二期）。
    Array,
    /// 对象（子字段展平，本行作分组占位）。
    Object,
}

/// 一个表单字段（官方 plainSchema/projectForm 一期裁剪合成的平面描述）。
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    /// 从段根到该字段的路径（嵌套对象逐层键名）。
    pub path: Vec<String>,
    /// 字段类型（宿主据此选控件；secret + String = 密码框）。
    pub field_type: FormFieldType,
    /// 标题（schema title；缺省由宿主用 path 尾段兜底）。
    pub title: Option<String>,
    /// 说明文本（schema description）。
    pub description: Option<String>,
    /// 当前配置值回显（**secret 恒 None**——值不进配置面）。
    pub value: Option<Value>,
    /// schema default 预填（**secret 恒 None**，官方 plainSchema 删 default 语义）。
    pub default: Option<Value>,
    /// 枚举下拉选项（schema enum，字符串化展示）。
    pub options: Vec<String>,
    /// 密码框标注（schema `role: "secret"`）。
    pub secret: bool,
    /// 必填（schema required；**secret 恒 false**，官方 plainSchema 删 required 语义）。
    pub required: bool,
}

/// 一个设置段的表单描述：字段清单＋脱敏 sidecar（契约④输出面）。
#[derive(Debug, Clone, PartialEq)]
pub struct FormDescription {
    /// 段名（对应 manifest ui 条目的 `section`）。
    pub section: String,
    /// 平铺字段清单（嵌套对象递归展平，`path` 表层级）。
    pub fields: Vec<FormField>,
    /// 脱敏 sidecar：全部 secret 槽位及其 set 事实（来自 [`crate::redact`]）。
    pub secrets: Vec<RedactedSecret>,
}

/// 投影设置投影服务（契约④服务形态）：宿主/App 面经 Context 服务注入消费
/// （同一份投影，CLI 一期展示、App 二期渲染，方案 §4.2）。
pub trait SettingsProjection: Send + Sync + 'static {
    /// 把一个插件的 config schema＋当前值投影成表单描述。
    fn project(&self, section: &str, schema: &Value, value: &Value) -> FormDescription;
}

/// 投影器实现：无状态纯投影（逻辑在 [`project_form`]，trait 只为服务键而设）。
#[derive(Debug, Default, Clone, Copy)]
pub struct SettingsProjector;

impl SettingsProjection for SettingsProjector {
    fn project(&self, section: &str, schema: &Value, value: &Value) -> FormDescription {
        project_form(section, schema, value)
    }
}

/// 投影纯函数：schema→字段清单（default 预填/枚举下拉/secret 密码框标注）＋值回显＋
/// 脱敏 sidecar。`value` 缺省传空对象即可（全字段走 default 预填）。
pub fn project_form(section: &str, schema: &Value, value: &Value) -> FormDescription {
    let mut fields = Vec::new();
    let value = ensure_object(value);
    // 脱敏 sidecar 与字段面同源（同一 schema 词汇：role = "secret"）
    let secrets = redact_secrets(schema, &value).secrets;
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        for (key, child) in properties {
            project_field(&mut fields, Vec::new(), key, child, &value, required.contains(&key.as_str()));
        }
    }
    FormDescription { section: section.to_string(), fields, secrets }
}

/// 值面兜底：非对象值按空对象处理（投影只走对象字段面）。
fn ensure_object(value: &Value) -> Value {
    if value.is_object() { value.clone() } else { Value::Object(serde_json::Map::new()) }
}

/// 递归投影一个字段（嵌套 object 展平子字段；array 停在自身，元素级投影二期）。
fn project_field(
    fields: &mut Vec<FormField>,
    prefix: Vec<String>,
    key: &str,
    schema: &Value,
    value: &Value,
    required: bool,
) {
    let mut path = prefix;
    path.push(key.to_string());
    let field_type = field_type_of(schema);
    let secret = schema.get("role").and_then(Value::as_str) == Some("secret");
    let current = value.get(key).cloned().filter(|v| !v.is_null());
    fields.push(FormField {
        path: path.clone(),
        field_type,
        title: schema.get("title").and_then(Value::as_str).map(str::to_string),
        description: schema.get("description").and_then(Value::as_str).map(str::to_string),
        // 值面：secret 恒不回显（值不进配置面）；缺值字段不冒 None 噪声
        value: if secret { None } else { current.clone() },
        // 官方 plainSchema：secret 删 default（永不预填）
        default: if secret { None } else { schema.get("default").cloned().filter(|v| !v.is_null()) },
        options: schema
            .get("enum")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(option_label).collect())
            .unwrap_or_default(),
        secret,
        // 官方 plainSchema：secret 删 required（可留空）
        required: required && !secret,
    });
    // 嵌套对象：子字段展平（本字段行作分组占位；值缺失传空对象走 default 预填面）
    if field_type == FormFieldType::Object {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            let child_required = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_default();
            let child_value = current.clone().unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            for (name, grandchild) in properties {
                project_field(fields, path.clone(), name, grandchild, &child_value, child_required.contains(&name.as_str()));
            }
        }
    }
}

/// schemars type → 表单控件类型（缺 type 视为字符串；带 enum 仍是 String+下拉）。
fn field_type_of(schema: &Value) -> FormFieldType {
    match schema.get("type").and_then(Value::as_str) {
        Some("number") => FormFieldType::Number,
        Some("integer") => FormFieldType::Integer,
        Some("boolean") => FormFieldType::Boolean,
        Some("array") => FormFieldType::Array,
        Some("object") => FormFieldType::Object,
        _ => FormFieldType::String,
    }
}

/// 枚举选项字符串化（数字/布尔枚举也按展示形式给宿主下拉）。
fn option_label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 平面字段：类型/default 预填/枚举下拉/required——投影数据面主路径。
    #[test]
    fn 平面投影四件() {
        let schema = json!({
            "type": "object",
            "properties": {
                "model": {"type": "string", "default": "m1", "description": "模型名"},
                "level": {"type": "string", "enum": ["a", "b"]},
                "retries": {"type": "integer", "default": 3},
                "strict": {"type": "boolean", "required": true}
            },
            "required": ["model"]
        });
        let desc = project_form("llm", &schema, &json!({"model": "live-model"}));
        assert_eq!(desc.section, "llm");
        let by_path = |name: &str| desc.fields.iter().find(|f| f.path.last().map(String::as_str) == Some(name)).expect("字段应存在");
        let model = by_path("model");
        assert_eq!(model.value, Some(json!("live-model")), "当前值应回显");
        assert_eq!(model.default, Some(json!("m1")), "default 应预填");
        assert!(model.required, "schema required 应标注");
        assert_eq!(model.field_type, FormFieldType::String);
        let level = by_path("level");
        assert_eq!(level.options, vec!["a".to_string(), "b".to_string()], "枚举应成下拉");
        assert_eq!(by_path("retries").field_type, FormFieldType::Integer);
        assert_eq!(by_path("strict").field_type, FormFieldType::Boolean);
    }

    /// secret 字段：密码框标注、值与 default 双空、required 摘除（官方 plainSchema 同构）。
    #[test]
    fn secret密码框标注与只写面() {
        let schema = json!({
            "type": "object",
            "properties": {
                "api_key": {"type": "string", "role": "secret", "default": "预填必删", "required": true}
            },
            "required": ["api_key"]
        });
        let desc = project_form("llm", &schema, &json!({"api_key": "sk-live"}));
        let field = &desc.fields[0];
        assert!(field.secret, "role=secret 应标注密码框");
        assert_eq!(field.value, None, "secret 当前值不得进配置面");
        assert_eq!(field.default, None, "secret default 必删（官方 plainSchema）");
        assert!(!field.required, "secret required 必删（可留空）");
        // sidecar 联动：槽位存在且已设值
        assert_eq!(desc.secrets.len(), 1);
        assert!(desc.secrets[0].set, "值面经 redact sidecar 报 set 事实");
    }

    /// 嵌套对象展平：path 逐层；子对象缺值走子字段 default 预填面。
    #[test]
    fn 嵌套对象展平与缺值预填() {
        let schema = json!({
            "type": "object",
            "properties": {
                "inner": {
                    "type": "object",
                    "properties": {"leaf": {"type": "string", "default": "d"}}
                }
            }
        });
        let desc = project_form("s", &schema, &json!({}));
        let paths: Vec<Vec<String>> = desc.fields.iter().map(|f| f.path.clone()).collect();
        assert_eq!(paths, vec![vec!["inner"], vec!["inner", "leaf"]], "嵌套应展平且保留分组行");
        let leaf = desc.fields.last().expect("leaf 应存在");
        assert_eq!(leaf.default, Some(json!("d")), "缺值时 default 预填面应可用");
        assert_eq!(leaf.value, None);
    }

    /// 值缺失/非对象值兜底：不冒 null 噪声；数组停在自身（元素级投影二期）。
    #[test]
    fn 缺值与非对象值兜底() {
        let schema = json!({
            "type": "object",
            "properties": {
                "list": {"type": "array", "items": {"type": "string"}},
                "plain": {"type": "string"}
            }
        });
        let desc = project_form("s", &schema, &json!({"list": ["a"]}));
        let list = &desc.fields[0];
        assert_eq!(list.field_type, FormFieldType::Array);
        assert_eq!(list.value, Some(json!(["a"])));
        let plain = &desc.fields[1];
        assert_eq!(plain.value, None, "缺值不冒 null 噪声");
        // 非对象值整体兜底为空对象（投影只走字段面）
        let desc = project_form("s", &schema, &json!("not-an-object"));
        assert!(desc.fields.iter().all(|f| f.value.is_none()), "非对象值面应全空");
    }
}
