//! 沙箱插件（task-19/K-10·P1-11a 实现批）：Landlock 写白名单+seccomp 网络禁断
//! +NNP+arg0 自唤起 helper 进程（设计 §3.4 C 案）+pre-main 加固三件套。
//!
//! 执行面（设计 §2）：provide `dyn SandboxEnforcer`（契约面在 yunyou-tools
//! sandbox.rs）——enforcer 是 tools 侧**可选消费**（不进 inject：禁用沙箱不
//! 瘫痪工具面），缺失=降级直跑+meta 标注（设计 §2 裁决 1）。
//!
//! # 降级语义（设计 §3.6，全部显式无静默）
//! - `[sandbox] enabled=false`：不 provide+warn 日志（降级唯一受控入口）；
//! - helper 应用失败（NotEnforced 等）：helper 专用退出码→parent 归一
//!   `tool/sandbox-failed` 拒执行，不回退裸跑。

use yunyou::projection_of;
use yunyou::{
    ConfigValue, Context, CordisError, ErrorKind, FiberOutput, Plugin, PluginLevel, PluginManifest,
    Result, ServiceId,
};
pub mod helper;

use yunyou_system_prompt::{Section, SystemPrompt};
use yunyou_tools::sandbox::{PreparedSandbox, SandboxEnforcer, WorkspacePolicy};
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 插件配置（`[sandbox]` 段镜像，设计 §2 config 面）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxPluginConfig {
    /// 沙箱总开关（volatile 热调：改配置重启生效；false=显式降级直跑）。
    pub enabled: bool,
    /// /tmp 与 TMPDIR 并入可写根（默认 true——bash pwd 捕获临时文件/编译器
    /// 临时文件落在 std::env::temp_dir()，不入集合则统一目录视图失效）。
    pub writable_tmp: bool,
    /// 网络禁断开关（默认 true；改配置重启生效——运行中放开网络=安全面突变，
    /// 设计 §2 config 面明示不热调）。
    pub network_denied: bool,
}

impl Default for SandboxPluginConfig {
    fn default() -> Self {
        Self { enabled: true, writable_tmp: true, network_denied: true }
    }
}

/// Landlock+seccomp 后端的 enforcer（parent 侧：策略构造+helper 参数集预编译；
/// 内核应用在 helper 进程——设计 §3.4 C 案「预编译在 parent、应用在 helper」）。
///
/// 持有 `[sandbox]` 段配置（task-24 接线）：策略完成点——bash 构造侧只声明
/// workspace 根，本类型在 prepare 时按配置补全（/tmp+TMPDIR 并入、网络语义
/// 落字段），`[sandbox]` 配置消费单源收在本 crate 不外泄 tools。
#[derive(Debug, Clone)]
pub struct LandlockEnforcer {
    cfg: SandboxPluginConfig,
}

impl LandlockEnforcer {
    /// setup 期以解析后的插件配置构造。
    pub fn new(cfg: SandboxPluginConfig) -> Self {
        Self { cfg }
    }
}

impl SandboxEnforcer for LandlockEnforcer {
    fn prepare(&self, policy: &WorkspacePolicy) -> std::io::Result<PreparedSandbox> {
        // 策略完成点（task-24）：
        // ① writable_tmp=true（默认）→ /tmp 与 $TMPDIR 并入可写根——bash pwd
        //    捕获临时文件与编译器临时文件落在 std::env::temp_dir()，不并入则
        //    统一目录视图失效、工具链临时文件全被拒（设计 §可写根构造原文）；
        // ② network_access = !network_denied（设计 §2 config 面既定语义，
        //    此前契约草图「恒 false」为未接线占位声明——云枢裁 2）。
        let mut effective = policy.clone();
        if self.cfg.writable_tmp {
            let mut tmp_roots = vec![std::env::temp_dir()];
            if let Some(tmpdir) = std::env::var_os("TMPDIR") {
                let tmpdir = std::path::PathBuf::from(tmpdir);
                if tmpdir.is_absolute() && !tmp_roots.contains(&tmpdir) {
                    tmp_roots.push(tmpdir);
                }
            }
            for root in tmp_roots {
                if !effective.writable_roots.contains(&root) {
                    effective.writable_roots.push(root);
                }
            }
        }
        effective.network_access = !self.cfg.network_denied;
        // 预编译（parent）：策略冻结为 argv JSON（可写根绝对路径已在构造侧
        // canonicalize；helper 侧从 JSON 重建规则——执行期零二次路径解析）。
        Ok(PreparedSandbox {
            policy_json: effective.to_policy_json(),
            command_cwd: effective
                .writable_roots
                .first()
                .cloned()
                .unwrap_or_else(std::env::temp_dir),
        })
    }
}

/// 模型告知段（设计 §2.1，lead 修正案④）：诚实律——段文本与执行面同源，
/// 仅在沙箱实际生效（enabled=true）时注册；文案四条行为学依据见设计 §2.1
/// （实现名不写、挑衅句式不写、临时目录内部机械面不写）。
const SECTION_NAME: &str = "harness:sandbox";
/// 段位：紧邻工具使用指导段（ORDER_TOOLS=1000）之后——先知道怎么用工具，
/// 再知道这些工具跑在什么边界里。
const ORDER_SANDBOX: f64 = 1100.0;

const SANDBOX_SECTION_TEXT: &str = "## 沙箱执行环境\n\
    你的 bash 命令在系统沙箱中执行。这是内核级硬边界，不是可协商的规则：\n\
    - **可写范围**：仅当前工作区目录。工作区之外的任何写入都会被拒绝——需要写工作区之外的位置时，停下来向用户说明诉求（目标路径与原因），由用户决定调整工作区或自行操作，不要尝试绕过。\n\
    - **无网络**：命令不能访问网络。`git clone`、`cargo add`、`pip install` 等需要下载的操作会失败，这是预期行为，不会因重试而改变；优先使用本地已有的依赖与仓库，确需联网时停下来告知用户。\n\
    - **权限拒绝≠故障**：看到 `Permission denied`、`Operation not permitted` 或命令异常退出时，先判断是否触碰了上述边界；若是边界拒绝，不要换写法反复重试（换 shell 语法、编码、间接调用都不会成功），直接改道或说明原因。\n\
    - **工作区内的常规开发动作（读写文件、构建、测试）不受影响。**";

/// 沙箱插件（bundled：Cargo workspace member+装配代码注册，非运行时发现）。
pub struct SandboxPlugin;

impl Plugin for SandboxPlugin {
    fn name(&self) -> &str {
        "yunyou-sandbox"
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-sandbox".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["sandbox.v1".into()],
            provides: vec![projection_of::<dyn SandboxEnforcer>()],
            inject: vec![],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![] // enforcer 是可选消费（tools 侧 get），不进 inject 强依赖
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        serde_json::to_value(schemars::schema_for!(SandboxPluginConfig)).ok()
    }

    fn setup(
        self: Arc<Self>,
        ctx: Context,
        config: ConfigValue,
    ) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: SandboxPluginConfig = serde_json::from_value(config).map_err(|err| {
                CordisError::new(ErrorKind::Internal, format!("sandbox/配置解析失败: {err}"))
            })?;
            if !cfg.enabled {
                // 显式降级（设计 §3.6）：不 provide+warn——降级唯一受控入口。
                eprintln!(
                    "[sandbox] 沙箱已禁用（[sandbox] enabled=false）：bash 子进程将以无内核沙箱方式直跑"
                );
                return Ok(FiberOutput);
            }
            ctx.provide(
                ServiceId::of::<dyn SandboxEnforcer>(),
                Arc::new(LandlockEnforcer::new(cfg)),
            )?;
            // 模型告知段（设计 §2.1 诚实律）：沙箱实际生效才注册；system-prompt
            // 缺席（未装配）=告知面缺席但执行面照常——降级显式不静默。
            match ctx.get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>()) {
                Ok(prompt) => {
                    let section = Section::new(
                        SECTION_NAME,
                        ORDER_SANDBOX,
                        SANDBOX_SECTION_TEXT.replace("\\n", "\n"),
                    );
                    let disposer = prompt.register_section(section)
                        .map_err(|err| {
                            CordisError::new(
                                ErrorKind::Internal,
                                format!("sandbox/告知段注册失败: {err}"),
                            )
                        })?;
                    // 注册即 effect（方案 §2.1.4）：插件卸载时段随注销（热插拔不留残尸）
                    ctx.effect("sandbox.告知段", move || Ok(Some(disposer)))?;
                }
                Err(err) => {
                    eprintln!(
                        "[sandbox] 告知段未注册（system-prompt 服务缺席: {err}）——模型面与执行面不同源，属装配异常"
                    );
                }
            }
            Ok(FiberOutput)
        })
    }
}

/// 兜底引用（pub use 供装配面类型推导；阶段 4 bash 接线消费）。
pub use SandboxPluginConfig as SandboxConfigMirror;

// 占位引用防未使用告警（阶段 3 helper 分支消费）：
// ErrorKind/CordisError 由 helper 归一错误路径消费。
#[allow(dead_code)]
fn _error_kinds_marker(_: ErrorKind, _: CordisError) {}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou::Context;

    /// 告知段三态（设计 §2.1 诚实律验收，裁 4）：
    /// ①enabled=true → 段在（模型面与执行面同源）；
    /// ②enabled=false → 段不在（降级显式，宁缺勿谎）；
    /// ③插件卸载 → 段消失（注册即 effect，热插拔不留残尸）。
    #[tokio::test]
    async fn 告知段三态() {
        // ① enabled=true：段在
        let root = Context::new();
        let sp = root.plugin(SystemPromptPluginMirror, serde_json::json!({}));
        sp.await_ready().await.expect("system-prompt 装载");
        let sb = root.plugin(SandboxPlugin, serde_json::json!({ "enabled": true }));
        sb.await_ready().await.expect("sandbox 装载");
        let prompt = root
            .get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>())
            .expect("段缝服务");
        let composed = prompt.compose("", &[]);
        assert!(composed.contains("沙箱执行环境"), "生效态应含告知段：{composed}");

        // ③ 卸载 sandbox 插件 → 段消失（注册即 effect 的注销路径）
        sb.dispose().await.expect("dispose 应成功");
        let composed_after = prompt.compose("", &[]);
        assert!(!composed_after.contains("沙箱执行环境"), "卸载后段应消失");

        // ② enabled=false：段不在（独立 root 防串扰）
        let root2 = Context::new();
        let sp2 = root2.plugin(SystemPromptPluginMirror, serde_json::json!({}));
        sp2.await_ready().await.expect("system-prompt 装载");
        let sb2 = root2.plugin(SandboxPlugin, serde_json::json!({ "enabled": false }));
        sb2.await_ready().await.expect("sandbox 降级装载应成功");
        let prompt2 = root2
            .get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>())
            .expect("段缝服务");
        let composed2 = prompt2.compose("", &[]);
        assert!(!composed2.contains("沙箱执行环境"), "降级态段不在（诚实律）");
    }

    /// 测试面镜像插件（system-prompt 装配最小壳；真实插件在 yunyou-system-prompt
    /// crate 内部——集成测试经其 pub 面装配）。
    use yunyou_system_prompt::SystemPromptPlugin as SystemPromptPluginMirror;
}

#[cfg(test)]
mod prepare_tests {
    use super::*;

    /// prepare 策略完成点（task-24 裁 1）：writable_tmp 并入 /tmp+TMPDIR、
    /// network_access 由 network_denied 派生——[sandbox] 配置消费单源验证。
    #[test]
    fn prepare按配置补全策略() {
        let enf = LandlockEnforcer::new(SandboxPluginConfig::default());
        let policy = WorkspacePolicy::v1(vec!["/ws".into()]);
        let prepared = enf.prepare(&policy).expect("prepare 应成功");
        let json = prepared.policy_json;
        assert!(json.contains(&std::env::temp_dir().to_string_lossy().to_string()), "应并入 /tmp：{json}");
        assert!(json.contains("\"network_access\":false"), "默认 network_denied=true→禁断：{json}");

        // 放开形态：network_denied=false → network_access=true（重启生效语义）
        let enf_open = LandlockEnforcer::new(SandboxPluginConfig {
            network_denied: false,
            ..SandboxPluginConfig::default()
        });
        let prepared_open = enf_open.prepare(&policy).expect("prepare 应成功");
        assert!(
            prepared_open.policy_json.contains("\"network_access\":true"),
            "denied=false→放开：{}",
            prepared_open.policy_json
        );

        // writable_tmp=false：不并入（显式关闸语义）
        let enf_no_tmp = LandlockEnforcer::new(SandboxPluginConfig {
            writable_tmp: false,
            ..SandboxPluginConfig::default()
        });
        let prepared_no = enf_no_tmp.prepare(&policy).expect("prepare 应成功");
        assert!(
            !prepared_no.policy_json.contains(&std::env::temp_dir().to_string_lossy().to_string()),
            "writable_tmp=false 不应并入 /tmp：{}",
            prepared_no.policy_json
        );
    }
}
