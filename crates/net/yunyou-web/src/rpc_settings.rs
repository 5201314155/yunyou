//! settings 域 unary RPC（W5 批，契约 web.v1 域-配置凭据与插件.md §1）：
//! `settings/sections`（段清单+revision）+ `settings/form`（schema 投影，
//! secret value/default 恒 null）+ `settings/write`（revision 乐观锁+深度校验）。
//!
//! # secret 纪律（契约 §4 红线，lead 2026-11-19 裁决）
//! settings/write 对 role:secret 路径**恒拒**并指路 credentials/set（单一专用
//! 写通道，不内嵌迁移逻辑）；本模块任何响应/投影序列化串**零明文**（测试锁死）。
//!
//! # 生效语义（裁决①）
//! applied 恒「需重启」诚实兜底（沙箱等段非热调；可热生效段实测后补）。

use crate::config_store::{section_schema, KNOWN_SECTIONS};
use crate::contract::{self, code, err_body_details, err_envelope};
use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;
use yunyou_credentials::{project_form, FormField, FormFieldType};

/// 段中文标题（sections 行；术语统一=设计规范 §三.5）。
fn section_title(section: &str) -> &'static str {
    match section {
        "llm" => "模型连接",
        "session" => "会话存储",
        "system_prompt" => "系统提示词",
        "tools" => "工具",
        "workspace" => "工作区",
        "agent_loop" => "主循环护栏",
        "retry" => "重试策略",
        "credentials" => "凭据托管",
        "sandbox" => "沙箱",
        "plugins" => "插件",
        _ => "设置",
    }
}

/// settings/sections（L1）：已知段清单（schema 可投影者），逐段带 revision。
pub(crate) fn settings_sections(state: &Arc<WebState>) -> Value {
    let revision = state.config_store.revision();
    let items: Vec<Value> = KNOWN_SECTIONS
        .iter()
        .filter(|section| section_schema(section).is_some())
        .map(|section| {
            json!({
                "section": section,
                "title": section_title(section),
                "revision": revision,
                "entry": {"kind": "settings"},
            })
        })
        .collect();
    json!({"ok": true, "value": {"items": items}})
}

/// 现配置中某段的当前值（文件缺席/段缺席=空对象——全字段走 schema default）。
fn section_value(state: &Arc<WebState>, section: &str) -> Value {
    std::fs::read_to_string(state.config_store.path())
        .ok()
        .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
        .and_then(|doc| doc.get(section.to_lowercase().as_str()).cloned())
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or_else(|| json!({}))
}

/// FormFieldType → 契约字段类型串（§1.2 fieldType 映射）。
fn field_type_name(field_type: FormFieldType) -> &'static str {
    match field_type {
        FormFieldType::String => "string",
        FormFieldType::Number => "number",
        FormFieldType::Integer => "integer",
        FormFieldType::Boolean => "boolean",
        FormFieldType::Array => "array",
        FormFieldType::Object => "object",
    }
}

/// FormField → 契约行（path 平铺数组；value/default 已由投影器脱敏）。
fn field_json(field: &FormField) -> Value {
    let mut row = json!({
        "path": field.path,
        "fieldType": field_type_name(field.field_type),
        "value": field.value,
        "default": field.default,
        "options": field.options,
        "secret": field.secret,
        "required": field.required,
    });
    if let Some(title) = &field.title {
        row["title"] = Value::String(title.clone());
    }
    if let Some(description) = &field.description {
        row["description"] = Value::String(description.clone());
    }
    row
}

/// settings/form（L1，payload {section!}）：schema 投影+当前值回显+secrets sidecar。
pub(crate) fn settings_form(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(section) = request.get("section").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "form 缺 section（settings/sections 查段清单）");
    };
    let Some(schema) = section_schema(section) else {
        return err_envelope(code::BAD_REQUEST, &format!("未知设置段 {section}（settings/sections 查段清单）"));
    };
    let current = section_value(state, section);
    let description = project_form(section, &schema, &current);
    let fields: Vec<Value> = description.fields.iter().map(field_json).collect();
    let secrets: Vec<Value> = description
        .secrets
        .iter()
        .map(|secret| json!({"path": secret.path, "set": secret.set}))
        .collect();
    json!({"ok": true, "value": {
        "section": section,
        "revision": state.config_store.revision(),
        "fields": fields,
        "secrets": secrets,
    }})
}

/// schema 中全部 secret 槽位路径（role:"secret" 扫描；顶层+嵌套对象递归）。
fn secret_paths(schema: &Value, prefix: &[String], out: &mut Vec<Vec<String>>) {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else { return };
    for (key, child) in properties {
        let mut path: Vec<String> = prefix.iter().map(|p| p.to_string()).collect();
        path.push(key.clone());
        if child.get("role").and_then(Value::as_str) == Some("secret") {
            out.push(path);
        } else {
            secret_paths(child, &path, out);
        }
    }
}

/// value 是否触及任一 secret 槽位（提交体含 secret 键=走专用写通道）。
fn touches_secret(value: &Value, secrets: &[Vec<String>]) -> Option<Vec<String>> {
    for path in secrets {
        let mut cursor = value;
        let mut hit = true;
        for key in path {
            match cursor.get(key.as_str()) {
                Some(child) => cursor = child,
                None => {
                    hit = false;
                    break;
                }
            }
        }
        if hit {
            return Some(path.clone());
        }
    }
    None
}

/// settings/write（L4，payload {section!,revision!,value!}）：revision 乐观锁 →
/// secret 恒拒（指路 credentials/set）→ 深度校验 → 原子写 → revision+1。
pub(crate) fn settings_write(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(section) = request.get("section").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "write 缺 section");
    };
    let Some(revision) = request.get("revision").and_then(Value::as_u64) else {
        return err_envelope(code::BAD_REQUEST, "write 缺 revision（乐观锁必填）");
    };
    let Some(value) = request.get("value") else {
        return err_envelope(code::BAD_REQUEST, "write 缺 value（段值对象）");
    };
    let Some(schema) = section_schema(section) else {
        return err_envelope(code::BAD_REQUEST, &format!("未知设置段 {section}（settings/sections 查段清单）"));
    };
    if let Err(current) = state.config_store.check_revision(revision) {
        let mut details = serde_json::Map::new();
        details.insert("current".into(), json!(current));
        return json!({"ok": false, "error": err_body_details(
            code::REVISION_CONFLICT,
            "配置已在别处更新：请重新加载后再写（settings/form 取最新 revision）",
            details,
        )});
    }
    // secret 红线：提交体触及 role:secret 槽位=恒拒（单一专用写通道，lead 裁决）
    let mut secrets = Vec::new();
    secret_paths(&schema, &[], &mut secrets);
    if let Some(path) = touches_secret(value, &secrets) {
        let mut details = serde_json::Map::new();
        details.insert("path".into(), json!(path));
        return json!({"ok": false, "error": err_body_details(
            code::CREDENTIALS_WRITE_ONLY,
            "secret 字段不经 settings/write：请走 credentials/set（值只进托管存储，永不回显）",
            details,
        )});
    }
    match state.config_store.write_section(section, value) {
        Ok(revision) => json!({"ok": true, "value": {
            "revision": revision,
            "applied": ["需重启"],
            "restartRequired": true,
        }}),
        Err(keys) => {
            let mut details = serde_json::Map::new();
            details.insert("keys".into(), json!(keys));
            json!({"ok": false, "error": err_body_details(
                code::CONFIG_VALIDATION,
                "配置段深度校验失败（未知键，deny_unknown_fields 拼写防呆）",
                details,
            )})
        }
    }
}

#[cfg(test)]
#[path = "rpc_settings_tests.rs"]
mod rpc_settings_tests;
