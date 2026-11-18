//! 沙箱契约面（契约⑨·task-19/P1-11a 阶段 1）：策略对象 v1+enforcer trait。
//!
//! 分层归属（设计 §2 裁决）：契约类型与 trait 落 yunyou-tools（主板规格层），
//! 实现（Landlock+seccomp 后端）落 yunyou-sandbox bundled 插件——依赖方向
//! 单向（插件依赖契约 crate，反向禁止）；内核与装配序列零改动。
//!
//! 四层阶梯定位（设计 §1）：本文件只承载 L2（workspace 授权快照，先授权后
//! 执行）与 L3（enforcer 契约）的数据面——L1 bash_guard 黑名单快筛不拆除
//! （D12 纵深防御），L4 二期不预造。

use std::path::PathBuf;

/// 策略快照（v1 冻结面；构造一次，Arc 贯穿 L2/L3，防两处解析漂移）。
///
/// v1 唯一语义=workspace-write（写白名单+网络禁断）；枚举留位不预造
///（只有一种可冻结语义时，枚举是猜测不是预留——设计 §5-1）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkspacePolicy {
    /// 可写根集合（全部 canonicalize 后绝对路径；含 workspace root、/dev/null、
    /// /tmp 与 TMPDIR 按 writable_tmp 开关并入——设计 §3.1）。
    pub writable_roots: Vec<PathBuf>,
    /// 出网许可。v1 语义=由 `[sandbox].network_denied` 派生（设计 §2 config 面
    /// 既定：denied=true→false 禁断=默认；denied=false→true 放开，重启生效）——
    /// 此前「恒 false」注释系未接线占位声明（task-24 接线时由云枢裁决更正，
    /// 字段集与 JSON 形态零变化，policy_version 保 1；NetworkMode 枚举留位二期）。
    pub network_access: bool,
    /// 策略语义版本（契约冻结位；helper 校验不识别即拒——设计 §3.1）。
    pub policy_version: u32,
}

impl WorkspacePolicy {
    /// v1 唯一语义构造：workspace-write（写白名单+网络禁断）。
    pub fn v1(writable_roots: Vec<PathBuf>) -> Self {
        Self { writable_roots, network_access: false, policy_version: 1 }
    }

    /// helper 传递序列化（argv JSON：--policy-json <json> -- <用户命令>）。
    /// 含可写根绝对路径，无敏感数据（设计 §3.4 C 案②）。
    pub fn to_policy_json(&self) -> String {
        serde_json::to_string(self).expect("WorkspacePolicy 序列化不应失败（纯数据字段）")
    }

    /// helper 侧反序列化+版本校验（不识别即拒——契约冻结位语义）。
    pub fn from_policy_json(json: &str) -> Result<Self, String> {
        let policy: Self =
            serde_json::from_str(json).map_err(|err| format!("策略 JSON 解析失败：{err}"))?;
        if policy.policy_version != 1 {
            return Err(format!(
                "策略版本不识别：{}（helper 支持 v1）",
                policy.policy_version
            ));
        }
        Ok(policy)
    }
}

/// 沙箱强制器产出：helper spawn 参数集（enforcer 预编译在 parent、应用在
/// helper——先授权后执行，执行期无二次路径解析，设计 §3.1）。
#[derive(Debug, Clone)]
pub struct PreparedSandbox {
    /// 策略 JSON（argv 传递段：--policy-json <json> -- <用户命令>）。
    pub policy_json: String,
    /// 用户命令工作目录（与规则根同源于一份 canonicalize 结果）。
    pub command_cwd: PathBuf,
}

impl PreparedSandbox {
    /// helper argv 构造（parent spawn 面）：program=当前二进制（arg0 自唤起，
    /// 设计 §3.4 C 案）；argv=[哨兵名, --policy-json, <json>, --, 用户命令...]。
    /// `--` 分隔防用户命令参数被 helper 误解析（设计 §3.4 C 案②）。
    pub fn helper_argv(&self, command: &[String]) -> (PathBuf, Vec<String>) {
        let program = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("yunyou-sandbox"));
        let mut argv = vec![
            "yunyou-sandbox".to_string(),
            "--policy-json".to_string(),
            self.policy_json.clone(),
            "--".to_string(),
        ];
        argv.extend(command.iter().cloned());
        (program, argv)
    }
}

/// 沙箱强制器契约（设计 §2：执行面=yunyou-sandbox bundled 插件提供，
/// tools 侧可选消费——enforcer 缺失=降级直跑+meta 标注，不瘫痪工具面）。
pub trait SandboxEnforcer: Send + Sync {
    /// 策略 → helper spawn 参数集。Err=NotEnforced（内核不支持 Landlock/
    /// 规则构建错）——fail-closed 拒执行不回退裸跑（设计 §3.6，无静默）。
    fn prepare(&self, policy: &WorkspacePolicy) -> std::io::Result<PreparedSandbox>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v1 默认面锁（task-24 裁 2）：默认构造 network_access=false（network_denied
    /// 默认 true 的派生面）+版本=1——锁默认不锁死语义（放开走显式配置+重启）。
    #[test]
    fn v1_冻结面_网络禁断与版本() {
        let policy = WorkspacePolicy::v1(vec!["/ws".into(), "/tmp".into()]);
        assert!(!policy.network_access, "v1 默认构造网络禁断（denied=true 派生面）");
        assert_eq!(policy.policy_version, 1);
        assert_eq!(policy.writable_roots.len(), 2);
    }

    /// helper 传递序列化 round-trip+版本校验拒绝（不识别即拒）。
    #[test]
    fn policy_json_roundtrip与版本拒() {
        let policy = WorkspacePolicy::v1(vec!["/ws".into()]);
        let json = policy.to_policy_json();
        let back = WorkspacePolicy::from_policy_json(&json).expect("round-trip 应成功");
        assert_eq!(back, policy);
        let bad = r#"{"writable_roots":["/ws"],"network_access":false,"policy_version":99}"#;
        assert!(WorkspacePolicy::from_policy_json(bad).is_err(), "版本不识别即拒");
    }

    /// C-1 修复锁（bash.rs prepare 接线）：enforcer 在位而 prepare 失败=
    /// fail-closed 拒执行（tool/sandbox-prepare+错误归因透传），不得静默降级
    /// 裸跑（旧 `.ok()` 行为）。mock enforcer 直接返回 Err 驱动拒绝臂。
    #[tokio::test(flavor = "multi_thread")]
    async fn prepare失败即拒不裸跑() {
        use crate::builtin::bash::BashTool;
        use crate::builtin::cwd::CwdService;
        use crate::contract::{ToolDefinition, ToolExec, ToolOutcome};
        use std::sync::Arc as StdArc;

        struct FailingEnforcer;
        impl SandboxEnforcer for FailingEnforcer {
            fn prepare(
                &self,
                _policy: &WorkspacePolicy,
            ) -> std::io::Result<PreparedSandbox> {
                Err(std::io::Error::other("模拟内核 Landlock 不可用"))
            }
        }

        let dir = std::env::temp_dir().join(format!("sbx-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = BashTool {
            cwd: StdArc::new(CwdService::new(&dir)),
            enforcer: Some(StdArc::new(FailingEnforcer)),
            tasks: None,
            background_after_secs: 60,
        };
        let exec = ToolExec {
            call_id: "sbx-fail".into(),
            cancel: tokio_util::sync::CancellationToken::new(),
            workspace: None,
        };
        let outcome = tool
            .execute(serde_json::json!({"cmd": "echo should-not-run"}), exec)
            .await;
        match &outcome {
            ToolOutcome::Err(err) => {
                assert_eq!(err.code, "tool/sandbox-prepare", "错误码锁死");
                assert!(err.message.contains("fail-closed"), "话术明写取舍：{}", err.message);
                assert!(err.message.contains("模拟内核 Landlock 不可用"), "底层错误归因透传：{}", err.message);
            }
            other => panic!("prepare 失败必须拒执行，实得：{other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
