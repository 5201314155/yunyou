//! 四工具族共享骨架：manifest 路径解析/读取/发现层形态构造。
//!
//! 职责切分：路径解析与文件 I/O 在此单点；contract_read/validate 各自只做
//! 语义层（比对/校验）——后续 77/78 同族工具复用本骨架。

use std::path::{Path, PathBuf};
use yunyou::CordisError;
use yunyou_util::BoxFut;

/// 工作区根锚定 + 相对路径守卫（防越界）：`rel` 必须落在 root 内——
/// `plugins/…` 形态；绝对路径与 `..` 逃逸拒绝（与 path_guard 同纪律）。
pub(crate) fn resolve_plugin_dir(root: &Path, rel: &str) -> Result<PathBuf, CordisError> {
    let candidate = Path::new(rel);
    let joined = if candidate.is_absolute() {
        return Err(CordisError::new(
            yunyou::ErrorKind::Internal,
            format!("plugin-dev/路径须为相对路径（相对工作区根），收到绝对路径：{rel}"),
        ));
    } else {
        root.join(candidate)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return Err(CordisError::new(
                        yunyou::ErrorKind::Internal,
                        format!("plugin-dev/路径越出根目录：{rel}"),
                    ));
                }
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if !normalized.starts_with(root) {
        return Err(CordisError::new(
            yunyou::ErrorKind::Internal,
            format!("plugin-dev/路径越出根目录：{rel}"),
        ));
    }
    Ok(normalized)
}

/// 读 manifest.toml 并解析为 TOML 表（fail-loud 人话）。
pub(crate) fn read_manifest(dir: &Path) -> BoxFut<'static, Result<toml::Value, CordisError>> {
    let path = dir.join("manifest.toml");
    Box::pin(async move {
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|err| {
                CordisError::new(
                    yunyou::ErrorKind::Internal,
                    format!("manifest.toml 读取失败（{}）：{err}", path.display()),
                )
            })?;
        toml::from_str(&text).map_err(|err| {
            CordisError::new(
                yunyou::ErrorKind::Internal,
                format!("manifest.toml 解析失败（{}）：{err}", path.display()),
            )
        })
    })
}

/// toml::Value → serde_json::Value 单源转换器（K-2 出参契约 canonical JSON）。
///
/// 取代「toml::to_string → serde_json::from_str」文本往返——该桥对嵌套表
/// 静默失败（unwrap_or_default 空串→空表，contract_read_hello 实测锚：磁盘
/// manifest 正确而出参全空）。toml 无 Null 变体：缺键一律 JSON null。
pub(crate) fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::json!(i),
        toml::Value::Float(f) => serde_json::json!(f),
        toml::Value::Boolean(b) => serde_json::json!(b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(toml_to_json).collect())
        }
        toml::Value::Table(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), toml_to_json(v));
            }
            serde_json::Value::Object(out)
        }
    }
}
