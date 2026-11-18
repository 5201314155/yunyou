//! 工具结果人话摘要（任务30 一期③引入自 adapter 拆出，任务30 二期③双修，
//! 任务30 三期①容错提取）。
//!
//! 职责：把观察者单槽记录的 ToolCall 参数摘要（`arguments_summary`，即参数
//! JSON 原文**截断 120 字符**）与工具 canonical value 组装成「运行/读/改/搜」
//! 动词句（task-37③：bash 动词由英文「Ran」统一为中文「运行」，全中文 UI）。
//!
//! **容错提取（任务30 三期①，业主两度实锤）**：`arguments_summary` 恒在
//! 120 字符截断，长命令 JSON 必然断裂——serde 整体解析对截断文本必败，
//! 回退即把原始 JSON 甩上屏（业主截图 `Ran {"cmd":"cat -n /home/wu/…`）。
//! 故字段提取**不依赖完整 JSON**：字面锚定 `"字段":"` 取值到下一个未转义
//! 引号或串尾；值内转义做最小还原（`\"`→`"`、`\\`→`\`），`\n` 等控制转义
//! 保留字面——单行摘要形态不破，终端可读。截断容忍合理：展示层本就按
//! 48 列截断，摘要信息量足够即可，不追求无损。误匹配风险评估：锚定串含
//! 键后紧跟值的完整形态，命令文本恰含 `"cmd":"` 字样才会误锚，概率极低
//! 且 fail-safe（宁取值略偏不上屏原始 JSON）。
//!
//! **字段映射（任务30 三期 B2，lead 裁决全族扩展）**：bash→cmd、
//! read/edit/write→path、glob/grep→pattern；映射字段提取失败（截断在
//! 字段名之前等最坏位置）退**首字段值通法**兜底，再失败回退 args 原文
//! ——宁展示原文不猜测，渲染层不编造。
//!
//! **退出码单点负责（任务30 二期③）**：bash 句内嵌「· 退出码 N」并以返回值
//! `has_exit = true` 声明，外层（adapter::tool_result_event）对其不再追加
//! 「· 退出码 N」——两处各补一次曾撞车成「退出码 0 · 退出码 0」（业主实测）；
//! 声明制比事后字符串去重可靠：谁生成退出码段谁声明，新增分支默认由外层
//! 统一追加，不会静默重复。
//!
//! fail-safe：args_summary 缺失或未知工具一律回退 A 线 summary（防无中生有）。

use serde_json::Value;

/// 生成（人话摘要，是否已含退出码段）。`has_exit=true` 时外层不得再追加退出码。
pub(crate) fn humanize(
    name: &str,
    args_summary: Option<&str>,
    content: &Value,
    exit_code: Option<i64>,
    fallback: &str,
) -> (String, bool) {
    let Some(args) = args_summary.filter(|args| !args.is_empty()) else {
        return (fallback.to_string(), false);
    };
    // 各工具的目标字段（B2 映射表）；提取三层 fail-safe 见模块注释
    let field = match name {
        "bash" => "cmd",
        "read" | "edit" | "write" => "path",
        "glob" | "grep" => "pattern",
        _ => return (fallback.to_string(), false),
    };
    let subject = extract(args, field).unwrap_or_else(|| args.to_string());
    match name {
        "bash" => match exit_code {
            // task-37③：动词与退出码段统一中文。task-66 P1-3：None=未知退出码
            // 不伪造 -1（-1 非真实 shell 退出码）——无退出码段，语义留给详情层。
            Some(code) => (format!("运行 {subject} · 退出码 {code}"), true),
            None => (format!("运行 {subject}"), false),
        }
        "read" => match content["content"].as_str() {
            // 行数从 canonical content 取；8KB 降级（canonical 为 String）
            // 时拿不到——仍给「读 <path>」而非回退 A 线 summary（A 线
            // summary 源自 render_outcome 的 pretty JSON，多行糊完成卡，
            // 任务30 四期 D-B 业主实锤）
            Some(text) => (
                format!("读 {subject} · {} 行", text.lines().count().max(1)),
                false,
            ),
            None => (format!("读 {subject}"), false),
        },
        "edit" | "write" => match content["replaced"].as_u64() {
            Some(count) if count > 0 => (format!("改 {subject} · 替换 {count} 处"), false),
            _ => (format!("改 {subject}"), false),
        },
        "glob" | "grep" => match content["count"].as_u64() {
            Some(count) => (format!("搜 {subject} · {count} 处命中"), false),
            // 同上：降级 canonical 拿不到命中数，pattern 本身仍是有效信息
            None => (format!("搜 {subject}"), false),
        },
        _ => unreachable!("field 映射与动词分支同表维护，新工具先加映射再加分支"),
    }
}

/// 三层容错提取：映射字段 → 首字段值通法 → None（调用方回退原文）。
/// 批 4（T1-b）起 tool_render activity 钩子复用同源提取（单源不二写）。
pub(crate) fn extract(args: &str, field: &str) -> Option<String> {
    field_value(args, field).or_else(|| first_field(args))
}

/// 字面锚定提取指定字段的值：定位 `"field"` 键 → 容忍冒号前后空白 → 取值
/// 到下一未转义引号或串尾（机制论证见模块注释；截断容忍）。空白容忍是
/// D-B 的另一半真凶：LLM 生成的参数 JSON 常带空格（`"pattern": "*"`），
/// 旧锚定串 `"pattern":"` 对其必败 → 回退原文糊卡。
fn field_value(args: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let key_pos = args.find(&needle)?;
    let after_key = args[key_pos + needle.len()..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    take_value(after_colon.strip_prefix('"')?)
}

/// 首字段值通法：首个键的值（截断最坏位置兜底——目标字段名被截掉时仍能
/// 给出至少一个有效参数片段，比整段原始 JSON 可读）。空白容忍同上。
fn first_field(args: &str) -> Option<String> {
    let head = args.trim_start().strip_prefix('{')?.trim_start();
    let key_body = head.strip_prefix('"')?;
    let key_end = key_body.find('"')?;
    let after_key = key_body[key_end + 1..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    take_value(after_colon.strip_prefix('"')?)
}

/// 从值起点取到未转义引号或串尾；`\"`/`\\` 还原，其余转义序列保留字面
/// （`\n` 不展开为真换行——单行摘要形态不破，终端可见即懂）。
fn take_value(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break, // 未转义引号=值终
            '\\' => match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => break, // 串尾截断在转义符上：丢弃孤立反斜杠
            },
            c => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// bash：参数 JSON 解析出 cmd——「运行 <命令> · 退出码 <码>」单一退出码形态
    /// （task-37③ 中文统一后，撞车断言改为同段计数）。
    #[test]
    fn bash_完整json_解析cmd() {
        let (text, has_exit) = humanize(
            "bash",
            Some(r#"{"cmd":"pwd; ls"}"#),
            &json!({"exit_code": 0, "output": ""}),
            Some(0),
            "A线摘要",
        );
        assert_eq!(text, "运行 pwd; ls · 退出码 0", "命令应取自 cmd 字段：{text:?}");
        assert!(has_exit, "bash 句已含退出码段，外层不得追加");
        assert!(!text.contains('{'), "原始 JSON 不得进摘要：{text:?}");
        assert_eq!(
            text.matches("退出码").count(),
            1,
            "退出码段只留一处（撞车回归）：{text:?}"
        );
    }

    /// bash：截断 JSON（120 字符断裂、无闭合引号）→ 容错提取到串尾——
    /// 业主实锤回归：`运行 {"cmd":"cat -n /home/wu/…` 不再甩原始 JSON。
    #[test]
    fn bash_截断json_提取到串尾() {
        let (text, has_exit) = humanize(
            "bash",
            Some(r#"{"cmd":"cargo test -p yunyou && cargo test -p yunyou-c"#),
            &json!({"exit_code": 0, "output": ""}),
            Some(0),
            "A线摘要",
        );
        assert_eq!(
            text,
            r#"运行 cargo test -p yunyou && cargo test -p yunyou-c · 退出码 0"#,
            "截断处取到串尾，不回退原始 JSON：{text:?}"
        );
        assert!(has_exit, "提取形态仍内嵌退出码段，外层不得追加");
        assert!(!text.contains("{\"cmd\""), "JSON 键不得进摘要：{text:?}");
    }

    /// bash：非 JSON 纯文本 → 三层提取全败回退原文（宁原文不编造）。
    #[test]
    fn bash_非json_回退原文() {
        let (text, has_exit) =
            humanize("bash", Some("ls -la"), &json!({"exit_code": 0}), Some(0), "A线摘要");
        assert_eq!(text, "运行 ls -la · 退出码 0");
        assert!(has_exit);
    }

    /// bash：值内转义引号还原（`\"`→`"`），单行形态不破。
    #[test]
    fn bash_转义引号还原() {
        let (text, _) =
            humanize("bash", Some(r#"{"cmd":"echo \"hi\""}"#), &json!({}), Some(0), "x");
        assert_eq!(text, r#"运行 echo "hi" · 退出码 0"#, "转义引号应还原：{text:?}");
    }

    /// bash：无 exit_code=未知——不带退出码段（task-66 P1-3：-1 非真实
    /// shell 退出码，未知≠-1 不伪造）。
    #[test]
    fn bash_无退出码_不带退出码段() {
        let (text, has_exit) = humanize("bash", Some(r#"{"cmd":"ls"}"#), &json!({}), None, "x");
        assert_eq!(text, "运行 ls", "未知退出码不带退出码段：{text:?}");
        assert!(!has_exit, "未知退出码不声明");
    }

    /// read：完整 JSON 取 path（B2 全族扩展）——`读 <路径> · N 行`。
    #[test]
    fn read_完整json_取path() {
        let (text, has_exit) = humanize(
            "read",
            Some(r#"{"path":"src/a.rs"}"#),
            &json!({"content": "  1\ta\n  2\tb\n"}),
            None,
            "旧摘要",
        );
        assert_eq!(text, "读 src/a.rs · 2 行", "path 应脱 JSON 化：{text:?}");
        assert!(!has_exit);
    }

    /// read：截断在第二字段内（path 完整）→ 照常提取 path。
    #[test]
    fn read_截断json_提取path() {
        let (text, _) = humanize(
            "read",
            Some(r#"{"path":"crates/yunyou/src/fiber.rs","start_line":1"#),
            &json!({"content": "  1\tx\n"}),
            None,
            "旧",
        );
        assert_eq!(text, "读 crates/yunyou/src/fiber.rs · 1 行");
    }

    /// read：截断最坏位置（字段名被截）→ 映射与首字段全败回退原文（fail-safe）。
    #[test]
    fn read_截断最坏位置_回退原文() {
        let (text, _) = humanize(
            "read",
            Some(r#"{"pa"#),
            &json!({"content": "  1\tx\n"}),
            None,
            "旧",
        );
        assert_eq!(text, r#"读 {"pa · 1 行"#, "提取全败应回退原文：{text:?}");
    }

    /// glob/grep：pattern 映射——`搜 <模式> · N 处命中`（业主截图回归：
    /// `✓ glob · 搜 {"pattern":"*"}` 不再甩 JSON）。
    #[test]
    fn glob_截断json_提取pattern() {
        let (text, has_exit) = humanize(
            "glob",
            Some(r#"{"pattern":"*.rs","path":"crates""#),
            &json!({"matches": ["a.rs"], "count": 1}),
            None,
            "旧",
        );
        assert_eq!(text, "搜 *.rs · 1 处命中", "pattern 应脱 JSON 化：{text:?}");
        assert!(!has_exit);
    }

    /// edit：完整 JSON 取 path；非 JSON（纯文本）回退原文。
    #[test]
    fn edit_两态() {
        let (text, _) = humanize(
            "edit",
            Some(r#"{"path":"a.rs","old_string":"x","new_string":"y"}"#),
            &json!({"replaced": 3}),
            None,
            "旧",
        );
        assert_eq!(text, "改 a.rs · 替换 3 处");

        let (text, _) = humanize("edit", Some("README.md"), &json!({"replaced": 3}), None, "旧");
        assert_eq!(text, "改 README.md · 替换 3 处", "非 JSON 回退原文：{text:?}");
    }

    /// 带空格参数 JSON（LLM 常见输出形态 `"pattern": "*"`）：提取容忍
    /// 冒号前后空白（D-B 另一半真凶——旧锚定串对其必败回退原文）。
    #[test]
    fn 带空格参数_提取成功() {
        let (text, _) = humanize(
            "glob",
            Some("{\n  \"pattern\": \"*.rs\",\n  \"path\": \"crates\"\n}"),
            &json!({"matches": ["a.rs"], "count": 1}),
            None,
            "旧",
        );
        assert_eq!(text, "搜 *.rs · 1 处命中", "多行带空格参数应提取 pattern：{text:?}");
    }

    /// glob：8KB 降级 canonical（String）拿不到命中数 → 「搜 <pattern>」，
    /// 不回退 A 线 summary（其为 render_outcome pretty JSON，多行糊完成卡
    /// =业主截图 `{ "count": 200, …` 病灶，任务30 四期 D-B）。
    #[test]
    fn glob_降级canonical_不糊json() {
        let degraded = serde_json::json!("[输出超 8KB，已折叠]\n路径1\n…");
        let (text, has_exit) = humanize(
            "glob",
            Some(r#"{"pattern": "*.rs", "path": "crates"}"#),
            &degraded,
            None,
            r#"{ "count": 200, "matches": [ pretty糊屏文本"#,
        );
        assert_eq!(text, "搜 *.rs", "降级时仍给 pattern 不回退 JSON：{text:?}");
        assert!(!text.contains("count"), "结果 JSON 不得进摘要：{text:?}");
        assert!(!has_exit);
    }

    /// grep：非 JSON 回退原文；has_exit=false（退出码段由外层统一追加）。
    #[test]
    fn grep_非json_回退原文() {
        let (text, has_exit) =
            humanize("grep", Some("TODO"), &json!({"matches": [], "count": 7}), None, "旧");
        assert_eq!(text, "搜 TODO · 7 处命中");
        assert!(!has_exit);
    }

    /// 回退路径：args_summary 缺失/未知工具 → 原样 A 线 summary（不编人话）。
    #[test]
    fn 缺失与未知工具_回退_a线() {
        let (text, has_exit) = humanize("bash", None, &json!({"exit_code": 0}), Some(0), "A线摘要");
        assert_eq!(text, "A线摘要");
        assert!(!has_exit, "回退句未内嵌退出码，外层应追加");

        let (text, has_exit) = humanize("webfetch", Some("url"), &json!({}), None, "旧摘要文本");
        assert_eq!(text, "旧摘要文本");
        assert!(!has_exit);
    }
}
