//! 工具 canonical value → DetailKind 解析族（自 adapter.rs 按职责拆出，
//! 行数清偿批 task-22；纯搬移零行为变更）：失败原因提取+按工具名分流详情构造。

use crate::render::event::DetailKind;

/// task-37②：提取人话失败原因——**双形态兼容**：观察者事件侧 Err 是
/// `{code, message}` 对象（turn.rs canonical，无 hint）；会话侧是 render_error
/// 文本（`[错误 {code}] {message}`，hint 另起行）。取 message 段首行；形态
/// 不符（防御）返回 None 不追加。
pub(crate) fn error_reason(content: &serde_json::Value) -> Option<String> {
    if let Some(message) = content.get("message").and_then(|value| value.as_str()) {
        let first = message.lines().next().unwrap_or(message).trim();
        return (!first.is_empty()).then(|| first.to_string());
    }
    let text = content.as_str()?;
    let first = text.lines().next()?.strip_prefix("[错误 ")?;
    let message = first.split_once("] ")?.1.trim();
    (!message.is_empty()).then(|| message.to_string())
}

/// canonical value → 详情块（按工具名分流；失败卡不展开——原因已在完成卡）。
/// tool_render 注册表的委托入口见 adapter.rs 的 re-export（T1-a 立缝）。
pub(crate) fn parse_detail(name: &str, content: &serde_json::Value, is_error: bool) -> Option<DetailKind> {
    if is_error {
        return None; // 失败原因（退出码/stderr 首行）已并入 summary
    }
    // 8KB 超限降级：content 非 object（String 提示）→ Plain 单行，不做工具级解析
    if !content.is_object() {
        return Some(DetailKind::Plain {
            lines: vec![content.as_str().unwrap_or("（详情不可用）").to_string()],
        });
    }
    match name {
        // bash：{exit_code, output}——output 已含 [stderr] 段（工具侧拼装）
        "bash" => Some(DetailKind::Bash {
            // task-66 P1-3：None=未知退出码不伪造 -1（-1 非真实 shell 退出码）
            exit_code: content["exit_code"].as_i64().map(|v| v as i32),
            output: content["output"].as_str().unwrap_or_default().to_string(),
        }),
        // read：{content: "   N\t行\n"}——行号文本解析回（起始行号, 内容行）
        "read" => content["content"].as_str().and_then(parse_numbered),
        // glob/grep：{matches: [...], count: N}
        "glob" | "grep" => Some(DetailKind::Plain {
            lines: content["matches"]
                .as_array()
                .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
        }),
        // edit：{"old":…,"new":…}（task-55 阶段 A：形状约定先行，阶段 B tools
        // 侧上抛真数据）——旧文整段 removed、新文整段 added、单 hunk（edit 语义
        // =一次原位替换，无多 hunk 折叠需求；上下文折叠随真数据端到端再评估）。
        // 形状不符（无 old/new 键）落 catch-all Plain 防御，不编造。
        "edit" => {
            // 批 2-T2a：tools 侧已上抛真 hunk（structuredPatch）则直通——
            // 形状 [{context, removed, added}]；缺键（旧 canonical/跨版本会话
            // 回放）退旧文整段伪 diff，零回归。
            if let Some(hunks) = content["structuredPatch"].as_array() {
                if let Some(detail) = diff_from_patch(hunks, false) {
                    return Some(detail);
                }
            }
            let old = content["old"].as_str();
            let new = content["new"].as_str();
            match (old, new) {
                (Some(o), Some(n)) => Some(diff_detail(o, n)),
                _ => Some(DetailKind::Plain {
                    lines: content
                        .as_object()
                        .map(|map| {
                            map.iter()
                                .map(|(k, v)| format!("{k}: {v}"))
                                .collect()
                        })
                        .unwrap_or_default(),
                }),
            }
        }
        // apply_patch（批 2-T2a）：structuredPatch=[{path, hunks:[…]}] 多文件
        // 真数据；缺键退 catch-all Plain 防御。
        "apply_patch" => content["structuredPatch"]
            .as_array()
            .and_then(|files| {
                let mut added = 0usize;
                let mut removed = 0usize;
                let mut hunks = Vec::new();
                for file in files {
                    let path = file["path"].as_str().map(str::to_string);
                    let file_hunks = file["hunks"].as_array()?;
                    for h in file_hunks {
                        let hunk = hunk_from(h, path.clone())?;
                        added += hunk.added.len();
                        removed += hunk.removed.len();
                        hunks.push(hunk);
                    }
                }
                (!hunks.is_empty()).then(|| DetailKind::Diff { added, removed, hunks })
            })
            .or_else(|| {
                content.as_object().map(|map| DetailKind::Plain {
                    lines: map.iter().map(|(k, v)| format!("{k}: {v}")).collect(),
                })
            }),
        // 其余（write 等）：canonical value 顶层键值摘为 Plain 行
        _ => Some(DetailKind::Plain {            lines: content
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(key, value)| match value.as_i64() {
                            Some(n) => format!("{key}: {n}"),
                            None => format!("{key}: {}", value.as_str().unwrap_or("?")),
                        })
                        .collect()
                })
                .unwrap_or_else(|| vec![content.to_string()]),
        }),
    }
}

/// structuredPatch 单 hunk value → DiffHunk（批 2-T2a；形状不符返回 None
/// 防御——调用方退化路径兜底，不编造）。
fn hunk_from(value: &serde_json::Value, path: Option<String>) -> Option<crate::render::event::DiffHunk> {
    let ctx = value["context"].as_u64()? as usize;
    let removed: Vec<String> = value["removed"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let added: Vec<String> = value["added"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    Some(crate::render::event::DiffHunk {
        path,
        old_start: value["old_start"].as_u64().map(|v| v as usize),
        context_lines: ctx,
        removed,
        added,
    })
}

/// structuredPatch 平铺数组（edit 单文件形态）→ Diff 详情（None=形状不符
/// 退调用方旧路径）。
fn diff_from_patch(hunks: &[serde_json::Value], with_path: bool) -> Option<DetailKind> {
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut out = Vec::new();
    for h in hunks {
        let path = if with_path { h["path"].as_str().map(str::to_string) } else { None };
        let hunk = hunk_from(h, path)?;
        added += hunk.added.len();
        removed += hunk.removed.len();
        out.push(hunk);
    }
    out.is_empty().then(|| DetailKind::Diff { added, removed, hunks: out })
}

/// 解析 read 工具的行号文本（`{:>4}\t行`）→ Read 详情（起始行号 + 内容行）。
fn parse_numbered(text: &str) -> Option<DetailKind> {
    let mut from = 1usize;
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let (no, content) = match raw.split_once('\t') {
            Some(pair) => pair,
            None => (raw, raw),
        };
        if index == 0 {
            from = no.trim().parse().unwrap_or(1);
        }
        lines.push(content.to_string());
    }
    (!lines.is_empty()).then(|| DetailKind::Read { path: String::new(), lines, from })
}

/// edit diff 详情构造（task-55 阶段 A）：旧文整段 removed、新文整段 added、
/// 单 hunk——形状与 [`crate::render::event::DetailKind::Diff`] 消费臂（render_diff 的
/// +/− 着色与折叠）对齐；行拆分按 \n（空尾行由 lines() 语义自然丢弃）。
fn diff_detail(old: &str, new: &str) -> DetailKind {
    let removed: Vec<String> = old.lines().map(str::to_string).collect();
    let added: Vec<String> = new.lines().map(str::to_string).collect();
    DetailKind::Diff {
        added: added.len(),
        removed: removed.len(),
        hunks: vec![crate::render::event::DiffHunk {
                path: None,
                old_start: None,
            context_lines: 0,
            removed,
            added,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    /// task-55 阶段 A：edit canonical old/new → Diff hunk（形状约定锁定）。
    #[test]
    fn edit_diff_分支_形状约定() {
        let content = serde_json::json!({
            "path": "README.md",
            "old": "# YunYou Agent\n旧说明",
            "new": "# 云悠 YunYou\n新说明"
        });
        let detail = parse_detail("edit", &content, false).expect("edit 应产 Diff 详情");
        match detail {
            DetailKind::Diff { added, removed, hunks } => {
                assert_eq!((added, removed), (2, 2), "增删行计数：{hunks:?}");
                assert_eq!(hunks.len(), 1, "单 hunk（edit=一次原位替换）");
                assert_eq!(hunks[0].removed, vec!["# YunYou Agent", "旧说明"]);
                assert_eq!(hunks[0].added, vec!["# 云悠 YunYou", "新说明"]);
            }
            other => panic!("应为 Diff 详情：{other:?}"),
        }
    }

    /// task-55 阶段 A：形状不符（无 old/new 键）落 Plain 防御，不编造 diff。
    #[test]
    fn edit_形状不符_落_plain防御() {
        let content = serde_json::json!({ "path": "x", "bytes": 12 });
        let detail = parse_detail("edit", &content, false).expect("应产详情");
        match detail {
            DetailKind::Plain { lines } => {
                assert!(lines.iter().any(|l| l.contains("path")), "键值摘要有序保留：{lines:?}");
            }
            other => panic!("形状不符应落 Plain：{other:?}"),
        }
    }
}
