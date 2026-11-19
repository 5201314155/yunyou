//! 权限判定 gate（task-16 S2：判定链 L0/L1/L2/L6 四层——第一个消费
//! [`yunyou_agent_loop::GateRegistryService`] 的策略卡；L5 工作区边界归 S4、
//! L7 模式矩阵归 S3，本 gate 对规则无命中返回 Pass 不越权定缺省）。

use crate::denylist;
use crate::rules::{Behavior, RuleSet};
use yunyou_agent_loop::{GateContext, GateDecision, PreToolUseGate, ToolCallView};
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 权限审批 gate：固定序 L0→规则三值→无命中 Pass。
pub struct PermissionGate {
    /// 预编译规则集（settings [permissions] 三值分列；S5 前静态——运行期
    /// 重载候 [permission] 配置热更卡）。
    rules: Arc<RuleSet>,
    /// L4 沙箱穿越开关（[permission] auto_allow_sandboxed；默认 false——
    /// 穿越默认关，愿进沙箱也要显式配置才生效，防静默放宽）。
    auto_allow_sandboxed: bool,
}

impl PermissionGate {
    /// 构造（规则集由装配侧加载编译后注入）。
    pub fn new(rules: RuleSet) -> Self {
        Self { rules: Arc::new(rules), auto_allow_sandboxed: false }
    }

    /// 设置 L4 穿越开关（[permission] auto_allow_sandboxed 装配透传）。
    pub fn set_auto_allow_sandboxed(&mut self, enabled: bool) {
        self.auto_allow_sandboxed = enabled;
    }
}

impl PreToolUseGate for PermissionGate {
    fn name(&self) -> &str {
        "permission-rules"
    }

    /// 执行序=10（早于缺省 100；免疫层与规则层应先于后续策略层）。
    fn order(&self) -> u32 {
        10
    }

    fn check(&self, call: &ToolCallView<'_>, _ctx: &GateContext) -> BoxFut<'_, Result<GateDecision, yunyou::CordisError>> {
        let rules = Arc::clone(&self.rules);
        let tool = call.name.to_string();
        let arguments = call.arguments.to_string();
        let mode = _ctx.mode.clone();
        let workspace_root = _ctx.workspace_root.clone();
        let sandbox_willing = _ctx.sandbox_willing;
        let auto_allow_sandboxed = self.auto_allow_sandboxed;
        Box::pin(async move {
            // L0 高危免疫层（最先判：任何规则/模式不可触达——设计稿 §一偏差 1）
            if let Some(reason) = denylist::check(&tool, &arguments) {
                return Ok(GateDecision::Deny { message: reason.into() });
            }
            // 内容提取：v1 用参数 JSON 的启发式内容位（command/path/url/file_path）
            // ——四族匹配器共用的「实际内容」单点，形态归一在此处（工具 schema
            // 各异，v1 抓常见键；抓不到=无内容，内容级规则不命中、整工具级照常）。
            let content = extract_content(&arguments);
            // L1/L2/L6 规则三值（deny>ask>allow 序，引擎内定序）
            match rules.decision_for(&tool, content.as_deref()) {
                Some(Behavior::Deny) => Ok(GateDecision::Deny {
                    message: format!("工具「{tool}」调用被 deny 规则拒绝（settings [permissions]）。"),
                }),
                // L7 模式后置变换：ask 规则仅 bypass 豁免（矩阵行「L2 ask 规则」）
                Some(Behavior::Ask) => {
                    if mode.as_deref().and_then(crate::mode::Mode::parse)
                        == Some(crate::mode::Mode::Bypass)
                    {
                        Ok(GateDecision::Allow { sandboxed: false })
                    } else if sandbox_willing == Some(true) && auto_allow_sandboxed {
                        // L4 沙箱穿越（claude :1088-1104 同律）：命令愿进沙箱
                        // ∧ 配置开 → 穿越 ask 规则自动放行（沙箱内执行标记）
                        Ok(GateDecision::Allow { sandboxed: true })
                    } else {
                        Ok(GateDecision::Ask { suggestions: vec![format!("allow {tool}")] })
                    }
                }
                Some(Behavior::Allow) => Ok(GateDecision::Allow { sandboxed: false }),
                // 无命中=L7 模式矩阵（§四表尾三行；mode 未知=Default 保守）
                None => {
                    let class = crate::mode::tool_class(&tool);
                    let parsed_mode = mode
                        .as_deref()
                        .and_then(crate::mode::Mode::parse)
                        .unwrap_or(crate::mode::Mode::Default);
                    let in_workspace = if class == crate::mode::ToolClass::Write {
                        in_workspace(content.as_deref(), workspace_root.as_deref())
                    } else {
                        None
                    };
                    match crate::mode::decide_unmatched(parsed_mode, class, in_workspace) {
                        crate::mode::Unmatched::Allow => Ok(GateDecision::Allow { sandboxed: false }),
                        crate::mode::Unmatched::Ask => Ok(GateDecision::Ask {
                            suggestions: vec![format!("allow {tool}")],
                        }),
                    }
                }
            }
        })
    }
}

/// 区内判定（写类）：S4 防逃逸版=boundary.rs canonicalize 三步（task-43
/// resolve_workspace 同款形态，symlink/../ 运行时闸；详见 boundary.rs 头注）。
fn in_workspace(path: Option<&str>, root: Option<&str>) -> Option<bool> {
    crate::boundary::in_workspace(path, root)
}

/// 参数 JSON 内容位提取（v1 启发式：常见键单点归一；deny_unknown_fields 的
/// 工具 schema 演进时在此处加键——加法演进点注记）。
fn extract_content(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    for key in ["command", "path", "file_path", "url", "pattern"] {
        if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // 测试断言惯例（同包既有测试同律）
mod tests {
    use super::*;

    fn gate_with(section: crate::rules::PermissionsSection) -> PermissionGate {
        PermissionGate::new(crate::rules::RuleSet::compile(&section).unwrap())
    }

    fn call<'a>(name: &'a str, arguments: &'a str) -> ToolCallView<'a> {
        ToolCallView { name, arguments }
    }

    /// 判定链四层：L0 免疫压规则、deny 规则、ask 规则、allow 规则、无命中 Pass。
    #[tokio::test]
    async fn 判定链四层() {
        let gate = gate_with(crate::rules::PermissionsSection {
            allow: vec!["Bash(cargo:*)".into(), "Bash(rm -rf /tmp/build:*)".into()],
            ask: vec!["write".into()],
            deny: vec![],
        });
        // L0：allow 规则也压不过免疫层（不可触达）
        let d = gate.check(&call("bash", "{\"command\":\"rm -rf /\"}"), &GateContext::default()).await.unwrap();
        assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
        // deny 规则
        let gate2 = gate_with(crate::rules::PermissionsSection {
            allow: vec![], ask: vec![], deny: vec!["Bash(git push --force:*)".into()],
        });
        let d = gate2.check(&call("bash", "{\"command\":\"git push --force origin main\"}"), &GateContext::default()).await.unwrap();
        assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
        // ask 规则（含 suggestions）
        let d = gate.check(&call("write", "{\"path\":\"/tmp/x\"}"), &GateContext::default()).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "{d:?}");
        // allow 规则
        let d = gate.check(&call("bash", "{\"command\":\"cargo build\"}"), &GateContext::default()).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: false });
        // 无命中=模式矩阵（S3）：web_fetch=读类→Allow（读类任何模式放行）
        let d = gate.check(&call("web_fetch", "{\"url\":\"https://docs.rs\"}"), &GateContext::default()).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: false });
    }

    /// S3 模式矩阵端到端：default 区内写 Allow/区外写 Ask；plan 写 Ask 读
    /// Allow；bypass 豁免 ask 规则但 deny 照拒；mode 未知=Default 保守。
    #[tokio::test]
    async fn s3_模式矩阵端到端() {
        let gate = gate_with(crate::rules::PermissionsSection {
            allow: vec![],
            ask: vec!["Bash(npm publish:*)".into()],
            deny: vec!["Bash(rm -rf:*)".into()],
        });
        let ctx = |mode: Option<&str>, root: Option<&str>| GateContext {
            mode: mode.map(String::from),
            workspace_root: root.map(String::from),
            sandbox_willing: None,
        };
        // default：区内写 Allow / 区外写 Ask
        let d = gate.check(&call("write", "{\"path\":\"/ws/a.txt\"}"), &ctx(Some("default"), Some("/ws"))).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: false });
        let d = gate.check(&call("write", "{\"path\":\"/etc/x\"}"), &ctx(Some("default"), Some("/ws"))).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "{d:?}");
        // plan：写 Ask（区内也问）、读 Allow、执行 Ask
        let d = gate.check(&call("write", "{\"path\":\"/ws/a.txt\"}"), &ctx(Some("plan"), Some("/ws"))).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "plan 区内写也问：{d:?}");
        let d = gate.check(&call("read", "{}"), &ctx(Some("plan"), Some("/ws"))).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: false });
        // bypass：ask 规则豁免 Allow；deny 规则照拒（不变式 3）
        let d = gate.check(&call("bash", "{\"command\":\"npm publish\"}"), &ctx(Some("bypass"), None)).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: false });
        let d = gate.check(&call("bash", "{\"command\":\"rm -rf /tmp/x\"}"), &ctx(Some("bypass"), None)).await.unwrap();
        assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
        // bypass 无命中执行类也 Allow
        let d = gate.check(&call("bash", "{\"command\":\"echo hi\"}"), &ctx(Some("bypass"), None)).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: false });
        // acceptEdits（驼峰兼容）：区内写 Allow/执行 Ask
        let d = gate.check(&call("write", "{\"path\":\"/ws/a.txt\"}"), &ctx(Some("acceptEdits"), Some("/ws"))).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: false });
        let d = gate.check(&call("bash", "{\"command\":\"make\"}"), &ctx(Some("accept_edits"), Some("/ws"))).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "{d:?}");
        // mode 未知=Default 保守（未知工具=执行类 Ask）
        let d = gate.check(&call("神奇工具", "{}"), &ctx(Some("yolo"), None)).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "{d:?}");
    }

    /// S4 L4 沙箱穿越三态：无意愿=Ask；有意愿+开关开=Allow{sandboxed:true}；
    /// 有意愿+开关关=Ask（穿越默认关防静默放宽）。
    #[tokio::test]
    async fn s4_沙箱穿越三态() {
        let gate = gate_with(crate::rules::PermissionsSection {
            allow: vec![], ask: vec!["Bash(cargo:*)".into()], deny: vec![],
        });
        let ctx = |willing: Option<bool>| GateContext {
            mode: Some("default".into()),
            workspace_root: None,
            sandbox_willing: willing,
        };
        // 无意愿信号：Ask
        let d = gate.check(&call("bash", "{\"command\":\"cargo build\"}"), &ctx(None)).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "{d:?}");
        // 有意愿+开关关（默认）：仍 Ask
        let d = gate.check(&call("bash", "{\"command\":\"cargo build\"}"), &ctx(Some(false))).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "{d:?}");
        // 有意愿+开关开：Allow{sandboxed:true}
        let mut open = PermissionGate::new(crate::rules::RuleSet::compile(&crate::rules::PermissionsSection {
            allow: vec![], ask: vec!["Bash(cargo:*)".into()], deny: vec![],
        }).unwrap());
        open.set_auto_allow_sandboxed(true);
        let d = open.check(&call("bash", "{\"command\":\"cargo build\"}"), &ctx(Some(true))).await.unwrap();
        assert_eq!(d, GateDecision::Allow { sandboxed: true });
        // 穿越不豁免 deny 规则（L1 前于 L4）
        let d = open.check(&call("bash", "{\"command\":\"rm -rf /tmp/x\"}"), &ctx(Some(true))).await.unwrap();
        assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
    }

    /// 内容位提取启发式：command/path/url 各键；无键=无内容（内容级规则不命中）。
    #[test]
    fn 内容位提取() {
        assert_eq!(extract_content("{\"command\":\"ls\"}").as_deref(), Some("ls"));
        assert_eq!(extract_content("{\"path\":\"/a/b\"}").as_deref(), Some("/a/b"));
        assert_eq!(extract_content("{\"url\":\"https://x\"}").as_deref(), Some("https://x"));
        assert_eq!(extract_content("{\"other\":1}"), None);
        assert_eq!(extract_content("not-json"), None);
    }

    /// 非 bash 工具不进 L0 扫描（清单语义=命令毁灭面）：write 带毁灭文本参数
    /// 不触发 L0（文本非命令），走 S3 矩阵——write=写类、root 未知=保守 Ask。
    #[tokio::test]
    async fn 非bash工具不进l0() {
        let gate = PermissionGate::new(crate::rules::RuleSet::default());
        let d = gate.check(&call("write", "{\"path\":\"rm -rf /\"}"), &GateContext::default()).await.unwrap();
        assert!(matches!(d, GateDecision::Ask { .. }), "{d:?}");
        // bash 带同文本=L0 命中恒 Deny（对照）
        let d = gate.check(&call("bash", "{\"command\":\"rm -rf /\"}"), &GateContext::default()).await.unwrap();
        assert!(matches!(d, GateDecision::Deny { .. }), "{d:?}");
    }
}
