//! hello-sdk：yunyou-plugin-sdk 自举示例（task-10）。
//!
//! 这个文件就是「用 SDK 写插件」的全部——零内核 import、零内核文档依赖；
//! 文档=SDK crate 的 SDK.md（随包分发）。

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use yunyou_plugin_sdk::{tool, Plugin, ToolFailure};

/// hello 工具参数（schema 自动派生进模型面；字段 doc-comment=给 LLM 的说明）。
#[derive(Deserialize, JsonSchema)]
struct HelloArgs {
    /// 要向谁问好（人名或称呼）
    name: String,
    /// 可选：正式称呼（缺省=false 用「你好」）
    #[serde(default)]
    formal: bool,
}

/// add 工具参数：演示多工具与数值参数。
#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    /// 加数 a
    a: f64,
    /// 加数 b
    b: f64,
}

fn main() -> std::process::ExitCode {
    Plugin::new("hello-sdk", "0.1.0")
        .category("tools")
        .description("SDK 自举示例：问好与加法")
        .tool(tool("hello", "向指定的人问好，返回问候语", |args: HelloArgs| {
            if args.name.trim().is_empty() {
                return Err(ToolFailure::with_hint(
                    "tool/invalid-args",
                    "name 不能为空",
                    "传入要问好的对象名，如「小云」",
                ));
            }
            let greeting = if args.formal {
                format!("您好，{}。云悠插件向您致意。", args.name)
            } else {
                format!("你好，{}！", args.name)
            };
            Ok(json!({ "greeting": greeting }))
        }))
        .tool(tool("add", "两个数相加", |args: AddArgs| {
            Ok(json!({ "sum": args.a + args.b }))
        }))
        .serve()
}
