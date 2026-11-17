//! /plugins 观测面命令（装载器 L1，K3；设计《装载器最小方案》验收标尺）。
//!
//! 职责：把当前装配的插件状态行（id/version/enabled/state/**source**）渲染为
//! 可读表——发现来源列（bundled/discovered）是 L1 的观测增量：用户丢
//! `plugins/<目录>/manifest.toml` → 重启 → `/plugins` 即见来源与启用态
//!（任务16 自举标尺「可见可开关」），开关动作走 yunyou.toml `[plugins.<id>]`
//! enable（/new 重建装配同效，无需重启进程）。
//!
//! 独立成文件的行数纪律：shell_commands.rs 已贴 400 硬顶，新增命令按职责
//! 拆出（家规第四条）；仍走命令缝注册（REPL 只认注册表分发，设计 §三）。

use crate::shell_commands::SessionSlot;
use crate::status::render_status_table;
use std::sync::Arc;
use yunyou::ServiceId;
use yunyou_commands::{
    CommandDefinition, CommandHandler, CommandInvocation, CommandKind, CommandOutput,
    CommandsService,
};
use yunyou_util::BoxFut;

/// 注册 /plugins 命令（走命令缝；装配完成与 /new 重建后各调用一次——
/// 重复注册同名会 Err fail-loud，由调用方保证每装配一轮一次）。
pub fn register_plugins_command(root: &yunyou::Context, slot: Arc<SessionSlot>) -> yunyou::Result<()> {
    let commands = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())?;
    let def = plugins_command(slot);
    let name = def.name.clone();
    let disposer = commands.register(def)?;
    // 注册即 effect（与壳命令同纪律）：root 卸载时自动除名
    root.effect(format!("观测命令 /{name}"), move || Ok(Some(disposer)))?;
    Ok(())
}

/// /plugins 命令定义（数据源=当前装配的 plugin_status 状态行，随 /new 换槽）。
fn plugins_command(slot: Arc<SessionSlot>) -> CommandDefinition {
    CommandDefinition {
        name: "plugins".into(),
        aliases: vec!["插件".into()],
        description: "看插件状态表（含发现来源 bundled/discovered）".into(),
        kind: CommandKind::Action,
        accepts_args: false,
        handler: plugins_handler(slot),
    }
}

/// /plugins handler：渲染状态表 + 开关指引（指引写路径不写内部实现，模型/用户可执行）。
fn plugins_handler(slot: Arc<SessionSlot>) -> CommandHandler {
    Arc::new(move |_inv: CommandInvocation| {
        let slot = slot.clone();
        Box::pin(async move {
            let session = slot.load();
            let mut text = render_status_table(&session.plugin_status);
            text.push_str(
                "\n开关：yunyou.toml [plugins.<id>] enable = true/false，改后重启生效（/new 后立即生效）",
            );
            Ok(CommandOutput::success(text))
        }) as BoxFut<'static, _>
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::PluginStatus;
    use crate::shell_commands::ActiveSession;
    use yunyou::Context;

    /// /plugins 经缝注册可解析，别名「插件」同源；输出含来源列与开关指引。
    #[tokio::test(flavor = "multi_thread")]
    async fn plugins命令经缝注册与渲染() {
        let root = Context::new();
        let handle = root.plugin(yunyou_commands::CommandsPlugin, serde_json::json!({}));
        handle.await_ready().await.expect("commands 插件应装载");
        let slot = SessionSlot::new(ActiveSession {
            root: root.clone(),
            handler: Arc::new(|_line: &str, _cancel: tokio_util::sync::CancellationToken| {
                Box::pin(async { Ok::<(), crate::run::AppError>(()) })
            }),
            plugin_status: vec![
                PluginStatus {
                    id: "yunyou-tools".into(),
                    version: "0.1.0".into(),
                    enabled: false,
                    state: "disabled",
                    source: "discovered",
                },
                PluginStatus {
                    id: "yunyou-llm-openai".into(),
                    version: "0.1.0".into(),
                    enabled: true,
                    state: "active",
                    source: "bundled",
                },
            ],
        });
        register_plugins_command(&root, slot.clone()).expect("注册应成功");
        let commands = root
            .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
            .expect("commands 服务应就绪");
        for token in ["plugins", "插件"] {
            assert!(commands.find(token).is_some(), "/{token} 应可解析");
        }
        let handler = commands.handler_of("plugins").expect("应有 handler");
        let output = handler(CommandInvocation { raw_input: "/plugins".into() }).await.unwrap();
        assert!(output.text.contains("yunyou-tools") && output.text.contains("discovered"),
            "discovered 来源应可见：{}", output.text);
        assert!(output.text.contains("bundled"), "bundled 来源应可见：{}", output.text);
        assert!(output.text.contains("[plugins.<id>]"), "开关指引应在：{}", output.text);
        // 重复注册 fail-loud（防同装配轮重复）
        assert!(register_plugins_command(&root, slot).is_err(), "同名重复注册必须 Err");
        root.dispose().await.expect("应优雅关闭");
    }
}
