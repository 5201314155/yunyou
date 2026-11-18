//! 规则语法与三值分列（task-16 S2，设计稿 §三；语法照抄 claude
//! `Tool(content)`+括号转义，permissionRuleParser.ts:41-49 同序）。

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 规则行为三值（PermissionRule.ts 同构）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    /// 拒绝。
    Deny,
    /// 问人。
    Ask,
    /// 放行。
    Allow,
}

/// 单条规则（预解析形态；字符串形态见 [`parse_rule`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// 工具名（归一后；未知工具名注册时报错不静默）。
    pub tool: String,
    /// 内容限定（None=整工具级；Some=内容级 `Tool(content)`）。
    pub content: Option<String>,
    /// 行为。
    pub behavior: Behavior,
}

/// 旧名归一表（LEGACY_TOOL_NAME_ALIASES 同构；v1 起步空表+两条 claude 已知别名）。
pub(crate) const LEGACY_TOOL_NAME_ALIASES: &[(&str, &str)] = &[
    ("Task", "Agent"),
    ("Bash", "bash"),
    ("Read", "read"),
    ("Write", "write"),
    ("Edit", "edit"),
];

/// 工具名归一（旧名别名→现名；未知工具名在此层不拒——匹配器按现名表判定）。
pub(crate) fn normalize_tool_name(name: &str) -> String {
    LEGACY_TOOL_NAME_ALIASES
        .iter()
        .find(|(legacy, _)| *legacy == name)
        .map(|(_, current)| current.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// 括号转义（escapeRuleContent 同序：反斜杠先转、括号后转）。
pub fn escape_rule_content(content: &str) -> String {
    content.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)")
}

/// 括号反转义（unescape 反序：括号先解、反斜杠后解）。
pub(crate) fn unescape_rule_content(content: &str) -> String {
    content.replace("\\(", "(").replace("\\)", ")").replace("\\\\", "\\")
}

/// 解析单条规则字符串：`Tool`（整工具级）或 `Tool(content)`（内容级）。
/// 形态错误（空名/空 content/不配对括号）=Err 上抛（注册时报错不静默）。
pub fn parse_rule(text: &str, behavior: Behavior) -> Result<Rule, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("规则为空串".into());
    }
    let (open, close) = (text.find('('), text.rfind(')'));
    match (open, close) {
        (None, None) => Ok(Rule {
            tool: normalize_tool_name(text),
            content: None,
            behavior,
        }),
        (Some(open), Some(close)) if close > open => {
            let tool = normalize_tool_name(text[..open].trim());
            if tool.is_empty() {
                return Err(format!("规则「{text}」工具名为空"));
            }
            let raw = &text[open + 1..close];
            if raw.is_empty() {
                return Err(format!("规则「{text}」内容为空（整工具级请去掉括号）"));
            }
            if !text[close + 1..].trim().is_empty() {
                return Err(format!("规则「{text}」尾部有多余字符"));
            }
            Ok(Rule {
                tool,
                content: Some(unescape_rule_content(raw)),
                behavior,
            })
        }
        _ => Err(format!("规则「{text}」括号不配对")),
    }
}

/// 标准 allow 预设（池续卡 2026-11-19，web-search 联动；设计稿 §四.2）：免打扰
/// 白名单——用户 settings 未配置时的基线放行。v1 口径：web_search 整工具级
/// （搜索目标 url 在结果内动态产生，执行前 gate 看不到具体域名；四域名
/// duckduckgo.com/searxng 实例/api.search.brave.com/api.tavily.com 的
/// `domain:` 内容级规则留待 web_fetch 类工具——url 在参数面可判时启用）。
pub const STANDARD_ALLOW_PRESET: &[&str] = &["web_search"];

/// settings.json `[permissions]` 段（三值分列；分源 v1=user 只读，写回候 S5）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionsSection {
    /// allow 列表。
    #[serde(default)]
    pub allow: Vec<String>,
    /// ask 列表。
    #[serde(default)]
    pub ask: Vec<String>,
    /// deny 列表。
    #[serde(default)]
    pub deny: Vec<String>,
}

/// 规则集（预编译后的内存形态；匹配见 [`crate::matchers`]）。
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    /// 全部规则（判定按 deny>ask>allow 序扫，见 [`RuleSet::decision_for`]）。
    pub rules: Vec<Rule>,
}

impl RuleSet {
    /// 从 settings 段编译（单条解析失败=Err 带条目定位——不静默丢规则）。
    pub fn compile(section: &PermissionsSection) -> Result<Self, String> {
        let mut rules = Vec::new();
        for (behavior, list, label) in [
            (Behavior::Deny, &section.deny, "deny"),
            (Behavior::Ask, &section.ask, "ask"),
            (Behavior::Allow, &section.allow, "allow"),
        ] {
            for text in list {
                rules.push(parse_rule(text, behavior).map_err(|e| format!("[{label}] {e}"))?);
            }
        }
        // 标准 allow 预设并入（deny/ask 用户规则压过预设=判定序 deny>ask>allow
        // 天然保证；预设只兜底免打扰，不缩用户权限面）
        for text in STANDARD_ALLOW_PRESET {
            rules.push(parse_rule(text, Behavior::Allow).map_err(|e| format!("[标准预设] {e}"))?);
        }
        Ok(Self { rules })
    }

    /// 从文件加载（settings.json 缺失=空集，诚实不猜——设计稿 §五：v1 先落
    /// user 源只读；分源合并候配置分源卡，TODO 挂此处改造点）。
    pub fn load_user_settings(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            // 缺文件也过 compile：标准 allow 预设（STANDARD_ALLOW_PRESET）在
            // 任何装载路径都在位——零配置基线一致，防「有配置反而少放行」
            return Self::compile(&PermissionsSection::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        #[derive(Deserialize, Default)]
        #[serde(default, deny_unknown_fields)]
        struct SettingsFile {
            #[serde(default)]
            permissions: PermissionsSection,
        }
        let file: SettingsFile = serde_json::from_str(&text)
            .map_err(|e| format!("解析 {} 失败：{e}", path.display()))?;
        Self::compile(&file.permissions)
    }

    /// 判定：deny>ask>allow 序扫（claude 1a/1b/2c 同序）；无命中=None
    ///（缺省行为归模式层 S3——本引擎只答规则命中态）。
    pub fn decision_for(&self, tool: &str, content: Option<&str>) -> Option<Behavior> {
        let tool = normalize_tool_name(tool);
        let order = |b: Behavior| match b {
            Behavior::Deny => 0,
            Behavior::Ask => 1,
            Behavior::Allow => 2,
        };
        let mut hits: Vec<Behavior> = self
            .rules
            .iter()
            .filter(|rule| {
                rule.tool == tool
                    && crate::matchers::content_matches(rule.content.as_deref(), content)
            })
            .map(|rule| rule.behavior)
            .collect();
        hits.sort_by_key(|b| order(*b));
        hits.into_iter().next()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // 测试断言惯例（同包既有测试同律）
mod tests {
    use super::*;

    /// 解析四态：整工具级/内容级/转义还原/形态错误。
    #[test]
    fn 规则解析四态() {
        let whole = parse_rule("WebSearch", Behavior::Allow).unwrap();
        assert_eq!(whole.tool, "WebSearch");
        assert_eq!(whole.content, None);
        let content = parse_rule("Bash(cargo build:*)", Behavior::Allow).unwrap();
        assert_eq!(content.tool, "bash");
        assert_eq!(content.content.as_deref(), Some("cargo build:*"));
        // 转义：字面括号进 content 需转义，反转义后还原
        let escaped = parse_rule("Bash(echo \\(paren\\))", Behavior::Ask).unwrap();
        assert_eq!(escaped.content.as_deref(), Some("echo (paren)"));
        assert_eq!(escape_rule_content("a(b)\\c"), "a\\(b\\)\\\\c");
        // 形态错误
        assert!(parse_rule("", Behavior::Deny).is_err());
        assert!(parse_rule("Bash()", Behavior::Deny).is_err());
        assert!(parse_rule("Bash(x", Behavior::Deny).is_err());
        assert!(parse_rule("Bash(x))tail", Behavior::Deny).is_err());
    }

    /// 判定序 deny>ask>allow+内容级/整工具级双层匹配+无命中 None。
    #[test]
    fn 判定序与双层匹配() {
        let set = RuleSet::compile(&PermissionsSection {
            allow: vec!["bash".into(), "Bash(cargo:*)".into()],
            ask: vec!["Bash(npm publish:*)".into()],
            deny: vec!["Bash(rm -rf:*)".into()],
        })
        .unwrap();
        // deny 压过 allow（同工具双层命中取严）
        assert_eq!(
            set.decision_for("bash", Some("rm -rf /")),
            Some(Behavior::Deny)
        );
        assert_eq!(set.decision_for("bash", Some("cargo build")), Some(Behavior::Allow));
        assert_eq!(set.decision_for("bash", Some("npm publish")), Some(Behavior::Ask));
        // 整工具级 allow：任意内容命中
        assert_eq!(set.decision_for("bash", Some("ls -la")), Some(Behavior::Allow));
        // 无命中
        assert_eq!(set.decision_for("web_fetch", Some("https://x")), None);
    }

    /// settings 加载：缺文件=空集；坏规则=Err 带定位；坏 JSON=Err。
    #[test]
    fn settings加载三态() {
        let dir = std::env::temp_dir().join(format!("perm-s2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 缺文件=标准预设基线（web_search 免打扰放行，见 STANDARD_ALLOW_PRESET）
        let absent = RuleSet::load_user_settings(&dir.join("absent.json")).unwrap();
        assert_eq!(absent.rules.len(), STANDARD_ALLOW_PRESET.len());
        // 好文件
        let good = dir.join("settings.json");
        std::fs::write(
            &good,
            r#"{"permissions":{"allow":["bash"],"deny":["Bash(rm -rf:*)"]}}"#,
        )
        .unwrap();
        let set = RuleSet::load_user_settings(&good).unwrap();
        assert_eq!(set.rules.len(), 2 + STANDARD_ALLOW_PRESET.len(), "用户 2 条+标准预设");
        // 坏规则=Err 带定位
        let bad = dir.join("bad.json");
        std::fs::write(&bad, r#"{"permissions":{"deny":["Bash("]}}"#).unwrap();
        let err = RuleSet::load_user_settings(&bad).unwrap_err();
        assert!(err.contains("[deny]"), "{err}");
        // 坏 JSON=Err
        let worse = dir.join("worse.json");
        std::fs::write(&worse, "{oops").unwrap();
        assert!(RuleSet::load_user_settings(&worse).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
