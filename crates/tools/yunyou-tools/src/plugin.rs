//! 工具插件：装载即注册 `dyn RegistryService` 并挂载六个内置工具（注册即 effect）。

use crate::builtin;
use crate::registry::{RegistryService, ToolsRegistry};
use yunyou::manifest_ui::UiEntry;
use yunyou::{ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId, projection_of};
use yunyou_commands::{CommandDefinition, CommandKind, CommandOutput, CommandsService};
use yunyou_util::BoxFut;
use std::path::Path;
use std::sync::Arc;

/// 工具插件配置。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct ToolsConfig {
    /// 文件类工具的根目录（相对 cwd 或绝对路径；默认当前目录）。
    #[serde(default = "default_root")]
    pub root_dir: String,
    /// 内置工具开关（工具名→bool；缺省空表=全开）。`false` = 不注册、不进
    /// schemas() 模型面（LLM 看不见=真关）；未知工具名键装载期 fail-loud。
    #[serde(default)]
    pub builtin: std::collections::BTreeMap<String, bool>,
    /// 前台自动转后台阈值（秒；默认 60，0=关自动转；显式 run_in_background
    /// 不受此限）。池 task-20。
    #[serde(default = "default_background_after_secs")]
    pub background_after_secs: u64,
}

fn default_background_after_secs() -> u64 {
    crate::builtin::bash::BACKGROUND_AFTER_DEFAULT_SECS
}

fn default_root() -> String {
    ".".into()
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self { root_dir: default_root(), builtin: Default::default(), background_after_secs: default_background_after_secs() }
    }
}

/// 工具插件：注册 `dyn RegistryService`（注册表）+ 六内置工具。
///
/// # 依赖
/// 无（内置工具不依赖 systemPrompt——方案 §2.5 依赖图的裁剪，差异已报备；
/// 工具描述注入由 agent-loop 组装）。
#[derive(Default)]
pub struct ToolsPlugin;

impl Plugin for ToolsPlugin {
    fn name(&self) -> &str {
        "yunyou-tools"
    }

    /// 声明面（方案 §2.2）：对齐 tools.v1 插槽规格、provide `dyn RegistryService`；
    /// 任务29 C4 起注入 `dyn CommandsService`（/tools-info 注册依赖）——内核 epoch
    /// 机制保证本插件装载时该服务已就绪（清单位次不构成正确性依赖）。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-tools".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["tools.v1".into()],
            provides: vec![projection_of::<dyn RegistryService>()],
            inject: vec![projection_of::<dyn CommandsService>()],
            ui: vec![UiEntry::Command { name: "tools-info".into() }],
            auto_enable: true,
        }
    }

    /// 依赖（任务29 C4）：/tools-info 经命令缝注册，装载等 commands 服务就绪。
    fn inject(&self) -> Vec<ServiceId> {
        vec![ServiceId::of::<dyn CommandsService>()]
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(serde_json::to_value(schemars::schema_for!(ToolsConfig)).expect("schema 可序列化"))
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: ToolsConfig = serde_json::from_value(config)
                .map_err(|err| yunyou::CordisError::new(yunyou::ErrorKind::Internal, format!("tools/配置解析失败: {err}")))?;
            let root: Arc<Path> = Arc::from(Path::new(&cfg.root_dir));
            let registry = ToolsRegistry::new();
            // 注册表本体先注册（agent-loop 才能取到）；显式擦除为 trait object
            //（泛型推断会把 P 定为具体类型 ToolsRegistry，导致 get::<dyn RegistryService> 类型不符）
            let service: Arc<dyn RegistryService> = registry.clone();
            ctx.provide(ServiceId::of::<dyn RegistryService>(), service)?;
            // 沙箱 enforcer 可选消费（设计 §2 裁决 1：不进 inject——禁用沙箱
            // 不瘫痪工具面；缺失=降级直跑+meta 标注。装配序 SandboxPlugin 先于
            // 本插件=bundled 清单顺序语义，见 cli/bundled.rs 顺序守护测试）。
            let enforcer: Option<std::sync::Arc<dyn crate::sandbox::SandboxEnforcer>> = ctx
                .get::<dyn crate::sandbox::SandboxEnforcer>(ServiceId::of::<
                    dyn crate::sandbox::SandboxEnforcer,
                >())
                .ok();
            // 内置工具：注册即 effect（本 fiber 卸载时逐个注销，逆序）。
            // 池 task-7 工具开关：builtin map 里值为 false 的工具跳过注册——
            // 不注册即不进 schemas() 模型面（LLM 看不见=真关，非仅禁执行），
            // 调用侧 get(name)=None 自然落「未知工具」错误路径。
            // 拼错防呆：map 键不在已知内置工具名清单 → fail-loud（配置即报错，
            // 不允许「关了个不存在的工具」静默通过）。
            // 后台任务注册表（池 task-20）：装配实例=会话生命周期，Drop 收割
            let task_registry = std::sync::Arc::new(crate::tasks::TaskRegistry::new());
            let tools = builtin::builtin_tools(root, enforcer, Some((task_registry, cfg.background_after_secs)));
            let known: std::collections::BTreeSet<&str> =
                tools.iter().map(|t| t.name()).collect();
            for key in cfg.builtin.keys() {
                if !known.contains(key.as_str()) {
                    return Err(yunyou::CordisError::new(
                        yunyou::ErrorKind::Internal,
                        format!(
                            "tools/[tools.builtin] 未知工具名 \"{key}\"（已知：{}）",
                            known.into_iter().collect::<Vec<_>>().join("、")
                        ),
                    ));
                }
            }
            for tool in tools {
                let name = tool.name().to_string();
                if !cfg.builtin.get(&name).copied().unwrap_or(true) {
                    continue; // 用户显式关闭：不注册、模型面不可见
                }
                // register 返回的撤销器直接登记为本 fiber 的 effect：
                // fiber 卸载 → 逆序执行 → 从注册表移除
                // 显式固有调用：trait 的 Arc<Self> 接收器会吞 Arc（循环内不可）
                let disposer = ToolsRegistry::register(&registry, tool)?;
                ctx.effect(format!("注册内置工具 {name}"), move || Ok(Some(disposer)))?;
            }
            // /tools-info（任务29 C4 插件命令示范，设计 §八）：动作类，列内置工具
            // name+描述——文案从注册表 schemas() 生成（新工具自动出现，禁硬编码）。
            // 声明面与实现面配对：manifest.ui 的 Command{name:"tools-info"} 即此注册。
            let commands = ctx
                .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
                .map_err(|err| {
                    yunyou::CordisError::new(
                        yunyou::ErrorKind::Internal,
                        format!("tools/commands 服务未就绪（inject 已声明，装配异常）: {err}"),
                    )
                })?;
            let schemas = registry.schemas();
            let disposer = commands.register(CommandDefinition {
                name: "tools-info".into(),
                aliases: vec!["工具".into()],
                description: "列出内置工具与用途".into(),
                kind: CommandKind::Action,
                accepts_args: false,
                handler: Arc::new(move |_inv| {
                    let schemas = schemas.clone();
                    Box::pin(async move {
                        let mut text = format!("内置工具（{}）：", schemas.len());
                        for schema in &schemas {
                            text.push_str(&format!("\n  {}  {}", schema.name, schema.description));
                        }
                        Ok(CommandOutput::success(text))
                    })
                }),
            })?;
            ctx.effect("注册内置命令 /tools-info", move || Ok(Some(disposer)))?;
            Ok(FiberOutput)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// manifest 声明面字段齐全（方案 §2.2，S1 验收；任务29 C4 起含 commands 依赖与 ui 声明）。
    #[test]
    fn manifest声明面字段齐全() {
        let m = ToolsPlugin.manifest();
        assert_eq!(m.id, "yunyou-tools");
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(m.contracts, vec!["tools.v1".to_string()]);
        assert_eq!(m.provides, vec!["dyn RegistryService".to_string()]);
        assert_eq!(m.inject, vec!["dyn CommandsService".to_string()]);
        assert_eq!(
            m.ui,
            vec![UiEntry::Command { name: "tools-info".into() }],
            "声明面与实现面配对（设计 §八）"
        );
    }

    /// manifest.inject 与 trait inject() 双向一致（装载前校验零违例）。
    #[test]
    fn manifest与trait_inject双向一致() {
        let plugin = ToolsPlugin;
        let issues = yunyou::validate_manifest(&plugin, &plugin.manifest());
        assert!(issues.is_empty(), "校验违例: {issues:?}");
    }

    /// /tools-info 经缝注册可执行（任务29 C4 示范件验收）：commands 先装、
    /// tools 后装（epoch 依赖形态的手动模拟），setup 完成后 find 到命令、
    /// handler 执行返回内置工具清单（文案从 schemas() 生成）。
    #[tokio::test(flavor = "multi_thread")]
    async fn tools_info经缝注册可执行() {
        let root = yunyou::Context::new();
        let handle = root.plugin(
            yunyou_commands::CommandsPlugin,
            serde_json::json!({}),
        );
        handle.await_ready().await.expect("commands 插件应装载");
        let handle = root.plugin(ToolsPlugin, serde_json::json!({"root_dir": "."}));
        handle.await_ready().await.expect("tools 插件应装载");
        let commands = root
            .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
            .expect("commands 服务");
        let descriptor = commands.find("tools-info").expect("/tools-info 应已注册");
        assert!(descriptor.kind == CommandKind::Action, "tools-info 是动作类");
        let handler = commands.handler_of("tools-info").expect("应有执行入口");
        let output = handler(yunyou_commands::CommandInvocation { raw_input: "/tools-info".into() })
            .await
            .expect("执行应成功");
        assert!(output.kind == yunyou_commands::CommandResultKind::Success);
        // 文案从注册表生成：含 bash（内置必有）与计数行
        assert!(output.text.contains("内置工具（"), "计数行在位: {}", output.text);
        assert!(output.text.contains("bash"), "内置 bash 应在列: {}", output.text);
        // 中文别名同源解析
        assert!(commands.find("工具").is_some(), "别名「工具」应解析到 tools-info");
    }
}
