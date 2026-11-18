//! 内容匹配器四族（task-16 S2，设计稿 §三；预编译=parse 一次、匹配多次——
//! RuleSet::compile 已把规则串解析为结构体，此处只做匹配语义）。
//!
//! v1 依赖纪律：不引 glob crate——路径通配自写 `*`/`**` 两通配段语义（够用
//! 即止，glob crate 报批候 S4 工作区边界一并裁）。

/// 内容级匹配：rule_content=None（整工具级）→ 恒命中；Some→按工具族语义。
pub(crate) fn content_matches(rule_content: Option<&str>, actual: Option<&str>) -> bool {
    match rule_content {
        None => true, // 整工具级：任意调用命中
        Some(pattern) => {
            let Some(actual) = actual else {
                return false; // 规则带内容限定，调用无内容=不命中
            };
            if let Some(pattern_host) = pattern.strip_prefix("domain:") {
                return domain_matches(pattern_host, actual);
            }
            if pattern.contains('/') || pattern.contains('~') {
                return path_matches(pattern, actual);
            }
            command_matches(pattern, actual)
        }
    }
}

/// Bash 族：命令前缀 `cmd:*`（token 级前缀：pattern 去尾 `:*` 后必须与命令
/// 首 token 序精确匹配）或整命令精确/通配匹配。
fn command_matches(pattern: &str, command: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(":*") {
        let prefix = prefix.trim();
        let command = command.trim();
        if prefix.is_empty() {
            return true;
        }
        // token 级前缀：按空白切分的整词前缀（防 `cargo:*` 命中 `cargox`）
        let pattern_tokens: Vec<&str> = prefix.split_whitespace().collect();
        let command_tokens: Vec<&str> = command.split_whitespace().collect();
        pattern_tokens.len() <= command_tokens.len()
            && pattern_tokens.iter().zip(&command_tokens).all(|(p, c)| p == c)
    } else {
        pattern.trim() == command.trim()
    }
}

/// 路径族：`*`（单段内任意）与 `**`（跨段任意）自写匹配；`~` 前缀按字面
/// （调用方传入已展开路径时用 `**` 通配，不做 home 猜测——诚实不猜路径）。
fn path_matches(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let val: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segments_match(&pat, &val)
}

/// 段级递归匹配（`**` 吞零或多段；`*` 吞单段内任意）。
fn segments_match(pat: &[&str], val: &[&str]) -> bool {
    match (pat.first(), val.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            // ** 吞 0..=n 段
            (0..=val.len()).any(|skip| segments_match(&pat[1..], &val[skip..]))
        }
        (Some(p), Some(v)) if segment_eq(p, v) => segments_match(&pat[1..], &val[1..]),
        _ => false,
    }
}

/// 段相等：`*`=段内任意通配（吞单段内任意字符），其余字面相等。
fn segment_eq(pattern: &str, value: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == value,
        Some((prefix, suffix)) => {
            value.len() >= prefix.len() + suffix.len()
                && value.starts_with(prefix)
                && value.ends_with(suffix)
        }
    }
}

/// WebFetch 族：`domain:<host>` 精确 host（大小写不敏感；v1 不做通配子域——
/// 子域放行=显式逐条列，防规则写宽）。
fn domain_matches(pattern_host: &str, url: &str) -> bool {
    let pattern_host = pattern_host.to_ascii_lowercase();
    let url = url.to_ascii_lowercase();
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(&url)
        .split(['/', ':', '?'])
        .next()
        .unwrap_or_default();
    host == pattern_host
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // 测试断言惯例（同包既有测试同律）
mod tests {
    use super::*;
    use crate::rules::Behavior;

    /// 命令族：token 级前缀（:*）+精确+防假前缀。
    #[test]
    fn 命令族匹配() {
        use crate::rules::RuleSet;
        let set = RuleSet::compile(&crate::rules::PermissionsSection {
            allow: vec!["Bash(cargo:*)".into(), "Bash(git status)".into()],
            deny: vec![],
            ask: vec![],
        })
        .unwrap();
        assert_eq!(set.decision_for("bash", Some("cargo build --release")), Some(Behavior::Allow));
        assert_eq!(set.decision_for("bash", Some("git status")), Some(Behavior::Allow));
        // token 级前缀防假命中：cargox 不是 cargo
        assert_eq!(set.decision_for("bash", Some("cargox build")), None);
        // 精确规则不命中其他命令
        assert_eq!(set.decision_for("bash", Some("git status -s")), None);
    }

    /// 路径族：**/×/段语义+~ 字面+逃逸形态不误放。
    #[test]
    fn 路径族匹配() {
        assert!(path_matches("~/.ssh/**", "~/.ssh/id_rsa"));
        assert!(path_matches("/home/wu/agent/yunyou/**", "/home/wu/agent/yunyou/Cargo.toml"));
        assert!(path_matches("/home/wu/agent/**/Cargo.toml", "/home/wu/agent/yunyou/Cargo.toml"));
        assert!(!path_matches("/home/wu/agent/yunyou/**", "/home/wu/agent/yunyou-evil/x"));
        assert!(!path_matches("/home/wu/**", "/etc/passwd"));
        // * 单段：吞段内任意但不跨段
        assert!(path_matches("/data/*.txt", "/data/a.txt"));
        assert!(!path_matches("/data/*.txt", "/data/sub/a.txt"));
        // content_matches 分派：含 / 即走路径族
        assert!(content_matches(Some("/data/*.txt"), Some("/data/b.txt")));
    }

    /// WebFetch 族：domain 精确（大小写不敏感、不吃端口路径、不匹配子域）。
    #[test]
    fn webfetch族匹配() {
        assert!(content_matches(Some("domain:docs.rs"), Some("https://docs.rs/x")));
        assert!(content_matches(Some("domain:docs.rs"), Some("HTTPS://DOCS.RS")));
        assert!(!content_matches(Some("domain:docs.rs"), Some("https://evil.com/docs.rs")));
        assert!(!content_matches(Some("domain:docs.rs"), Some("https://docs.rs.evil.com")));
        assert!(!content_matches(Some("domain:docs.rs"), Some("https://notdocs.rs")));
    }

    /// 整工具级恒命中；内容级规则对无内容调用不命中。
    #[test]
    fn 层级边界() {
        assert!(content_matches(None, None));
        assert!(content_matches(None, Some("任意")));
        assert!(!content_matches(Some("cargo:*"), None));
    }
}
