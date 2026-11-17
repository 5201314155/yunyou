//! UI 注册缝条目 schema（契约 v1 冻结，插件化方案 §4）。
//!
//! manifest `ui: []` 数组的条目类型（S8 冻结，方案 §4.2「一期只在 manifest 留 ui
//! 数组位并冻结条目 schema（kind=settings/command/page 预留 page），避免二期破格式」）：
//! - `settings`（设置表单段）/`command`（命令入口）：一期 schema 合法；
//! - `page`（页面级贡献）：二期 App 面预留**空变体**——serde 对 struct variant 默认
//!   忽略未声明字段，二期补字段时旧宿主可照常解析（向前兼容证据见测试）。
//!
//! 条目 schema 即契约 v1 的一部分（方案 §2.5）：破坏性变更（改字段/删 kind）必须
//! 发 v2 并保留 v1 兼容期。宿主承载面：`settings` 由 settings 投影服务渲染（一期
//! CLI 启动展示表单描述，零界面代码，方案 §4.1）；`command` 的 REPL 命令表二期接。

use serde::{Deserialize, Serialize};

/// UI 注册缝条目：`kind` 作 serde tag 的标签枚举（方案 §4.1 官方形态对照：
/// `ui: [{ kind: "settings", section: "tools" }]`、`ui: [{ kind: "command", name: "plugins" }]`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum UiEntry {
    /// 设置表单段：指向自身 `config_schema()` 生成 schema 的一个段，
    /// 宿主经 settings 投影服务把 schema 投成表单描述（契约④，方案 §3-④）。
    Settings {
        /// 表单段名（与配置段对齐，如 "llm"）；不得为空。
        section: String,
    },
    /// 入口/命令：REPL 命令表条目。一期 schema 冻结、宿主命令表二期接（方案 §4.1
    /// 一期裁定「/plugins 命令属诊断面可后置」；声明面先冻结防二期破格式）。
    Command {
        /// 命令名（不带斜杠，如 "plugins"）；不得为空。
        name: String,
    },
    /// 页面级贡献（二期 App 面预留，方案 §4.2）：**空变体**，一期宿主校验拒装。
    Page {},
}

impl UiEntry {
    /// 条目 kind 字符串（诊断与日志用）。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Settings { .. } => "settings",
            Self::Command { .. } => "command",
            Self::Page { .. } => "page",
        }
    }
}

/// ui 数组装载前校验（由 [`crate::manifest::validate_manifest`] 调用）。
///
/// 一期宿主承载面=「设置表单段」（方案 §4.1 一期裁定）：`page` 无承载面即报违例
/// （拒装）；`settings.section`/`command.name` 非空校验。返回中文违例清单（空=通过）。
pub(crate) fn validate_ui_entries(entries: &[UiEntry]) -> Vec<String> {
    let mut issues = Vec::new();
    for entry in entries {
        match entry {
            UiEntry::Settings { section } if section.is_empty() => issues.push(
                "ui 条目 settings.section 不得为空（应指向自身 config_schema 的段名）".into(),
            ),
            UiEntry::Command { name } if name.is_empty() => issues.push(
                "ui 条目 command.name 不得为空（REPL 命令名，不含斜杠）".into(),
            ),
            UiEntry::Page { .. } => issues.push(
                "ui 条目 kind=page 属二期 App 面预留（方案 §4.2）：一期宿主无承载面，拒装".into(),
            ),
            _ => {}
        }
    }
    issues
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试断言惯用 unwrap/expect；mod 级单点豁免（S-ISO-1 硬门口径，防逐处散落）
mod tests {
    use super::*;

    /// settings/command 条目的 tag 序列化形态（官方 manifest 形态对照，方案 §4.1）。
    #[test]
    fn settings与command条目tag形态() {
        let settings: UiEntry =
            serde_json::from_str(r#"{"kind": "settings", "section": "llm"}"#).expect("应可解析");
        assert_eq!(
            settings,
            UiEntry::Settings { section: "llm".into() },
            "kind=tag 应映射到 Settings 变体"
        );
        let command: UiEntry =
            serde_json::from_str(r#"{"kind": "command", "name": "plugins"}"#).expect("应可解析");
        assert_eq!(command, UiEntry::Command { name: "plugins".into() });
        assert_eq!(
            serde_json::to_value(&UiEntry::Command { name: "x".into() }).expect("应可序列化"),
            serde_json::json!({"kind": "command", "name": "x"}),
            "tag 键名与小写命名应冻结"
        );
    }

    /// page 预留空变体：一期解析接受（含未知字段——serde 对 struct variant 默认忽略
    /// 未声明字段），为二期加字段留出向前兼容。
    #[test]
    fn page预留变体向前兼容() {
        let page: UiEntry = serde_json::from_str(r#"{"kind": "page"}"#).expect("空 page 应可解析");
        let with_extra: UiEntry =
            serde_json::from_str(r#"{"kind": "page", "path": "future"}"#)
                .expect("带未来字段的 page 应可解析（未知字段忽略=向前兼容）");
        assert_eq!(page, UiEntry::Page {});
        assert_eq!(with_extra, UiEntry::Page {});
        assert_eq!(page.kind(), "page");
    }

    /// 未知 kind 拒绝（枚举封闭：契约外条目装载前即报，不静默吞）。
    #[test]
    fn 未知kind拒绝() {
        let result: Result<UiEntry, _> = serde_json::from_str(r#"{"kind": "ghost"}"#);
        assert!(result.is_err(), "未知 kind 必须报错（契约 v1 封闭集）");
    }

    /// 装载前校验：page 拒装、空 section/name 报违例、合法清单零违例。
    #[test]
    fn 装载前校验正反用例() {
        let legal = vec![
            UiEntry::Settings { section: "llm".into() },
            UiEntry::Command { name: "plugins".into() },
        ];
        assert!(validate_ui_entries(&legal).is_empty(), "合法清单应零违例");
        let bad = vec![
            UiEntry::Settings { section: String::new() },
            UiEntry::Command { name: String::new() },
            UiEntry::Page {},
        ];
        let issues = validate_ui_entries(&bad);
        assert_eq!(issues.len(), 3, "三个违例应逐条报出：{issues:?}");
        assert!(issues[0].contains("section 不得为空"));
        assert!(issues[1].contains("name 不得为空"));
        assert!(issues[2].contains("一期宿主无承载面"), "page 拒装理由应点名承载面");
    }
}
