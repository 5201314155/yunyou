//! plugin_contract_read 工具（K-2-①）：manifest 契约面结构化读取+漂移比对。

use crate::shared::{read_manifest, resolve_plugin_dir};
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_util::BoxFut;

/// `plugin_contract_read`：读指定插件目录的 manifest.toml，返回结构化契约面
/// （id/version/level/contracts/provides/inject/ui/auto_enable）——开发者
/// 不用打开 TOML 文件肉眼比对，声明面一眼可见。
pub struct ContractReadTool {
    /// 工作区根（路径守卫锚点）。
    pub root: std::sync::Arc<std::path::Path>,
}

impl ToolDefinition for ContractReadTool {
    fn name(&self) -> &str {
        "plugin_contract_read"
    }

    fn description(&self) -> &str {
        "读取插件目录的 manifest.toml 并返回结构化契约面（id/版本/级别/契约/提供/注入/UI/自动启用）——开发插件时核对声明面，不用肉眼比对 TOML。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "插件目录相对路径（相对工作区根，如 plugins/hello）"}
            },
            "required": ["path"]
        })
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let root = self.root.clone();
        Box::pin(async move {
            let Some(rel) = args.get("path").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 path 参数");
            };
            let dir = match resolve_plugin_dir(&root, rel) {
                Ok(dir) => dir,
                Err(err) => return ToolOutcome::err("tool/path-outside-root", &err.to_string()),
            };
            let manifest = read_manifest(&dir).await;
            match manifest {
                Ok(m) => {
                    let contract = serde_json::json!({
                        "id": m.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        "version": m.get("version").and_then(|v| v.as_str()).unwrap_or(""),
                        "level": m.get("level").and_then(|v| v.as_str()).unwrap_or(""),
                        "contracts": m.get("contracts").cloned().unwrap_or(serde_json::Value::Null),
                        "provides": m.get("provides").cloned().unwrap_or(serde_json::Value::Null),
                        "inject": m.get("inject").cloned().unwrap_or(serde_json::Value::Null),
                        "ui": m.get("ui").cloned().unwrap_or(serde_json::Value::Null),
                        "auto_enable": m.get("auto_enable").cloned().unwrap_or(serde_json::Value::Null),
                        "path": dir.display().to_string(),
                    });
                    ToolOutcome::ok(contract)
                }
                Err(err) => ToolOutcome::err("plugin-dev/manifest-read", &err.to_string()),
            }
        })
    }
}
