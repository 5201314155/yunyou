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
}

impl PermissionGate {
    /// 构造（规则集由装配侧加载编译后注入）。
    pub fn new(rules: RuleSet) -> Self {
        Self { rules: Arc::new(rules) }
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
                Some(Behavior::Ask) => Ok(GateDecision::Ask {
                    suggestions: vec![format!("allow {tool}")],
                }),
                Some(Behavior::Allow) => Ok(GateDecision::Allow { sandboxed: false }),
                // 无命中=Pass（缺省行为归模式层 S3；本引擎不越权定缺省）
                None => Ok(GateDecision::Pass),
            }
        })
    }
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
        // 无命中=Pass（缺省归 S3 模式层）
        let d = gate.check(&call("web_fetch", "{\"url\":\"https://docs.rs\"}"), &GateContext::default()).await.unwrap();
        assert_eq!(d, GateDecision::Pass);
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

    /// 非 bash 工具不进 L0 扫描（清单语义=命令毁灭面）。
    #[tokio::test]
    async fn 非bash工具不进l0() {
        let gate = PermissionGate::new(crate::rules::RuleSet::default());
        let d = gate.check(&call("write", "{\"path\":\"rm -rf /\"}"), &GateContext::default()).await.unwrap();
        assert_eq!(d, GateDecision::Pass);
    }
}
