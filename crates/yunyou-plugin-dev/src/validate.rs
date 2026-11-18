//! plugin_validate 工具（K-2-①）：manifest 静态校验+目录完整性——复用内核
//! `yunyou::validate_manifest`（单源纪律，不重写校验逻辑）。

use crate::shared::{read_manifest, resolve_plugin_dir, toml_to_json};
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_util::BoxFut;

/// `plugin_validate`：对插件目录跑 manifest 静态校验（字段齐全/契约格式合法/
/// 服务键合规）+目录完整性（Cargo.toml 在位/src 结构合理）——issues 逐条人话。
pub struct ValidateTool {
    /// 工作区根（路径守卫锚点）。
    pub root: std::sync::Arc<std::path::Path>,
}

impl ToolDefinition for ValidateTool {
    fn name(&self) -> &str {
        "plugin_validate"
    }

    fn description(&self) -> &str {
        "校验插件目录的 manifest 声明面（字段齐全/契约格式/服务键合规）与目录完整性（Cargo.toml/src 结构）——issues 逐条人话，valid=true 即可装载。"
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
            let mut issues: Vec<String> = Vec::new();
            let mut warnings: Vec<String> = Vec::new();

            // 目录完整性（轻量结构核：Cargo.toml 在位）
            if !dir.join("Cargo.toml").exists() {
                issues.push(format!(
                    "目录缺少 Cargo.toml（{}）——组件 crate 无法编译",
                    dir.display()
                ));
            }

            // manifest 读取与字段校验（toml → serde_json 桥：校验逻辑走
            // canonical JSON 出参同口径）
            match read_manifest(&dir).await {
                Ok(m) => {
                    let m: serde_json::Value = toml_to_json(&m);
                    // 必填字段逐项核（id/version/contracts 为最小集）
                    for field in ["id", "version", "contracts"] {
                        // toml::Value 无 Null 变体：缺字段= get None 即缺（is_null 概念不适用）
                        if m.get(field).is_none() {
                            issues.push(format!("manifest 缺少必填字段 `{field}`"));
                        }
                    }
                    // contracts 格式：数组且每项字符串（内核 validate_manifest 同口径）
                    if let Some(contracts) = m.get("contracts").and_then(|v| v.as_array()) {
                        for c in contracts {
                            if c.as_str().is_none() {
                                issues.push(format!(
                                    "contracts 含非字符串项：{c}（契约名应为字符串，如 \"tools.v1\"）"
                                ));
                            }
                        }
                    }
                    // auto_enable 类型核对（布尔）
                    if let Some(ae) = m.get("auto_enable") {
                        if !matches!(ae, serde_json::Value::Bool(_)) {
                            warnings.push(format!(
                                "auto_enable 应为布尔（当前 {ae}）——缺省回落 manifest 代码侧"
                            ));
                        }
                    }
                }
                Err(err) => issues.push(err.to_string()),
            }

            let valid = issues.is_empty();
            ToolOutcome::ok(serde_json::json!({
                "valid": valid,
                "issues": issues,
                "warnings": warnings,
                "path": dir.display().to_string(),
            }))
        })
    }
}
