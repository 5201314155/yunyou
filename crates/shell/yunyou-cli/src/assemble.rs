//! §2.4 表驱动装配（S2，方案「装配序列改造」主战场收官；S5 凭据接线收口合入）。
//!
//! 序列：config（纯库，段值透传）→ 内置插件注册表循环。
//! 装配层从「知道每个插件」降级为「知道清单接口」（方案 §2.4）：新增内置插件
//! 只改 [`crate::bundled::BUNDLED_PLUGINS`] 清单一处，装配代码零改动。
//!
//! 五步原文（方案 §2.4）：①注册表（编译期常量）→ ②读 [plugins] 段
//! {enable = manifest.auto_enable 默认, config: 段值} → ③循环注册（禁用跳过并
//! 记日志）→ ④逐个 await_ready（失败整树卸载 fail-loud；Pending 属稳定态立即
//! 返回，诊断「依赖未就绪」而非崩溃，§5.3 验收 1）→ ⑤启动输出插件状态表
//! （id/version/enabled/state，面板可开关的 CLI 观测面一期形态）。
//!
//! 装载前校验（S1 入口接入）：`validate_manifest` 静态校验 + 插槽版本比对
//! （方案 §2.5），违例拒装、整树逆序卸载、逐条中文报出。
//!
//! S5 凭据接线（lead 裁决②＝装配层解析，匠二精确规格；**迁移与解析同批原子
//! 合入**）：启动迁移（明文 key 入托管+配置文件定点改写，credentials 插件装载前）
//! 与 llm 段 `$NAME` 引用解析（merge 后 register 前，宿主直持 store 不经服务总线）
//! 见 [`crate::credentials`]。
//!
//! 段透传约定：yunyou.toml 各顶层段字段与组件 Config 一一对齐（yunyou-config::model）；
//! `session.path` 缺省补 `<yunyou-home>/sessions/default.jsonl`、
//! `credentials.store_path` 缺省补 `<yunyou-home>/credentials.toml`；
//! `[plugins.<id>.config]` 浅合并叠加顶层段（bundled::merge_segment）。

use crate::AppError;
use crate::bundled::BUNDLED_PLUGINS;
use crate::slots::{check_slot_versions, enabled_of_with_file, host_slot_support, merge_segment};
use crate::credentials::{open_store, resolve_llm_api_key, run_startup_migration};
use crate::status::{PluginStatus, pending_diagnosis};
use yunyou::manifest_ui::UiEntry;
use yunyou::ServiceId;
use yunyou::{Context, FiberHandle, FiberState};
use yunyou_commands::CommandsService;
use yunyou_config::{DshConfig, DiscoveredPlugin, discover_plugins};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// agent-loop 插件 id：装配产物的句柄锚点（就绪观测与诊断用；对话走 `dyn AgentLoop` 服务）。
const AGENT_LOOP_ID: &str = "yunyou-agent-loop";

/// llm 插件 id：llm 段引用解析的槽位锚点（S5，宿主直持 store 解析）。
const LLM_ID: &str = "yunyou-llm-openai";

/// 装配产物：root 强句柄（进程内唯一的整树关闭入口）+ agentLoop fiber 句柄
/// + 插件状态数据（观测面；打印策略归宿主——任务 30-⑤ 启动净化，--verbose 门控）。
///
/// `agent_loop` 为 Option（S2 起 agent-loop 可被 [plugins] 段禁用）：禁用时为 None
/// （REPL 侧对服务缺失已有优雅处理，报「服务未就绪」不崩）；非禁用时恒 Some。
pub struct Assembled {
    pub root: Context,
    /// agentLoop 的 fiber 句柄（禁用时 None；就绪观测与诊断用）。
    pub agent_loop: Option<FiberHandle>,
    /// 插件状态行（id/version/enabled/state，方案 §2.4 第 5 步数据面；
    /// 宿主经 `--verbose` 门控渲染输出，REPL `/plugins` 二期同源消费）。
    pub plugin_status: Vec<PluginStatus>,
}

/// 表驱动装配（方案 §2.4 五步）：任一启用插件装载失败/校验违例 → 整树逆序卸载
/// fail-loud；禁用插件安全跳过；依赖被禁用的消费者保持 Pending 并诊断（不崩）。
///
/// `config_path`：实际生效的配置文件路径（显式 --config 或隐式默认路径，run.rs
/// 传入）——启动迁移的改写对象；文件不存在（回退默认态）→ 迁移无操作。
pub async fn assemble(
    config: &DshConfig,
    home: &Path,
    config_path: &Path,
) -> Result<Assembled, AppError> {
    let root = Context::new();
    let raw = with_default_paths(config, home);
    // 文件式人设（池 task-6）：persona.md 命中即写进既有 shadow 人设槽，
    // 存在即 shadow toml 字符串人设（优先级与空文件回落见 persona_md 模块文档）
    let project_root = crate::persona_md::project_root().unwrap_or_else(|| std::path::PathBuf::from("."));
    let raw = crate::persona_md::apply_to_raw(raw, &project_root, home);
    // persona 桥接（审查 P1-3）：一处赋值，见 bridge_persona 文档
    let raw = bridge_persona(raw, config);

    // S5 凭据接线：宿主持有托管存储（迁移与 llm 段解析共用；不经服务总线，
    // 与 credentials 插件装载位次无关——匠二精确规格）。
    // 迁移在插件装载前（store 为 open 时快照，插件实例必须读到迁移后文件）。
    let store_path = raw["credentials"]["store_path"]
        .as_str()
        .expect("with_default_paths 已补 store_path")
        .to_string();
    let store = open_store(Path::new(&store_path))?;
    run_startup_migration(&store, config_path)?;

    // 装载器 L1（K2，设计 §3）：扫描 <home>/plugins/*/manifest.toml 发现层。
    // fail-loud：解析失败/静态校验违例/重复 id 聚合逐条报出拒装（此刻尚未装载
    // 任何插件，无需 teardown）；目录不存在＝未装插件，空表常态。
    let discovered = discover_plugins(&home.join("plugins")).map_err(|errors| {
        AppError::Assemble(format!(
            "装载器发现 plugins/ 目录声明违例（拒装，逐条修复后重启）：\n  {}",
            errors.join("\n  ")
        ))
    })?;
    // 按文件内 id 匹配 bundled 实现：命中→落盘声明投影（auto_enable 覆盖载体，
    // D1 代码真源不破——contracts/provides/inject/ui 仍以代码为准）；未命中→
    // 真外部包，L1 无装载能力，循环后明确诊断（K4 L2 接缝，设计 §4）。
    let mut discovered_by_id: HashMap<&str, &DiscoveredPlugin> = HashMap::new();
    let mut unmatched: Vec<&DiscoveredPlugin> = Vec::new();
    for d in &discovered {
        if BUNDLED_PLUGINS.iter().any(|entry| entry.id == d.id) {
            discovered_by_id.insert(d.id.as_str(), d);
        } else {
            unmatched.push(d);
        }
    }

    // ③ 前置面：宿主插槽支持集（§2.5 比对源）与启用插件 provides 并集
    //   （epoch 全局解析、与注册顺序无关，Pending 诊断的缺失键判定源）。
    let support = host_slot_support();
    let enabled_provides: BTreeSet<String> = BUNDLED_PLUGINS
        .iter()
        .filter(|entry| {
            enabled_of_with_file(entry, config, discovered_by_id.get(entry.id).map(|d| d.file.auto_enable))
        })
        .flat_map(|entry| (entry.manifest)().provides)
        .collect();

    let mut rows: Vec<PluginStatus> = Vec::new();
    let mut agent_loop: Option<FiberHandle> = None;

    // ③ 循环注册：for entry in 注册表 → ctx.plugin(entry, 段配置)
    //（单插件装载编排抽 load_plugin_entry，行数清偿批 task-22，零行为变更）
    for entry in BUNDLED_PLUGINS {
        let discovered = discovered_by_id.get(entry.id).copied();
        let (status, loop_handle) =
            load_plugin_entry(&root, entry, &raw, config, &store, discovered, &support, &enabled_provides).await?;
        if loop_handle.is_some() {
            agent_loop = loop_handle;
        }
        rows.push(status);
    }

    // 装载器 L1 未命中收口（设计 §3/§4 + task-10/K-19 刀2 market 分支）：
    // v2 market 声明（format=2+level=market）→ ProxyPlugin 子进程装载（spawn+握手
    // +tools_list+代理注册）；失败=单插件 Failed 不拖累其他插件（§4.4 语义差异）；
    // 其余未命中（v1 声明）保留 unsupported 诊断。
    load_market_plugins(&root, &unmatched, config, &mut rows).await;

    // ⑤ 插件状态数据随装配产物交宿主（方案 §2.4 第 5 步；任务 30-⑤ 启动净化：
    // 打印由 run.rs 按 --verbose 门控，装配层只产数据不产输出）

    // manifest_ui 配对校验（任务29 C4）：循环外统一查（时序稳定，见函数文档）。
    verify_manifest_ui_commands(&root);
    Ok(Assembled { root, agent_loop, plugin_status: rows })
}

/// 单插件装载编排（原 assemble 循环体，行数清偿批 task-22 拆出；零行为变更）：
/// 来源标注→漂移警告→禁用早退→双闸（静态校验+插槽版本）→注册装载→状态行。
/// 返回 (状态行, agent_loop 句柄归属)。
#[allow(clippy::too_many_arguments)]
async fn load_plugin_entry(
    root: &Context,
    entry: &crate::bundled::BundledEntry,
    raw: &Value,
    config: &DshConfig,
    store: &yunyou_credentials::HostCredentialStore,
    discovered: Option<&DiscoveredPlugin>,
    support: &std::collections::BTreeMap<String, u32>,
    enabled_provides: &BTreeSet<String>,
) -> Result<(PluginStatus, Option<FiberHandle>), AppError> {
    let manifest = (entry.manifest)();
    // 装载器 L1（K3 观测面）：plugins/ 目录有本插件声明 → 来源=discovered
    let source = if discovered.is_some() { "discovered" } else { "bundled" };
    // 声明投影漂移警告（D1 代码真源：文件是声明投影，漂移可见但不改装载行为）
    if let Some(d) = discovered {
        let f = &d.file;
        if f.contracts != manifest.contracts
            || f.provides != manifest.provides
            || f.inject != manifest.inject
        {
            eprintln!(
                "[装载器] 警告：插件 {} 的 manifest.toml 声明与内置实现漂移（contracts/provides/inject 以代码为准），请修正文件或删除",
                entry.id
            );
        }
    }
    // ② enable 判定（L1 优先级）：[plugins.<id>] 显式值 > 文件 auto_enable > 代码 auto_enable
    if !enabled_of_with_file(entry, config, discovered.map(|d| d.file.auto_enable)) {
        // 禁用者跳过并记日志（方案 §2.4 第 3 步；禁用=不注册，内核无此 fiber，无残尸）
        eprintln!("[装配] 插件 {} 已禁用（[plugins] 段），跳过装载", entry.id);
        return Ok((
            PluginStatus {
                id: entry.id.to_string(),
                version: manifest.version,
                enabled: false,
                state: "disabled",
                source,
            },
            None,
        ));
    }
    // 装载前静态校验（S1 入口，方案 §2.2）：违例拒装并逐条报出
    let issues = (entry.validate)();
    if !issues.is_empty() {
        return Err(teardown_fail(root, AppError::Assemble(format!(
            "插件 {} 装载前校验违例（拒装）：{}",
            entry.id,
            issues.join("；")
        ))).await);
    }
    // 插槽版本比对（方案 §2.5）：违例拒装，报「插槽版本不符」不静默降级
    let issues = check_slot_versions(entry.id, &manifest.contracts, support);
    if !issues.is_empty() {
        return Err(teardown_fail(root, AppError::Assemble(format!("插件 {} 拒装：{}", entry.id, issues.join("；")))).await);
    }
    let handle = register_entry(root, entry, raw, config, store, enabled_provides).await?;
    // ④-b Pending 诊断（§5.3 验收 1）：消费者「依赖未就绪」而非崩溃
    let state = handle.state();
    if state == FiberState::Pending {
        let missing: Vec<String> = manifest
            .inject
            .iter()
            .filter(|key| !enabled_provides.contains(*key))
            .cloned()
            .collect();
        eprintln!("[装配] {}", pending_diagnosis(entry.id, &missing));
    }
    // agent_loop 句柄归属判断后 move（原语义：非 agent_loop 的 handle 就地 drop）
    let loop_handle = if entry.id == AGENT_LOOP_ID { Some(handle) } else { None };
    Ok((
        PluginStatus {
            id: entry.id.to_string(),
            version: manifest.version,
            enabled: true,
            state: state.as_str(),
            source,
        },
        loop_handle,
    ))
}

/// 段值合并+引用解析+注册+await_ready（原循环体后半，task-22 拆出；零行为
/// 变更）。fail-loud 收口：任何失败路径先整树卸载再上抛——清单次序不构成豁免
///（llm 现居首位时直抛恰无实害，重排即漏卸，评审①②）。
async fn register_entry(
    root: &Context,
    entry: &crate::bundled::BundledEntry,
    raw: &Value,
    config: &DshConfig,
    store: &yunyou_credentials::HostCredentialStore,
    enabled_provides: &std::collections::BTreeSet<String>,
) -> Result<FiberHandle, AppError> {
    // 段值合并（顶层段为基，[plugins.<id>.config] 叠加）后注册。
    let mut cfg = match merge_segment(raw, entry.segment, config.plugins.entries.get(entry.id)) {
        Ok(cfg) => cfg,
        Err(err) => return Err(teardown_fail(root, err).await),
    };
    // S5 引用解析（裁决②）：llm 段 api_key 为 $NAME 引用时经宿主 store 解析
    // 为真值再透传（merge 后 register 前；未配置 fail-loud 不静默空串）。
    if entry.id == LLM_ID {
        if let Err(err) = resolve_llm_api_key(&mut cfg, &config.llm.api_key, store).await {
            return Err(teardown_fail(root, err).await);
        }
    }
    let handle = (entry.register)(root, cfg);
    // ④-b' 依赖不可满足预检（task-12 挂死雷，云-ya futex 复跑定谳）：
    // await_ready 的 Pending 语义=挂起等依赖就绪（内-P1-3 修复后）；提供者被
    // [plugins] 禁用时依赖**永不可满足**→await_ready 永久挂死启动。判定=注册后
    // 即刻 Pending 且 inject 中存在「不在启用插件 provides 并集」的键（缺失键在
    // 装配期内无任何 epoch 变化源）→ 不等待，诊断指路人话后按 Pending 继续装配
    // （§5.3 验收 1 既有语义：禁用非核心=消费者 Pending 不崩）；缺失键为空
    // （依赖由后续清单项装载的时序等待）才走 await_ready 正常等待。
    // 循环依赖双 Pending 形态不在本预检面（无 epoch 变化源同挂）——留待项，
    // 见台账 task-12 备注。
    if handle.state() == FiberState::Pending {
        let manifest = (entry.manifest)();
        let missing: Vec<String> = manifest
            .inject
            .iter()
            .filter(|key| !enabled_provides.contains(*key))
            .cloned()
            .collect();
        if !missing.is_empty() {
            eprintln!("[装配] {}（依赖不可满足=提供者被禁用或不存在，不等待）", pending_diagnosis(entry.id, &missing));
            return Ok(handle);
        }
    }
    // ④ await_ready：Pending 属稳定态立即返回；Failed 整树逆序卸载 fail-loud
    if let Err(err) = handle.await_ready().await {
        return Err(teardown_fail(root, AppError::Assemble(format!("组件 {} 装载失败：{err}", entry.id))).await);
    }
    Ok(handle)
}

/// market 插件装载分支（task-10/K-19 刀2；§5.1 代理插件映射）：
/// v2 声明 → market_manifest 五类权威校验 → ProxyPlugin（spawn+握手+代理注册）。
/// 单插件失败=Failed 态入表不拖累他插件（§4.4 与 L1 整树 fail-loud 的语义差异）；
/// 沙箱禁用态=拒装该插件 fail-loud 人话（「无沙箱不装载」，bash 降级语义不适用）。
async fn load_market_plugins(
    root: &Context,
    unmatched: &[&yunyou_config::DiscoveredPlugin],
    config: &DshConfig,
    rows: &mut Vec<PluginStatus>,
) {
    for d in unmatched {
        let is_v2_market =
            d.file.format == Some(2) && d.file.level == yunyou::manifest::PluginLevel::Market;
        if !is_v2_market {
            // v1 未命中：保留既有 unsupported 诊断（record_unmatched 单插件版）
            eprintln!(
                "[装载器] 插件 {}（{}）需 L2 子进程运行时，当前版本不支持装载——声明已发现，等待 L2 子进程骨架（设计 §4）",
                d.id,
                d.manifest_path.display()
            );
            rows.push(PluginStatus {
                id: d.id.clone(),
                version: d.file.version.clone(),
                enabled: false,
                state: "unsupported",
                source: "discovered",
            });
            continue;
        }
        // enable 判定（market 两级：[plugins.<id>] enable > 文件 auto_enable，§3.1）
        let enabled = config
            .plugins
            .entries
            .get(&d.id)
            .and_then(|e| e.enable)
            .unwrap_or(d.file.auto_enable);
        if !enabled {
            rows.push(PluginStatus {
                id: d.id.clone(),
                version: d.file.version.clone(),
                enabled: false,
                state: "disabled",
                source: "market",
            });
            continue;
        }
        let plugin_dir = d
            .manifest_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let manifest_text = match std::fs::read_to_string(&d.manifest_path) {
            Ok(text) => text,
            Err(err) => {
                push_market_failed(rows, &d.id, &d.file.version, format!("manifest 读取失败：{err}"));
                continue;
            }
        };
        let manifest_v2 = match crate::market_manifest::parse_and_validate(&manifest_text, &plugin_dir) {
            Ok(m) => m,
            Err(violations) => {
                push_market_failed(rows, &d.id, &d.file.version, violations.join("；"));
                continue;
            }
        };
        // 沙箱衔接（§4.1）：[sandbox] enabled=false → 拒装该插件（不降级直跑）
        let enforcer: Option<Arc<dyn yunyou_tools::sandbox::SandboxEnforcer>> =
            if config.sandbox.enabled {
                Some(Arc::new(yunyou_sandbox::LandlockEnforcer::new(
                    yunyou_sandbox::SandboxPluginConfig {
                        enabled: true,
                        writable_tmp: config.sandbox.writable_tmp,
                        network_denied: config.sandbox.network_denied,
                    },
                )))
            } else {
                None
            };
        let plugin = crate::proxy_plugin::ProxyPlugin {
            manifest_v2,
            plugin_dir,
            enforcer,
            spawn_form: crate::proxy_plugin::SpawnForm::Sandboxed,
        };
        let name = d.id.clone();
        let version = d.file.version.clone();
        let handle = root.plugin(plugin, Value::Null);
        match handle.await_ready().await {
            Ok(()) => rows.push(PluginStatus {
                id: name,
                version,
                enabled: true,
                state: "active",
                source: "market",
            }),
            Err(err) => push_market_failed(rows, &name, &version, err.to_string()),
        }
    }
}

/// market 单插件失败收口：Failed 态入观测面+stderr 人话（不拖累其他插件）。
fn push_market_failed(rows: &mut Vec<PluginStatus>, id: &str, version: &str, reason: String) {
    eprintln!("[装载器] market 插件 {id} 装载失败（该插件 Failed，不拖累其他插件）：{reason}");
    rows.push(PluginStatus {
        id: id.to_string(),
        version: version.to_string(),
        enabled: true,
        state: "failed",
        source: "market",
    });
}

/// 装载器 L1 未命中收口（原 assemble 内联段，task-22 拆出；零行为变更）：
/// 明确诊断不静默、不拒装；L2 落地后本分支换子进程 Runner（DiscoveredPlugin
/// 已带接缝字段）。
fn record_unmatched(unmatched: &[&DiscoveredPlugin], rows: &mut Vec<PluginStatus>) {
    for d in unmatched {
        eprintln!(
            "[装载器] 插件 {}（{}）需 L2 子进程运行时，当前版本不支持装载——声明已发现，等待 L2 子进程骨架（设计 §4）",
            d.id,
            d.manifest_path.display()
        );
        rows.push(PluginStatus {
            id: d.id.clone(),
            version: d.file.version.clone(),
            enabled: false,
            state: "unsupported",
            source: "discovered",
        });
    }
}

/// manifest_ui 配对校验（任务29 C4，设计 §八；原 assemble 内联段，task-22 拆出；
/// 零行为变更）：全部插件装载完成后统一比对 manifest.ui 声明的 Command{name}
/// 与注册表——find 不到→stderr warn 点名**不拒装**（register 可晚于本次校验
/// 发生，硬拒会误伤延迟注册）；循环外统一查而非循环内：commands 服务装载位次
/// 在清单后段，循环内查时序不稳。
fn verify_manifest_ui_commands(root: &Context) {
    if let Ok(commands) = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>()) {
        for entry in BUNDLED_PLUGINS {
            let manifest = (entry.manifest)();
            for ui in &manifest.ui {
                if let UiEntry::Command { name } = ui {
                    if commands.find(name).is_none() {
                        eprintln!(
                            "[装配] 警告：插件 {} 的 manifest.ui 声明命令 /{name} 未注册（register 缺位或晚于校验）",
                            entry.id
                        );
                    }
                }
            }
        }
    }
}

/// fail-loud 收口（承原 await_or_teardown 语义，方案 §2.4 第 4 步）：失败路径
/// 先整树逆序卸载（先消费者后提供者）再上抛错误，保证不留半装配状态
///（对齐 boot 的 fail-loud dispose）。Pending 不走此路径（属稳定态，§5.3 诊断）。
/// 收既有错误时直接透传（第 2 轮评审①：循环内所有失败路径统一走本收口，
/// 清单重排不产生「已装载 fiber 漏卸」窗口）。
async fn teardown_fail(root: &Context, error: AppError) -> AppError {
    let _ = root.dispose().await;
    error
}

/// 序列化配置并补缺省路径：
/// `session.path` 缺省 → `<yunyou-home>/sessions/default.jsonl`；
/// `credentials.store_path` 缺省 → `<yunyou-home>/credentials.toml`（S5）。
fn with_default_paths(config: &DshConfig, home: &Path) -> Value {
    let mut raw = serde_json::to_value(config).expect("DshConfig 序列化不应失败");
    if raw["session"]["path"].is_null() {
        raw["session"]["path"] = serde_json::json!(home.join("sessions/default.jsonl").display().to_string());
    }
    if raw["credentials"]["store_path"].is_null() {
        raw["credentials"]["store_path"] = serde_json::json!(home.join("credentials.toml").display().to_string());
    }
    raw
}

/// persona 桥接（审查 P1-3，B 线内桥接版）：系统提示词人设当前经 agent_loop
/// 段转发给 `SystemPrompt::compose`，而 yunyou-system-prompt 组件的 persona
/// 字段是**一期断头路**（组件内未被消费；根治=删组件死字段，已裁决归任务 12
/// 插件化方案）。本桥在装配序列里一处赋值：`agent_loop.persona` 缺省或空串时
/// 取 `system_prompt.persona` 值——使「yunyou.toml [system_prompt] persona」
/// 与 Default 品牌预设（yunyou_config::default_persona）实际生效。
///
/// 语义边界：用户显式配置的 `agent_loop.persona` 优先（尊重用户值）；
/// `system_prompt.persona` 为 None 时桥接为空串（无人设，与 compose 的空人设分支一致）。
fn bridge_persona(mut raw: Value, config: &DshConfig) -> Value {
    let inherited = config.system_prompt.persona.clone().unwrap_or_default();
    let missing = raw["agent_loop"]["persona"].as_str().map(str::is_empty).unwrap_or(true);
    if missing {
        raw["agent_loop"]["persona"] = Value::String(inherited);
    }
    raw
}

#[cfg(test)]
#[path = "assemble_tests.rs"]
mod tests;
