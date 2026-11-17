//! 内置插件注册表（bundled 清单，编译期常量数组，方案 §2.4 第 1 步）。
//!
//! 主板论铁律的落点：装配层从「知道每个插件」降级为「知道清单接口」——新增内置
//! 插件只改本清单一处＋建组件 crate（技能 §四 bundled 实操五步第③步），**严禁改
//! 装配代码**。条目的 `register` 是泛型收窄点：内核 [`yunyou::Context::plugin`]
//! 收具体插件类型，表驱动循环经 fn 指针统一调用（编译期常量，零运行时注册开销）。
//!
//! 一期八 bundled 齐（方案 §6.1 验收口径 + 任务29 C2 + S10）：llm-openai /
//! session / system-prompt / commands（任务29 命令注册缝）/ tools / retry（S10
//! 任务18 第一张实战卡）/ agent-loop / credentials（S5/S8 收口合入）。

use yunyou::manifest::PluginManifest;
use yunyou::{ConfigValue, Context, FiberHandle, Plugin, validate_manifest};
use yunyou_agent_loop::AgentLoopPlugin;
use yunyou_commands::CommandsPlugin;
use yunyou_credentials::CredentialsPlugin;
use yunyou_llm_openai::OpenAiPlugin;
use yunyou_retry::RetryPlugin;
use yunyou_sandbox::SandboxPlugin;
use yunyou_session::SessionPlugin;
use yunyou_system_prompt::SystemPromptPlugin;
use yunyou_team::TeamPlugin;
use yunyou_tools::ToolsPlugin;


/// 内置插件注册表条目（方案 §2.4 第 1-3 步的表结构）。
pub(crate) struct BundledEntry {
    /// 插件 id（= manifest.id，bundled 用 crate 名）；yunyou.toml `[plugins]` 段键。
    pub id: &'static str,
    /// yunyou.toml 顶层已知段名（存量五段；新机制插件无顶层段填 `""`，
    /// 段值只走 `[plugins.<id>.config]`——配置面不再锁死插件清单）。
    pub segment: &'static str,
    /// 注册函数（方案 §2.4 第 3 步）：表驱动循环唯一调用点，泛型具体类型在此收窄。
    pub register: fn(&Context, ConfigValue) -> FiberHandle,
    /// 装载前静态校验（S1 入口接入，方案 §2.2）：实例化→manifest→[`validate_manifest`]，
    /// 返回违例清单（空 = 通过；非空装配层拒装并逐条报出）。
    pub validate: fn() -> Vec<String>,
    /// 静态声明面：`auto_enable`（enable 缺省回落，方案 §2.4 第 2 步）与
    /// `contracts`（宿主插槽版本比对源，方案 §2.5）的读取入口。
    pub manifest: fn() -> PluginManifest,
}

/// 装载前静态校验的通用实现：`manifest()` → [`validate_manifest`]（S1 入口接入）。
/// 注册表条目经 `|| check_manifest(XxxPlugin)` 引用（单元素单元结构体，
/// const 上下文可构造，闭包零捕获仍可强转 fn 指针）。
pub(crate) fn check_manifest<P: Plugin>(plugin: P) -> Vec<String> {
    let manifest = plugin.manifest();
    validate_manifest(&plugin, &manifest)
}

/// 内置插件注册表（编译期常量数组，方案 §2.4 第 1 步清单）。
///
/// 顺序语义（task-24 修订）：一般而言顺序仅影响注册与状态表输出次序（真组件
/// 声明 `inject` 后 epoch 机制等依赖就绪再装载）——**例外=sandbox→tools 的
/// optional-get 顺序依赖**（tools setup 期可选 get 沙箱 enforcer，设计 §2
/// 裁决 1：禁用沙箱不瘫痪工具面，故不进 inject 走清单顺序）与
/// system-prompt→sandbox（告知段注册需段缝先 Active）。顺序守护测试
/// （tests/bundled_order.rs）机械锁防未来重排静默破坏。
pub(crate) static BUNDLED_PLUGINS: &[BundledEntry] = &[
    BundledEntry {
        id: "yunyou-llm-openai",
        segment: "llm",
        register: |ctx, cfg| ctx.plugin(OpenAiPlugin, cfg),
        validate: || check_manifest(OpenAiPlugin),
        manifest: || OpenAiPlugin.manifest(),
    },
    BundledEntry {
        id: "yunyou-session",
        segment: "session",
        register: |ctx, cfg| ctx.plugin(SessionPlugin, cfg),
        validate: || check_manifest(SessionPlugin),
        manifest: || SessionPlugin.manifest(),
    },
    BundledEntry {
        id: "yunyou-system-prompt",
        segment: "system_prompt",
        register: |ctx, cfg| ctx.plugin(SystemPromptPlugin, cfg),
        validate: || check_manifest(SystemPromptPlugin),
        manifest: || SystemPromptPlugin.manifest(),
    },
    // 任务29 C4：commands 列在 tools/agent-loop 之前——tools 声明 inject
    // dyn CommandsService（/tools-info 注册依赖），内核 Pending 语义是「不阻塞
    // 启动、重启恢复」非运行中自动唤醒，清单位次提前即免重启（顺序仍是优化
    // 而非正确性依赖：缺位次时 epoch 机制保持 Pending+诊断，不炸）。
    BundledEntry {
        id: "yunyou-commands",
        segment: "",
        register: |ctx, cfg| ctx.plugin(CommandsPlugin, cfg),
        validate: || check_manifest(CommandsPlugin),
        manifest: || CommandsPlugin.manifest(),
    },
    // task-24（P1-11a 实现批收口）：沙箱插件——system-prompt 之后（告知段注册
    // 需段缝 Active）、tools 之前（tools setup 期 optional get enforcer，
    // bundled 清单顺序语义=本清单第一条正确性依赖，顺序守护测试在位）。
    BundledEntry {
        id: "yunyou-sandbox",
        segment: "sandbox",
        register: |ctx, cfg| ctx.plugin(SandboxPlugin, cfg),
        validate: || check_manifest(SandboxPlugin),
        manifest: || SandboxPlugin.manifest(),
    },
    BundledEntry {
        id: "yunyou-tools",
        segment: "tools",
        register: |ctx, cfg| ctx.plugin(ToolsPlugin, cfg),
        validate: || check_manifest(ToolsPlugin),
        manifest: || ToolsPlugin.manifest(),
    },
    // S10（任务18，云伍）：重试策略插件——注册序在 agent-loop 之前（消费时序
    // 论证见 agent-loop plugin.rs rustdoc：agent-loop setup 期容错 get，缺服务
    // 即直连 fail-fast，不进 inject）。
    BundledEntry {
        id: "yunyou-retry",
        segment: "retry",
        register: |ctx, cfg| ctx.plugin(RetryPlugin, cfg),
        validate: || check_manifest(RetryPlugin),
        manifest: || RetryPlugin.manifest(),
    },
    BundledEntry {
        id: "yunyou-agent-loop",
        segment: "agent_loop",
        register: |ctx, cfg| ctx.plugin(AgentLoopPlugin, cfg),
        validate: || check_manifest(AgentLoopPlugin),
        manifest: || AgentLoopPlugin.manifest(),
    },
    // 第 6 bundled（S5/S8 收口合入，2026-11-14 第 6 席留位兑现）：凭据托管存储
    // +settings 投影双服务。宿主直接持有 store 实例做迁移与 llm 段引用解析
    //（crate::credentials），不经服务总线、与本插件装载位次无关（匠二精确规格）。
    BundledEntry {
        id: "yunyou-credentials",
        segment: "credentials",
        register: |ctx, cfg| ctx.plugin(CredentialsPlugin, cfg),
        validate: || check_manifest(CredentialsPlugin),
        manifest: || CredentialsPlugin.manifest(),
    },
    // K-7（任务26 T2，云络）：团队插件——lead 身份源+TeamService+工具族注册；
    // segment 共享 "session"（⑤改判）：与 session 卡同读 [session].path=成员
    // 会话目录 config 单源；[plugins.yunyou-team.config] 叠加团队私有键（裁③）
    BundledEntry {
        id: "yunyou-team",
        segment: "session",
        register: |ctx, cfg| ctx.plugin(TeamPlugin, cfg),
        validate: || check_manifest(TeamPlugin),
        manifest: || TeamPlugin.manifest(),
    },
];
#[cfg(test)]
mod tests {
    use super::*;

    /// S1 校验在装配侧接线回归：全部内置插件的 manifest 装载前校验应零违例
    ///（组件 manifest 与 trait inject 双向一致、contracts 格式合法、服务键合规）。
    #[test]
    fn 内置插件装载前校验零违例() {
        for entry in BUNDLED_PLUGINS {
            let issues = (entry.validate)();
            assert!(issues.is_empty(), "插件 {} 校验违例：{issues:?}", entry.id);
        }
    }

    /// 注册表静态面自洽：id 与 manifest.id 一致（[plugins] 段键的锚定契约）。
    #[test]
    fn 注册表_id与manifest一致() {
        for entry in BUNDLED_PLUGINS {
            let manifest = (entry.manifest)();
            assert_eq!(entry.id, manifest.id, "[plugins] 段键必须等于 manifest.id");
            assert!(manifest.auto_enable, "内置插件默认应启用（enable 缺省回落语义）");
        }
    }

    /// 顺序守护（task-24 裁 3③）：optional-get 顺序依赖的机械锁——
    /// system-prompt→sandbox（告知段注册需段缝先 Active）与 sandbox→tools
    /// （tools setup 期 optional get enforcer）。未来重排若破坏此序，本测试
    /// 红锁阻止静默破坏；解除需连带修订 bundled.rs 头注与本测试并留痕。
    #[test]
    fn 沙箱顺序守护() {
        let idx = |id: &str| {
            BUNDLED_PLUGINS
                .iter()
                .position(|entry| entry.id == id)
                .unwrap_or_else(|| panic!("bundled 清单缺 {id}"))
        };
        assert!(
            idx("yunyou-system-prompt") < idx("yunyou-sandbox"),
            "system-prompt 必须先于 sandbox（告知段段缝）"
        );
        assert!(
            idx("yunyou-sandbox") < idx("yunyou-tools"),
            "sandbox 必须先于 tools（enforcer optional-get）"
        );
    }
}
