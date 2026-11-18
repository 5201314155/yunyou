//! credentials 域 unary RPC（W5 批，契约 web.v1 域-配置凭据与插件.md §2）：
//! `credentials/list`（槽位目录：path/set/reference/source 三值，零明文）+
//! `credentials/set`（值只进托管存储+配置定点改写为引用——迁移语义，幂等）+
//! `credentials/delete`（unset；缺失 no-op）。
//!
//! # source 三值（裁决②，lead 2026-11-19）
//! `env`（环境变量覆盖，只测存在性**永不读出值**）> `store`（托管存储）>
//! `file`（配置文件明文，迁移兼容残存）；否则未配置=source 缺席。
//!
//! # 红线
//! 任何响应零明文；set 响应 {set:true}；describe 消费服务事实面（防泄漏三律）。

use crate::contract::{self, code, err_envelope};
use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;
use yunyou::ServiceId;
use yunyou_credentials::reference::LLM_API_KEY_REF;
use yunyou_credentials::CredentialService;

/// 服务取用（缺失=凭据插件未启用，人话拒绝）。
fn service(state: &Arc<WebState>) -> Result<Arc<dyn CredentialService>, Value> {
    state
        .turns
        .root()
        .get::<dyn CredentialService>(ServiceId::of::<dyn CredentialService>())
        .map_err(|_| {
            err_envelope(
                code::NOT_IMPLEMENTED,
                "凭据服务未就绪（凭据插件未启用），凭据面不可用",
            )
        })
}

/// 配置文件 llm.api_key 当前字面值（None=未写/引用/缺席——file 残存探测）。
fn config_plain_api_key(state: &Arc<WebState>) -> Option<String> {
    let text = std::fs::read_to_string(state.config_store.path()).ok()?;
    let doc: toml::Table = toml::from_str(&text).ok()?;
    let key = doc.get("llm")?.get("api_key")?.as_str()?.to_string();
    // `$` 引用=已迁移形态，非明文残存
    (!yunyou_credentials::reference::is_reference(&key)).then_some(key)
}

/// 托管存储文件的全部引用名（[references] 表键；文件缺席=空）。
fn store_reference_names(state: &Arc<WebState>) -> Vec<String> {
    let path = state.config_store.path().with_file_name("credentials.toml");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
        .and_then(|doc| {
            doc.get("references")?.as_table().map(|table| {
                table.keys().cloned().collect()
            })
        })
        .unwrap_or_default()
}

/// credentials/list（L1）：schema secret 槽位（llm.api_key）+托管存储槽位投影。
pub(crate) async fn credentials_list(state: &Arc<WebState>) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let mut items: Vec<Value> = Vec::new();
    // 一期唯一 schema 槽位：llm.api_key（role:secret 扫描面随 schema 增长扩展）
    let reference = LLM_API_KEY_REF;
    let configured = matches!(service.describe(reference).await, Ok(info) if info.configured);
    let source = match service.describe(reference).await {
        Ok(info) => info.source,
        Err(_) => None,
    };
    // file 残存=配置明文（迁移兼容）——describe 不知配置文件，本地探测；
    // 明文残存也构成「已配置」事实（迁移前 key 在 file 生效，裁决②三值投影）
    let file_plain = config_plain_api_key(state).is_some();
    let configured = configured || file_plain;
    let source = if file_plain { Some("file".to_string()) } else { source };
    let mut row = json!({
        "path": ["llm", "api_key"],
        "set": configured,
        "reference": reference,
    });
    if let Some(source) = source {
        row["source"] = Value::String(source);
    }
    items.push(row);
    // 托管存储其余槽位（schema 外自定义引用；同样只列事实面）
    for name in store_reference_names(state) {
        if name == "YUNYOU_LLM_API_KEY" {
            continue; // 上面已列（llm.api_key 同槽）
        }
        let reference = format!("${name}");
        let configured = matches!(service.describe(&reference).await, Ok(info) if info.configured);
        let source = match service.describe(&reference).await {
            Ok(info) => info.source,
            Err(_) => None,
        };
        let mut row = json!({
            "path": [name],
            "set": configured,
            "reference": reference,
        });
        if let Some(source) = source {
            row["source"] = Value::String(source);
        }
        items.push(row);
    }
    json!({"ok": true, "value": {
        "storePath": "credentials.toml（yunyou-home 下，600 权限）",
        "items": items,
    }})
}

/// credentials/set（L4，payload {path!,value!}）：llm.api_key→值入托管存储+
/// 配置定点改写为引用（迁移语义，幂等——重复 set 同值不炸）；其余 path 拒收
/// （一期唯一 schema 槽位，域细则 §2.1）。
pub(crate) async fn credentials_set(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let path: Vec<String> = request
        .get("path")
        .and_then(Value::as_array)
        .map(|array| array.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let Some(value) = request.get("value").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "set 缺 value（凭据值；空值拒绝——未配置=处处一致）");
    };
    if value.trim().is_empty() {
        return err_envelope(code::BAD_REQUEST, "set 值为空白（空值=未配置，请走 delete）");
    }
    if path == vec!["llm", "api_key"] {
        // 值入托管存储（幂等覆盖）；配置定点改写为引用（迁移语义）
        if let Err(err) = service.set(LLM_API_KEY_REF, value).await {
            return err_envelope(code::BAD_REQUEST, &err.to_string());
        }
        if let Err(keys) = state.config_store.write_section(
            "llm",
            &json!({"api_key": LLM_API_KEY_REF}),
        ) {
            // 引用改写失败（如校验不过）：托管值已在，配置残明文待下次迁移——诚实留痕
            eprintln!("[credentials] api_key 引用改写未落盘：{keys:?}");
        }
        return json!({"ok": true, "value": {"set": true}});
    }
    err_envelope(
        code::BAD_REQUEST,
        &format!("path {path:?} 不是已知凭据槽位（一期唯一：[\"llm\",\"api_key\"]）"),
    )
}

/// credentials/delete（L3，前端二次确认）：unset（缺失=no-op，对齐官方语义）。
pub(crate) async fn credentials_delete(state: &Arc<WebState>, payload: &Value) -> Value {
    let service = match service(state) {
        Ok(service) => service,
        Err(envelope) => return envelope,
    };
    let request = contract::request_of(payload);
    let path: Vec<String> = request
        .get("path")
        .and_then(Value::as_array)
        .map(|array| array.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if path == vec!["llm", "api_key"] {
        match service.unset(LLM_API_KEY_REF).await {
            Ok(()) => return json!({"ok": true, "value": {"deleted": true}}),
            Err(err) => return err_envelope(code::BAD_REQUEST, &err.to_string()),
        }
    }
    err_envelope(
        code::BAD_REQUEST,
        &format!("path {path:?} 不是已知凭据槽位（一期唯一：[\"llm\",\"api_key\"]）"),
    )
}

#[cfg(test)]
#[path = "rpc_credentials_tests.rs"]
mod rpc_credentials_tests;
