//! plugins 域 unary RPC（W6 批，契约 web.v1 域-配置凭据与插件.md §4）：
//! `plugins/list`（磁盘 manifest ∪ config [plugins] 合并投影）+
//! `plugins/enable`（启停写 [plugins] 段，revision 乐观锁）+
//! `plugins/config`（读/写插件段配置）+ `plugins/tools`（方案 A：诚实空 items
//! +status.errors 人话指路——registry 无 per-plugin 归属，bundled 归属投影
//! 挂后续批，lead 2026-11-19 裁决）。
//!
//! # 装载时序语义（契约 §4.3）
//! 写入落配置即响应；装载/拒装结果经 `config/reloaded` 事件（Events 流广播）
//! 与下次 list 反映——本面不改运行态，不伪造即时生效。

use crate::contract::{self, code, err_envelope};
use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;

/// 磁盘插件清单（discover_plugins 投影；目录缺席=空——bundled 不在磁盘）。
fn discovered(state: &Arc<WebState>) -> Vec<yunyou_config::manifest_file::DiscoveredPlugin> {
    let dir = state.config_store.path().with_file_name("plugins");
    yunyou_config::manifest_file::discover_plugins(&dir)
        .map(|found| found)
        .unwrap_or_default()
}

/// config [plugins] 段值（缺席=空对象）。
fn plugins_config(state: &Arc<WebState>) -> Value {
    std::fs::read_to_string(state.config_store.path())
        .ok()
        .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
        .and_then(|doc| doc.get("plugins").cloned())
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or_else(|| json!({}))
}

/// 某插件 enabled：配置 enable ?? manifest.auto_enable（契约 §4.1）。
fn enabled_of(config: &Value, id: &str, auto_enable: bool) -> bool {
    config
        .get(id)
        .and_then(|entry| entry.get("enable"))
        .and_then(Value::as_bool)
        .unwrap_or(auto_enable)
}

/// plugins/list（L1）：合并投影（磁盘 manifest 行+bundled/内置占位候共享清单）。
pub(crate) fn plugins_list(state: &Arc<WebState>) -> Value {
    let config = plugins_config(state);
    let revision = state.config_store.revision();
    let mut seen: Vec<String> = Vec::new();
    let mut items: Vec<Value> = Vec::new();
    for plugin in discovered(state) {
        let manifest = &plugin.manifest;
        seen.push(manifest.id.clone());
        items.push(json!({
            "id": manifest.id,
            "version": manifest.version,
            "level": format!("{:?}", manifest.level).to_lowercase(),
            "contracts": manifest.contracts,
            "provides": manifest.provides,
            "inject": manifest.inject,
            "ui": manifest.ui,
            "autoEnable": manifest.auto_enable,
            "enabled": enabled_of(&config, &manifest.id, manifest.auto_enable),
            "configRevision": revision,
        }));
    }
    // bundled 内置插件（不在磁盘目录）：config [plugins] 已列条目诚实投影
    // （manifest 九字段缺=值缺席不伪造；完整 bundled 清单候 yunyou-plugin-manage 下沉批）
    if let Some(entries) = config.as_object() {
        for (id, entry) in entries {
            if seen.contains(id) {
                continue;
            }
            items.push(json!({
                "id": id,
                "enabled": entry.get("enable").and_then(Value::as_bool),
                "configRevision": revision,
            }));
        }
    }
    json!({"ok": true, "value": {"items": items}})
}

/// 写 [plugins] 段后广播 config/reloaded（websocket.md §4：revision 作废缓存）。
fn notify_config_reloaded(state: &Arc<WebState>) {
    state.wire.broadcast_event(
        crate::bridge::StreamKind::Events,
        json!({"type": "config/reloaded", "revision": state.config_store.revision()}),
    );
}

/// plugins/enable（L3，前端二次确认；payload {pluginId!,enable!,revision!}）。
pub(crate) fn plugins_enable(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(plugin_id) = request.get("pluginId").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "enable 缺 pluginId");
    };
    let Some(enable) = request.get("enable").and_then(Value::as_bool) else {
        return err_envelope(code::BAD_REQUEST, "enable 缺 enable（布尔）");
    };
    let Some(revision) = request.get("revision").and_then(Value::as_u64) else {
        return err_envelope(code::BAD_REQUEST, "enable 缺 revision（乐观锁必填）");
    };
    if let Err(current) = state.config_store.check_revision(revision) {
        let mut details = serde_json::Map::new();
        details.insert("current".into(), json!(current));
        return json!({"ok": false, "error": serde_json::json!({
            "code": code::REVISION_CONFLICT,
            "message": "配置已在别处更新：请重新加载后再写",
            "details": details,
        })});
    }
    // 写 [plugins.<id>] 条目（保留该插件既有 config 键——浅合并）
    let mut config = plugins_config(state);
    let entry = config
        .as_object_mut()
        .and_then(|object| object.get_mut(plugin_id).cloned())
        .unwrap_or_else(|| json!({}));
    let mut entry = entry;
    entry["enable"] = json!(enable);
    match state.config_store.write_section("plugins", &config) {
        Ok(_) => {
            notify_config_reloaded(state);
            json!({"ok": true, "value": {"pluginId": plugin_id, "enable": enable, "note":
                "已落配置；装载/卸载经下次装配生效（config/reloaded 事件已推）"}})
        }
        Err(keys) => json!({"ok": false, "error": serde_json::json!({
            "code": code::CONFIG_VALIDATION,
            "message": "插件段写入深度校验失败（deny_unknown_fields 拼写防呆）",
            "details": {"keys": keys},
        })}),
    }
}

/// plugins/config（L3；payload {pluginId!,revision?,config?}——config 省略=读）。
pub(crate) fn plugins_config_read_write(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(plugin_id) = request.get("pluginId").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "config 缺 pluginId");
    };
    let config = plugins_config(state);
    match request.get("config") {
        None => {
            // 读：该插件段值（缺席=空对象诚实）
            let value = config.get(plugin_id).cloned().unwrap_or_else(|| json!({}));
            json!({"ok": true, "value": {"pluginId": plugin_id, "config": value,
                "revision": state.config_store.revision()}})
        }
        Some(config_value) => {
            let Some(revision) = request.get("revision").and_then(Value::as_u64) else {
                return err_envelope(code::BAD_REQUEST, "写配置必带 revision（乐观锁）");
            };
            if let Err(current) = state.config_store.check_revision(revision) {
                let mut details = serde_json::Map::new();
                details.insert("current".into(), json!(current));
                return json!({"ok": false, "error": serde_json::json!({
                    "code": code::REVISION_CONFLICT,
                    "message": "配置已在别处更新：请重新加载后再写",
                    "details": details,
                })});
            }
            let mut merged = config;
            merged[plugin_id] = config_value.clone();
            match state.config_store.write_section("plugins", &merged) {
                Ok(_) => {
                    notify_config_reloaded(state);
                    json!({"ok": true, "value": {"pluginId": plugin_id, "note":
                        "已落配置；装载/拒装经下次装配生效（ui kind=page 拒装=plugin/ui-page-rejected）"}})
                }
                Err(keys) => json!({"ok": false, "error": serde_json::json!({
                    "code": code::CONFIG_VALIDATION,
                    "message": "插件段写入深度校验失败（deny_unknown_fields 拼写防呆）",
                    "details": {"keys": keys},
                })}),
            }
        }
    }
}

/// plugins/tools（L1，方案 A）：诚实空 items+status.errors 人话指路。
pub(crate) fn plugins_tools(state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let plugin_id = request.get("pluginId").and_then(Value::as_str).unwrap_or("");
    json!({"ok": true, "value": {
        "items": [],
        "status": {"running": false, "errors": [format!(
            "插件 {plugin_id} 工具清单暂不可投影：内置工具归属面未建（后续批）——查看启动日志（yunyou web 加 --verbose 重启可看装载失败原因）"
        )]},
    }})
}

#[cfg(test)]
#[path = "rpc_plugins_tests.rs"]
mod rpc_plugins_tests;
