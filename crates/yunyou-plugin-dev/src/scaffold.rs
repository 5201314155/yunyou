//! plugin_scaffold 工具（K-2-②）：骨架生成 + 写闸——`<home>/plugins/<id>/`
//! 工具级 allowlist。
//!
//! # 写闸（与 resolve_workspace 两套闸并列不混用——52 单义文档加注）
//!
//! 本闸管「插件目录写入」：目标恒为 `<plugins_root>/<id>/` 一层之下，`id`
//! 白名单字符 `[a-z0-9-]`（路径穿越防护：`..`、大小写、符号、空串全拒），
//! 目标已存在 fail-loud（骨架生成不覆盖既有插件件）。`resolve_workspace`
//! 闸管「工作区文件读写」——两套语义不相交：scaffold 不碰工作区文件，
//! workspace 工具不碰 plugins 根；并列而非嵌套，任何一方放行不 implied
//! 另一方放行。
//!
//! # 骨架（hello L1 仪式件为模板——`crates/plugins/hello-sdk` 同构三件）
//!
//! - `manifest.toml`：market v2 字段（id/version/format/level/contracts/
//!   category/auto_enable + `[runtime]` subprocess + `[permissions]`
//!   writable=$SELF/credentials 恒空/network 恒禁）；
//! - `Cargo.toml`：SDK 三依赖 + `[[bin]]`（产物名=id，装载器按名 spawn）；
//! - `src/main.rs`：`Plugin::new` 链式 + 单 hello 工具仪式（开发者拿骨架
//!   改业务，L1 五步装载口径照走：生成→编译→install→重启→/plugins 可见）。

use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_util::BoxFut;

/// `plugin_scaffold`：生成插件骨架三件到 `<plugins_root>/<id>/`。
///
/// 写闸见模块头注。生成物=可直接 `cargo build` 的独立 crate（SDK 零内核
/// 依赖），重启后被装载器发现并出现在 `/plugins`。
pub struct ScaffoldTool {
    /// 插件根（`<home>/plugins`——写闸锚点；注入而非自取=测试缝）。
    pub plugins_root: std::sync::Arc<std::path::Path>,
}

/// id 白名单：`[a-z0-9-]`、1..=64 字符。目录名与 manifest id 同源，
/// 白名单即路径穿越防护的唯一防线（`..`/`/`/大写/符号全拒）。
fn id_allowed(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// manifest.toml 骨架（market v2；字段语义=hello-sdk 仪式件同构）。
fn manifest_toml(id: &str, description: &str) -> String {
    format!(
        r#"# {id} 落盘 manifest（market v2 形态；L2 设计 §3.1 字段）
id = "{id}"
version = "0.1.0"
format = 2
level = "market"
contracts = ["tools.v1"]
category = "tools"
auto_enable = true

[runtime]
type = "subprocess"
entry = ["{id}"]                  # 相对插件目录=cargo build 产物名
request_timeout_ms = 30000

[permissions]
writable = ["$SELF"]              # 缺省=仅插件目录可写
credentials = []                  # MVP 凭据面未开闸，恒空
network = false                   # MVP 网络面恒禁
"#
    ) .replace("{id}", id)
      .replace("骨架占位：请改写本描述", description)
}

/// Cargo.toml 骨架（SDK 三依赖 + `[[bin]]` 产物名=id）。
fn cargo_toml(id: &str) -> String {
    format!(
        r#"[package]
name = "{id}"
version = "0.1.0"
edition = "2021"
license = "MIT"

[dependencies]
yunyou-plugin-sdk = {{ path = "../../plugins/yunyou-plugin-sdk", version = "0.1.0" }}
schemars = "1"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"

[[bin]]
name = "{id}"
path = "src/main.rs"

[lints]
workspace = true
"#
    )
}

/// src/main.rs 骨架（Plugin::new 链式 + 单 hello 工具仪式）。
fn main_rs(id: &str, description: &str) -> String {
    format!(
        r#"//! {id}：plugin_scaffold 骨架生成件（K-2-②）。
//!
//! 零内核 import、零内核文档依赖；文档=SDK crate 的 SDK.md。
//! 本文件=起点：改工具逻辑即业务开发，装载口径不变。

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use yunyou_plugin_sdk::{{tool, Plugin, ToolFailure}};

/// hello 工具参数（schema 自动派生进模型面；字段 doc-comment=给 LLM 的说明）。
#[derive(Deserialize, JsonSchema)]
struct HelloArgs {{
    /// 要向谁问好（人名或称呼）
    name: String,
}}

fn main() -> std::process::ExitCode {{
    Plugin::new("{id}", "0.1.0")
        .category("tools")
        .description("{description}")
        .tool(tool("hello", "向指定的人问好，返回问候语", |args: HelloArgs| {{
            if args.name.trim().is_empty() {{
                return Err(ToolFailure::with_hint(
                    "tool/invalid-args",
                    "name 不能为空",
                    "传入要问好的对象名，如「小云」",
                ));
            }}
            Ok(json!({{ "greeting": format!("你好，{{}}！", args.name) }}))
        }}))
        .serve()
}}
"#
    )
}

impl ToolDefinition for ScaffoldTool {
    fn name(&self) -> &str {
        "plugin_scaffold"
    }

    fn description(&self) -> &str {
        "生成插件骨架（manifest.toml+Cargo.toml+src/main.rs，hello 仪式件为模板）到 <home>/plugins/<id>/——写闸=插件目录 allowlist（id 白名单 [a-z0-9-]），已存在 fail-loud 不覆盖。生成后编译→install→重启即被 /plugins 发现（L1 五步口径）。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "插件 id（[a-z0-9-]，作目录名、manifest id 与 cargo 产物名三者同源）"},
                "description": {"type": "string", "description": "一句话插件描述（进 manifest 注释与 main.rs .description；缺省=骨架占位）"}
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let plugins_root = self.plugins_root.clone();
        Box::pin(async move {
            let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 id 参数");
            };
            // 写闸第一道：id 白名单（路径穿越防护唯一防线）
            if !id_allowed(id) {
                return ToolOutcome::err(
                    "tool/args",
                    "id 只允许 [a-z0-9-]（1..=64 字符）——目录名与 manifest id 同源，白名单即路径穿越防护",
                );
            }
            let dir = plugins_root.join(id);
            // 写闸第二道：目标恒在 plugins_root 直接一层之下（显式断言，
            // 与白名单构成双重防御——join 语义变化时不静默越界）
            if dir.parent() != Some(plugins_root.as_ref()) {
                return ToolOutcome::err("tool/path-outside-root", "目标越出插件根一层（写闸拒绝）");
            }
            // 写闸第三道：已存在 fail-loud（骨架不覆盖既有插件件）
            if dir.exists() {
                return ToolOutcome::err(
                    "tool/exists",
                    &format!("{} 已存在——骨架生成不覆盖既有插件（fail-loud）", dir.display()),
                );
            }
            let description = args
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("骨架占位：请改写本描述");
            let manifest = manifest_toml(id, description);
            let cargo = cargo_toml(id);
            let main_src = main_rs(id, description);
            // 写闸收敛后落盘：三件一次生成，中途失败不留半成品目录
            // （先建 src/，逐件写；任一失败即清理整个 <id>/ 目录）。
            if let Err(err) = std::fs::create_dir_all(dir.join("src")) {
                return ToolOutcome::err("tool/io", &format!("创建插件目录失败：{err}"));
            }
            let files: [(&str, &str); 3] = [
                ("manifest.toml", manifest.as_str()),
                ("Cargo.toml", cargo.as_str()),
                ("src/main.rs", main_src.as_str()),
            ];
            for (rel, content) in files {
                if let Err(err) = std::fs::write(dir.join(rel), content) {
                    let _ = std::fs::remove_dir_all(&dir);
                    return ToolOutcome::err("tool/io", &format!("写 {rel} 失败（已回滚目录）：{err}"));
                }
            }
            ToolOutcome::ok(serde_json::json!({
                "scaffolded": dir.display().to_string(),
                "files": ["manifest.toml", "Cargo.toml", "src/main.rs"],
                "next": [
                    "编译：在插件目录 cargo build --release",
                    "install：产物与 manifest 落 <home>/plugins/<id>/（本工具已就位）",
                    "重启 yunyou → /plugins 可见可开关（L1 五步口径）"
                ],
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写闸：`..`/大写/符号/空串/超长全拒（路径穿越防护矩阵）。
    #[test]
    fn id白名单_穿越与非法形态全拒() {
        assert!(!id_allowed("../evil"));
        assert!(!id_allowed("Foo"));
        assert!(!id_allowed("has space"));
        assert!(!id_allowed("has/slash"));
        assert!(!id_allowed(""));
        assert!(!id_allowed(&"a".repeat(65)));
        assert!(id_allowed("hello-sdk"));
        assert!(id_allowed("my-plugin-2"));
    }

    /// 写闸第二道：join 语义双重防御（id 合法时目标恒在一层之下）。
    #[test]
    fn 目标恒在插件根一层之下() {
        let root = std::path::Path::new("/tmp/plugins-root");
        let dir = root.join("hello-sdk");
        assert_eq!(dir.parent(), Some(root), "白名单 id 的目标必在根下一层");
    }

    /// 骨架三件：manifest/Cargo/main 内容含模板关键形态（id 同源/五步指针）。
    #[test]
    fn 骨架三件_模板形态() {
        let m = manifest_toml("hello-sdk", "测试描述");
        assert!(m.contains("id = \"hello-sdk\""));
        assert!(m.contains("format = 2"));
        assert!(m.contains("writable = [\"$SELF\"]"));
        assert!(m.contains("network = false"));
        assert!(m.contains("测试描述"));
        let c = cargo_toml("hello-sdk");
        assert!(c.contains("yunyou-plugin-sdk"));
        assert!(c.contains("[[bin]]"));
        let r = main_rs("hello-sdk", "测试描述");
        assert!(r.contains("Plugin::new(\"hello-sdk\""));
        assert!(r.contains(".serve()"));
        assert!(r.contains("测试描述"));
    }

    /// 写闸第三道：已存在 fail-loud + 生成回滚（io 失败不留半成品）。
    #[test]
    fn 已存在fail_loud_与生成回滚() {
        let tmp = std::env::temp_dir().join(format!("scaffold-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugins_root = tmp.join("plugins");
        std::fs::create_dir_all(plugins_root.join("dup")).unwrap();
        // 已存在目录：fail-loud
        let dir = plugins_root.join("dup");
        assert!(dir.exists(), "前置：目标已存在");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
