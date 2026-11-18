//! 适配层（task-10 B 线）：A 线 agent-loop 事件 → 渲染视图（event::TurnEvent）。
//!
//! 职责：事件形状转换 + 工具 canonical value → DetailKind 解析（bash/read/
//! glob/grep/其余）+ 树字段透传（一期 None 平铺）。纯函数无 I/O，单测即契约；
//! DetailKind 的数据缺口（diff hunks）等工具侧产出后在此接入。

// canonical→DetailKind 解析族拆至子模块（行数清偿批 task-22；纯搬移零行为
// 变更，mod 声明在本文件不动 render/mod.rs）。
mod parse;

/// tool_render 契约层的委托入口（T1-a 立缝，语义同 parse_detail/error_reason）。
pub(crate) use parse::{error_reason as parse_error_reason, parse_detail as parse_detail_for};
use crate::render::event::TurnEvent as RenderEvent;

/// 云织 ToolResult → 渲染完成事件：完成卡（✓/✗ + 人话摘要 + 退出码）+ 详情块解析。
///
/// **人话摘要（任务 30-③）**：`args_summary` 来自
/// 观察者单槽记录的 ToolCall 参数摘要（观察者接线，A 线零改动）；按工具名分流
/// 生成「运行/读/改/搜」动词句（[`humanize`]，任务30 二期③起拆独立模块；
/// task-37③ bash 动词统一中文）。
///
/// **退出码单点（任务30 二期③）**：humanize 以 `has_exit` 声明句内已含退出码段
/// （bash 的「· 退出码 N」），此处只对未声明者追加「· 退出码 N」——两处各补一次
/// 曾撞车成「退出码 0 · 退出码 0」（业主实测），声明制从生成点根除重复。
///
/// **失败分型（task-37①，lead 批复：不动 Outcome 枚举，渲染侧翻转）**：bash
/// 非零退出对模型是**数据**（exit_code/stderr 是模型要反应的输入，bash.rs 语义
/// 锁 exit 3→Ok），对用户是**失败**——卡面 ✗/错误色按用户视角呈现；此处仅翻转
/// RenderEvent 的 ok 位，`is_error`/会话语义/模型可见面零改动，详情层 stderr
/// 照常展开（parse_detail 的 is_error 参数不受影响）。
///
/// **失败原因（task-37②）**：`is_error` 时 content=`render_error` 文本
/// （`[错误 {code}] {message}`，hint 可选）——摘 message 段并入摘要，卡面即见
/// 人话原因（原「✗ read · 读 x.txt」无原因）；hint 不上卡（太长），全文在
/// 会话 JSONL。
///
/// 详情数据源 = A 线 `content`（工具 canonical value 原样，上限 8KB，超限降级
/// 为 String 提示）；解析失败一律退化为 Plain 行，不阻塞渲染（适配层自担）。
pub(crate) fn tool_result_event(
    name: &str,
    summary: &str,
    content: &serde_json::Value,
    exit_code: Option<i64>,
    is_error: bool,
    args_summary: Option<&str>,
) -> RenderEvent {
    // T1-a 立缝：摘要/错误/详情统一走 tool_render 注册表分发（委托既有
    // humanize/parse 实现，行为零变更；T1-b 起各工具拆独立渲染器实现）。
    let renderer = crate::render::tool_render::renderer(name);
    let (mut summary, has_exit) = renderer.humanized_summary(name, args_summary, content, exit_code, summary);
    if !has_exit {
        if let Some(code) = exit_code {
            summary.push_str(&format!(" · 退出码 {code}"));
        }
    }
    // task-37②：失败原因并入摘要（人话；hint 不上卡，会话 JSONL 持有全文）
    if is_error {
        if let Some(reason) = renderer.error_summary(content) {
            summary.push_str(&format!(" · 失败：{reason}"));
        }
    }
    // 批 5（diff 三层·完成卡统计）：edit/apply_patch 成功时摘要尾追加
    // 「+增 −删」行计数（claude code DiffFileList 形态，〈挖掘〉§6.4；数据
    // 自 canonical structuredPatch 逐行统计，缺键/失败静默不加——不编造）。
    // 取舍注记：计数随 summary 单段呈 dim 灰阶，绿红分色候 summary 分段
    // 渲染改造（摘要入会话记录面，内嵌 SGR 会污染记录文本）。
    if !is_error && (name == "edit" || name == "apply_patch") {
        if let Some(stats) = patch_stats(content) {
            summary.push_str(&format!(" +{} −{}", stats.0, stats.1));
        }
    }
    // task-37①：bash 非零退出 → 卡面失败态（渲染侧翻转，is_error 语义不动）
    let bash_failed = name == "bash" && matches!(exit_code, Some(code) if code != 0);
    RenderEvent::ToolResult {
        call_id: String::new(), // 渲染层一期不消费 id（树缩进由观察者归集拼装）
        name: name.to_string(),
        ok: !is_error && !bash_failed,
        summary: summary.clone(),
        detail: renderer.result_detail(name, content, is_error),
        parent_tool_call_id: None,
    }
}

// error_reason/parse_detail/parse_numbered/diff_detail → adapter/parse.rs
// （task-22 搬移）。

/// structuredPatch 行计数（批 5）：双形兼容——apply_patch 文件包装
/// （[{path, hunks:[…]}]）与 edit 单文件直排（[{old_start, removed, added}]，
/// 元素无 path/hunks 键即 hunk 本体）；形状不符返回 None（旧 canonical/
/// 回放静默退位，不编造）。
fn patch_stats(content: &serde_json::Value) -> Option<(usize, usize)> {
    let items = content.get("structuredPatch")?.as_array()?;
    let is_file_wrapped = items.first().is_some_and(|f| f.get("hunks").is_some());
    let (mut added, mut removed) = (0usize, 0usize);
    let count = |h: &serde_json::Value, added: &mut usize, removed: &mut usize| -> Option<()> {
        *added += h.get("added")?.as_array()?.len();
        *removed += h.get("removed")?.as_array()?.len();
        Some(())
    };
    for item in items {
        if is_file_wrapped {
            for h in item.get("hunks")?.as_array()? {
                count(h, &mut added, &mut removed)?;
            }
        } else {
            count(item, &mut added, &mut removed)?;
        }
    }
    (added + removed > 0).then_some((added, removed))
}

    #[test]
    fn 完成卡加删计数_双形() {
        use serde_json::json;
        // edit 直排形
        let content = json!({"structuredPatch": [{"old_start": 1, "context": 0, "removed": ["a","b"], "added": ["c"]}]});
        assert_eq!(patch_stats(&content), Some((1, 2)));
        // apply_patch 包装形
        let content2 = json!({"structuredPatch": [{"path": "x", "hunks": [{"old_start": 1, "context": 0, "removed": ["a"], "added": ["b","c"]}], "hunks2": []}]});
        assert_eq!(patch_stats(&content2), Some((2, 1)));
        // 缺键静默退位
        assert_eq!(patch_stats(&json!({"old": "x"})), None);
        // 完成 summary 追加：edit 成功卡尾含 +N −R
        let ev = tool_result_event("edit", "", &content, None, false, Some("{\"path\":\"a.rs\"}"));
        if let crate::render::event::TurnEvent::ToolResult { summary, .. } = &ev {
            assert!(summary.ends_with("+1 −2"), "完成卡计数：{summary:?}");
        } else {
            panic!("应为 ToolResult");
        }
    }
#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::event::DetailKind; // task-66 P1-3：测试断言 Option 语义所需（lib 已无直接引用）
    use serde_json::json;
    
    /// bash：canonical value → Bash 详情（exit_code/output 直取）。
    #[test]
    fn 适配_bash() {
        let ev = tool_result_event(
            "bash",
            "echo hi",
            &json!({"exit_code": 0, "output": "hi\n"}),
            Some(0),
            false,
            None,
        );
        let RenderEvent::ToolResult { ok, summary, detail, .. } = ev else { panic!("应得 ToolResult") };
        assert!(ok && summary.contains("退出码 0"));
        assert!(matches!(detail, Some(DetailKind::Bash { exit_code: Some(0), ref output }) if output == "hi\n"));
    }

    /// read：行号文本 → Read 详情（起始行号 + 内容行；tab 分隔解析）。
    #[test]
    fn 适配_read_行号文本() {
        let ev = tool_result_event(
            "read",
            "a.rs",
            &json!({"content": "  10\tfn a() {\n  11\t}\n"}),
            None,
            false,
            None,
        );
        let RenderEvent::ToolResult { detail, .. } = ev else { panic!("应得 ToolResult") };
        match detail {
            Some(DetailKind::Read { lines, from, .. }) => {
                assert_eq!(from, 10, "起始行号取自首行行号");
                assert_eq!(lines, vec!["fn a() {", "}"]);
            }
            other => panic!("应得 Read 详情：{other:?}"),
        }
    }

    /// 失败卡不展开详情（原因已在完成卡——云核契约第 6 条）。
    #[test]
    fn 适配_失败不展开() {
        let ev = tool_result_event("bash", "boom", &json!({"exit_code": 1}), Some(1), true, None);
        let RenderEvent::ToolResult { ok, detail, summary, .. } = ev else { panic!("应得 ToolResult") };
        assert!(!ok && detail.is_none() && summary.contains("退出码 1"));
    }

    /// task-37①：bash 非零退出 → 卡面失败态（渲染侧翻转），但详情层 stderr
    /// 照常展开（is_error 语义不动——对模型仍是数据，对用户是失败）。
    #[test]
    fn bash_非零退出_卡面失败态详情仍展开() {
        let ev = tool_result_event(
            "bash",
            "旧",
            &json!({"exit_code": 2, "output": "ls: cannot access '/x': No such file\n[stderr] ls: 写错误\n"}),
            Some(2),
            false,
            Some(r#"{"cmd":"ls /x"}"#),
        );
        let RenderEvent::ToolResult { ok, summary, detail, .. } = ev else { panic!("应得 ToolResult") };
        assert!(!ok, "非零退出卡面应为失败态：{ok}");
        assert!(summary.contains("运行 ls /x · 退出码 2"), "{summary:?}");
        match detail {
            Some(DetailKind::Bash { exit_code, output }) => {
                assert_eq!(exit_code, Some(2), "详情层退出码原样（Option 语义 task-66 P1-3）");
                assert!(output.contains("[stderr]"), "详情层 stderr 照常展开：{output:?}");
            }
            other => panic!("应得 Bash 详情：{other:?}"),
        }
    }

    /// task-37②：工具 Err → 摘要并人话失败原因，hint 不上卡（全文在会话 JSONL）。
    /// 双形态：对象形态（观察者事件 canonical）+文本形态（render_error）。
    #[test]
    fn 失败原因并入摘要() {
        let ev = tool_result_event(
            "read",
            "旧",
            &json!({ "code": "tool/not-found", "message": "文件不存在: 沙盒/笔记.txt" }),
            None,
            true,
            Some(r#"{"path":"沙盒/笔记.txt"}"#),
        );
        let RenderEvent::ToolResult { ok, summary, .. } = ev else { panic!("应得 ToolResult") };
        assert!(!ok);
        assert_eq!(
            summary,
            "读 沙盒/笔记.txt · 失败：文件不存在: 沙盒/笔记.txt",
            "对象形态失败原因应并入摘要：{summary:?}"
        );
        assert!(!summary.contains("[错误"), "内部码形态不得上卡：{summary:?}");

        // 文本形态（render_error，带 hint）同规
        let ev2 = tool_result_event(
            "read",
            "旧",
            &json!("[错误 not_found] 没有那个文件或目录\n提示：确认相对路径基于当前工作目录"),
            None,
            true,
            Some(r#"{"path":"沙盒/笔记.txt"}"#),
        );
        let RenderEvent::ToolResult { summary, .. } = ev2 else { panic!("应得 ToolResult") };
        assert_eq!(
            summary,
            "读 沙盒/笔记.txt · 失败：没有那个文件或目录",
            "文本形态失败原因应并入摘要：{summary:?}"
        );
        assert!(!summary.contains("提示："), "hint 不上卡：{summary:?}");
    }

    /// 8KB 超限降级（content 为 String）：退化为 Plain 单行，不 panic。
    #[test]
    fn 适配_超限降级_string() {
        let ev = tool_result_event("bash", "big", &json!("输出超限，看会话 JSONL"), Some(0), false, None);
        let RenderEvent::ToolResult { detail, .. } = ev else { panic!("应得 ToolResult") };
        match detail {
            Some(DetailKind::Plain { lines }) => assert_eq!(lines.len(), 1),
            other => panic!("应退化为 Plain：{other:?}"),
        }
    }

    /// glob/grep：matches 数组 → Plain 行。
    #[test]
    fn 适配_search() {
        let ev = tool_result_event(
            "grep",
            "TODO",
            &json!({"matches": ["a.rs:1", "b.rs:9"], "count": 2}),
            None,
            false,
            None,
        );
        let RenderEvent::ToolResult { detail, .. } = ev else { panic!("应得 ToolResult") };
        match detail {
            Some(DetailKind::Plain { lines }) => assert_eq!(lines.len(), 2),
            other => panic!("应得 Plain：{other:?}"),
        }
    }

    /// 人话摘要集成（任务 30 二期③）：args JSON 解析出 cmd + 退出码单一形态锁死——
    /// bash 全链=「运行 <命令> · 退出码 <码>」（task-37③ 中文统一），外层不得再追加。
    #[test]
    fn 人话_bash_单一退出码形态() {
        let ev = tool_result_event(
            "bash",
            r#"{"exit_code": 0, "output": "hi\n"}"#,
            &json!({"exit_code": 0, "output": "hi\n"}),
            Some(0),
            false,
            Some(r#"{"cmd":"pwd; ls"}"#),
        );
        let RenderEvent::ToolResult { summary, .. } = ev else { panic!("应得 ToolResult") };
        assert_eq!(summary, "运行 pwd; ls · 退出码 0", "{summary:?}");
        assert_eq!(
            summary.matches("退出码").count(),
            1,
            "退出码信息只留一处：{summary:?}"
        );
        assert!(!summary.contains('{'), "原始 JSON 不得进摘要：{summary:?}");
    }

    /// 人话摘要：非 bash 工具退出码段由外层追加（has_exit=false 路径）；
    /// args_summary=None 全体回退 A 线 summary（不伪造，集成层契约）。
    #[test]
    fn 人话_外层追加与回退() {
        let read = tool_result_event(
            "read", "旧摘要", &json!({"content": "  1\ta\n  2\tb\n"}), None, false, Some(r#"{"path":"src/a.rs"}"#),
        );
        let RenderEvent::ToolResult { summary, .. } = read else { panic!() };
        // 任务30 三期①/B2：path 容错提取脱 JSON 化（原断言锁 JSON 原文形态已随行为升级）
        assert_eq!(summary, "读 src/a.rs · 2 行", "{summary:?}");

        // 未知工具：回退 A 线 summary（fail-safe）
        let other = tool_result_event("webfetch", "旧摘要文本", &json!({}), None, false, Some("url"));
        let RenderEvent::ToolResult { summary, .. } = other else { panic!() };
        assert_eq!(summary, "旧摘要文本", "{summary:?}");

        // 回退路径（fail-safe 另一半）：观察者单槽无记录（args_summary=None）——
        // 任何已知工具都不得编人话，原样 A 线 summary（防无中生有）
        let none_bash = tool_result_event("bash", "A线摘要", &json!({"exit_code": 0}), Some(0), false, None);
        let RenderEvent::ToolResult { summary, .. } = none_bash else { panic!() };
        assert!(summary.starts_with("A线摘要 · 退出码 0"), "None 摘要应回退 A 线：{summary:?}");
        assert!(!summary.contains("运行 "), "无参数摘要时不得伪造动词句：{summary:?}");

        let none_read = tool_result_event(
            "read",
            "A线摘要",
            &json!({"content": "  1\ta\n"}),
            None,
            false,
            None,
        );
        let RenderEvent::ToolResult { summary, .. } = none_read else { panic!() };
        assert_eq!(summary, "A线摘要", "read 无摘要同样回退：{summary:?}");
    }
}
