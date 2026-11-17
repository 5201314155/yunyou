//! `role('secret')` 结构性脱敏（契约⑤/P1-2 最小版；官方
//! `packages/settings/settings/src/redact.ts` 同构平移）。
//!
//! schema 由 schemars 派生（字段级 `#[schemars(extend("role" = "secret"))]` 平铺为
//! JSON Schema 顶层 `role` 键）；walk 语义对齐官方 redact.ts（2026-11-14 实读）：
//! - 声明为 secret 的字段**整字段移除**，sidecar 记 `{path, set}`——配置面据此渲染
//!   「只写型密码框」而不接触值本身；
//! - object：按 `properties` 声明走子字段；值里 schema 未声明的**未知键原样保留**
//!   （官方 rebuilt[key] = entry 语义）；值缺失的 secret 槽位**仍枚举**（set=false，
//!   表单因此知道槽位存在）；
//! - array：按 `items` 走逐元素，path 带下标；
//! - anyOf/allOf：逐分支穿行，保守移除任一分支声明的 secret（官方 union/intersect
//!   reduce 语义）；
//! - 输入永不原地修改（返回重建副本）。
//!
//! serde_json 无 `undefined`：walk 以 `Option<Value>` 表达「移除」（None=键缺席）；
//! 官方 `value === undefined` 的 set 判定对应「值缺失或 JSON null」。

use serde_json::Value;
use std::collections::BTreeMap;

/// 一个 schema 声明的 secret 槽位（官方 RedactedSecret 同构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedSecret {
    /// 从段根到该字段的路径（对象键与数组下标均字符串化）。
    pub path: Vec<String>,
    /// 脱敏前该槽位是否持有值。
    pub set: bool,
}

/// 脱敏产物：移除全部 secret 字段后的值 + 槽位清单（官方 RedactedValue 同构）。
#[derive(Debug, Clone, PartialEq)]
pub struct RedactedValue {
    /// 移除 secret 字段后的副本。
    pub value: Value,
    /// 全部可达 secret 槽位（同路径合并，set 取或——官方 positions 语义）。
    pub secrets: Vec<RedactedSecret>,
}

/// walk 一层：`Some(重建值)` 或 `None`（本节点被移除/值缺失）。
///
/// 「缺失」表达：官方 walk 以 `undefined` 传播（缺值字段不产出键、深层 secret
/// 槽位仍枚举 set=false）；serde_json 无 undefined 且本链路输入源为 TOML（无
/// null 值），故 `Value::Null` 一律按「缺失」处理——leaf 缺值返回 None（不冒键），
/// object 缺值仍递归枚举子槽位（对齐官方 undefined 传播语义）。
fn walk(schema: Option<&Value>, value: &Value, path: &[String], secrets: &mut Vec<RedactedSecret>) -> Option<Value> {
    let Some(schema) = schema else {
        return if value.is_null() { None } else { Some(value.clone()) };
    };
    // 官方 node.meta.role === 'secret'：整字段移除（schemars extend 平铺在顶层 role 键）
    if schema.get("role").and_then(Value::as_str) == Some("secret") {
        secrets.push(RedactedSecret { path: path.to_vec(), set: !value.is_null() });
        return None;
    }
    match node_kind(schema) {
        NodeKind::Object => walk_object(schema, value, path, secrets),
        NodeKind::Array => walk_array(schema, value, path, secrets),
        NodeKind::Composite => walk_composite(schema, value, path, secrets),
        NodeKind::Leaf => {
            if value.is_null() { None } else { Some(value.clone()) }
        }
    }
}

/// schema 节点分类（官方 type 分支的 JSON Schema 对应物）。
enum NodeKind {
    Object,
    Array,
    /// 组合节点：anyOf/allOf（官方 union/intersect）。
    Composite,
    Leaf,
}

fn node_kind(schema: &Value) -> NodeKind {
    match schema.get("type").and_then(Value::as_str) {
        Some("object") | Some("array") => NodeKind::from_type(schema),
        _ => {
            if schema.get("anyOf").is_some() || schema.get("allOf").is_some() {
                NodeKind::Composite
            } else {
                NodeKind::Leaf
            }
        }
    }
}

impl NodeKind {
    fn from_type(schema: &Value) -> NodeKind {
        if schema.get("type").and_then(Value::as_str) == Some("object") {
            NodeKind::Object
        } else {
            NodeKind::Array
        }
    }
}

/// object：properties 声明字段走子 walk（值缺失仍枚举槽位但不产键）；未知键原样保留。
fn walk_object(schema: &Value, value: &Value, path: &[String], secrets: &mut Vec<RedactedSecret>) -> Option<Value> {
    let properties = match schema.get("properties").and_then(Value::as_object) {
        Some(properties) => properties,
        // 无 properties 声明的 object：无脱敏声明可走，原样通过
        None => return Some(value.clone()),
    };
    let source = match value.as_object() {
        Some(source) => source,
        // 值缺失（Null）：仍递归枚举深层 secret 槽位（set=false），但不产出键
        None => {
            for (key, child) in properties {
                let mut child_path = path.to_vec();
                child_path.push(key.clone());
                walk(Some(child), &Value::Null, &child_path, secrets);
            }
            return None;
        }
    };
    let mut rebuilt = serde_json::Map::new();
    // 未知键保留（官方：schema 未声明的键不属于脱敏语义，原样通过）
    for (key, entry) in source {
        if !properties.contains_key(key) {
            rebuilt.insert(key.clone(), entry.clone());
        }
    }
    for (key, child) in properties {
        let child_value = source.get(key).cloned().unwrap_or(Value::Null);
        let mut child_path = path.to_vec();
        child_path.push(key.clone());
        if let Some(stripped) = walk(Some(child), &child_value, &child_path, secrets) {
            rebuilt.insert(key.clone(), stripped);
        }
    }
    Some(Value::Object(rebuilt))
}

/// array：items 声明走逐元素，path 带下标（官方 node.inner walk 同构）。
fn walk_array(schema: &Value, value: &Value, path: &[String], secrets: &mut Vec<RedactedSecret>) -> Option<Value> {
    let items = schema.get("items");
    let source = match value.as_array() {
        Some(array) => array,
        None => return Some(value.clone()),
    };
    let mut rebuilt = Vec::with_capacity(source.len());
    for (index, entry) in source.iter().enumerate() {
        let mut item_path = path.to_vec();
        item_path.push(index.to_string());
        rebuilt.push(match walk(items, entry, &item_path, secrets) {
            Some(stripped) => stripped,
            // 元素被移除时以 null 占位保下标稳定（serde_json 无稀疏数组）
            None => Value::Null,
        });
    }
    Some(Value::Array(rebuilt))
}

/// anyOf/allOf：逐分支穿行，保守移除任一分支声明的 secret（官方 reduce 语义）。
fn walk_composite(schema: &Value, value: &Value, path: &[String], secrets: &mut Vec<RedactedSecret>) -> Option<Value> {
    let mut current = Some(value.clone());
    for key in ["anyOf", "allOf"] {
        if let Some(branches) = schema.get(key).and_then(Value::as_array) {
            for branch in branches {
                current = walk(Some(branch), current.as_ref().unwrap_or(&Value::Null), path, secrets);
                current.as_ref()?;
            }
        }
    }
    current
}

/// 移除 schema 声明的全部 `role('secret')` 字段（官方 redactSecrets 同构）。
///
/// 同路径多分支命中合并为一条（set 取或）；输入不被修改。
pub fn redact_secrets(schema: &Value, value: &Value) -> RedactedValue {
    let mut secrets = Vec::new();
    let stripped = walk(Some(schema), value, &[], &mut secrets);
    // 同路径合并（官方 positions Map：set 取或）
    let mut merged: BTreeMap<Vec<String>, bool> = BTreeMap::new();
    for secret in secrets {
        let slot = merged.entry(secret.path).or_insert(false);
        *slot = *slot || secret.set;
    }
    RedactedValue {
        value: stripped.unwrap_or(Value::Null),
        secrets: merged.into_iter().map(|(path, set)| RedactedSecret { path, set }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 平面对象：secret 字段整字段移除 + set 标注；普通字段原样；未知键保留。
    #[test]
    fn 平面脱敏与未知键保留() {
        let schema = json!({
            "type": "object",
            "properties": {
                "api_key": {"type": "string", "role": "secret"},
                "model": {"type": "string"}
            }
        });
        let value = json!({"api_key": "sk-abc", "model": "m1", "extra": 42});
        let redacted = redact_secrets(&schema, &value);
        assert_eq!(
            redacted.value,
            json!({"model": "m1", "extra": 42}),
            "secret 字段移除、普通字段与未知键原样"
        );
        assert_eq!(
            redacted.secrets,
            vec![RedactedSecret { path: vec!["api_key".into()], set: true }],
            "sidecar 应记路径与 set"
        );
    }

    /// 嵌套与数组：path 逐层展开，数组下标字符串化；缺值槽位仍枚举（set=false）。
    #[test]
    fn 嵌套数组与槽位枚举() {
        let schema = json!({
            "type": "object",
            "properties": {
                "providers": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "token": {"type": "string", "role": "secret"},
                            "name": {"type": "string"}
                        }
                    }
                },
                "deep": {
                    "type": "object",
                    "properties": {"key": {"type": "string", "role": "secret"}}
                }
            }
        });
        let value = json!({
            "providers": [{"token": "t1", "name": "a"}, {"name": "b"}],
            "deep": {}
        });
        let redacted = redact_secrets(&schema, &value);
        assert_eq!(
            redacted.value,
            json!({
                "providers": [{"name": "a"}, {"name": "b"}],
                "deep": {}
            }),
            "数组元素内 secret 移除；未设值的槽位不冒出键"
        );
        assert_eq!(
            redacted.secrets,
            vec![
                RedactedSecret { path: vec!["deep".into(), "key".into()], set: false },
                RedactedSecret { path: vec!["providers".into(), "0".into(), "token".into()], set: true },
                RedactedSecret { path: vec!["providers".into(), "1".into(), "token".into()], set: false },
            ],
            "存在元素内的缺值属性槽位也枚举（set=false，官方 object-properties-always 语义；BTreeMap 路径序）"
        );
    }

    /// anyOf 保守移除：任一分支声明 secret 即移除；同路径合并 set 取或。
    #[test]
    fn anyof分支保守移除与合并() {
        let schema = json!({
            "anyOf": [
                {"type": "object", "properties": {"credential": {"type": "string", "role": "secret"}}},
                {"type": "object", "properties": {"credential": {"type": "string", "role": "secret"}}}
            ]
        });
        let value = json!({"credential": "tok"});
        let redacted = redact_secrets(&schema, &value);
        assert_eq!(redacted.value, json!({}), "任一分支声明 secret 即移除");
        assert_eq!(
            redacted.secrets,
            vec![RedactedSecret { path: vec!["credential".into()], set: true }],
            "两分支同路径命中应合并为一条（官方 positions 语义）"
        );
    }

    /// 非 secret 输入零损伤：无 role 声明的 schema/标量原样通过（输入不被修改）。
    #[test]
    fn 非secret输入零损伤() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let value = json!({"a": "x"});
        let redacted = redact_secrets(&schema, &value);
        assert_eq!(redacted.value, json!({"a": "x"}));
        assert!(redacted.secrets.is_empty());
        // 标量：无 properties 可走，原样
        assert_eq!(redact_secrets(&json!({"type": "string"}), &json!("plain")).value, json!("plain"));
        // 数组元素是 secret：null 占位保下标（serde_json 无稀疏数组的取态）
        let schema = json!({"type": "array", "items": {"type": "string", "role": "secret"}});
        let redacted = redact_secrets(&schema, &json!(["a", "b"]));
        assert_eq!(redacted.value, json!([null, null]));
        assert_eq!(redacted.secrets.len(), 2);
    }
}
