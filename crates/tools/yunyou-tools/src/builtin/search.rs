//! 检索类内置工具：glob（文件名匹配）/ grep（正则搜索，跳过二进制）。
//! 相对路径/模式锚定**工作区根**（task-52 与 fs 三件同款全族单义：不随
//! bash cd 漂移，见 builtin/cwd.rs resolve_workspace 同款语义）；结果经
//! canonicalize+starts_with 运行时闸过滤（symlink 越界词法盲视的运行时补强）。

use crate::builtin::cwd::CwdService;
use crate::builtin::HINT_PATH_OUTSIDE_ROOT;
use crate::contract::{PresentationMeta, ToolDefinition, ToolExec, ToolOutcome, render_outcome};
use std::path::Path;
use std::sync::Arc;
use yunyou_util::BoxFut;

/// glob 匹配数上限。
const MAX_GLOB_RESULTS: usize = 200;
/// grep 搜索的文件数上限。
const MAX_GREP_FILES: usize = 500;
/// grep 匹配输出上限。
const MAX_GREP_MATCHES: usize = 200;

/// glob 工具：当前目录视图内的文件名模式匹配。
pub struct GlobTool {
    pub cwd: Arc<CwdService>,
}

impl ToolDefinition for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "按 glob 模式（如 src/**/*.rs）列出文件，返回相对工作区根的路径列表；模式基于工作区根解析（git 项目根；非 git 目录=启动目录；可 [workspace] root 配置指定），不随 bash cd 漂移。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"pattern": {"type": "string", "description": "glob 模式，相对工作区根"}},
            "required": ["pattern"]
        })
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "matches": {"type": "array", "items": {"type": "string"}, "description": "匹配文件（相对路径）"},
                "count": {"type": "integer", "description": "匹配数"}
            },
            "required": ["matches", "count"]
        }))
    }

    fn presentation_meta(&self) -> Option<PresentationMeta> {
        Some(PresentationMeta::new("查找文件", "按模式列出文件", "search"))
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd = self.cwd.clone();
        Box::pin(async move {
            let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 pattern 参数");
            };
            if pattern.contains("..") {
                return ToolOutcome::err_h("tool/path-outside-root", "模式不得包含 ..", HINT_PATH_OUTSIDE_ROOT);
            }
            // 相对模式锚工作区根（task-52 全族单义）；展示仍相对根（根恒定路径稳定）
            let full_pattern = cwd.workspace_root().join(pattern.trim_start_matches('/'));
            let canonical_root = std::fs::canonicalize(cwd.workspace_root())
                .unwrap_or_else(|_| cwd.workspace_root().to_path_buf());
            let matches_iter = match glob::glob(&full_pattern.to_string_lossy()) {
                Ok(paths) => paths
                    .filter_map(|entry| entry.ok())
                    .filter(|path| path.is_file())
                    // 运行时闸：glob 展开经 symlink 可达根外——canonicalize 后
                    // 逐结果校验 starts_with(根)，越界结果静默剔除（防内容外泄）
                    .filter(|path| {
                        std::fs::canonicalize(path)
                            .map(|c| c.starts_with(&canonical_root))
                            .unwrap_or(false)
                    })
                    .map(|path| {
                        path.strip_prefix(cwd.root())
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .into_owned()
                    }),
                Err(err) => return ToolOutcome::err("tool/glob", format!("模式非法: {err}")),
            };
            // task-72：截断知情标注——+1 探测（第 201 个命中即溢出，walk 止步
            // 不全量计数；诚实呈现「超过上限」不伪造总数）。
            let mut matches: Vec<String> = Vec::with_capacity(MAX_GLOB_RESULTS);
            let mut truncated = false;
            for name in matches_iter {
                if matches.len() < MAX_GLOB_RESULTS {
                    matches.push(name);
                } else {
                    truncated = true;
                    break;
                }
            }
            let mut out = serde_json::json!({"matches": matches, "count": matches.len(), "truncated": truncated});
            if truncated {
                out["summary"] = serde_json::json!(format!(
                    "已截断：显示前 {} 个，匹配数超过上限——请收窄 pattern（限定子目录或扩展名）",
                    MAX_GLOB_RESULTS
                ));
            }
            ToolOutcome::ok(out)
        })
    }
}

/// grep 工具：正则搜索（递归文本文件，跳过二进制）。
pub struct GrepTool {
    pub cwd: Arc<CwdService>,
}

impl ToolDefinition for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "用正则递归搜索文本文件，输出 文件:行号:内容 列表；二进制文件跳过；默认搜索工作区根，subdir 相对工作区根。不随 bash cd 漂移。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "正则表达式"},
                "subdir": {"type": "string", "description": "可选：限定搜索的子目录（相对工作区根）"}
            },
            "required": ["pattern"]
        })
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "matches": {"type": "array", "items": {"type": "string"}, "description": "文件:行号:内容 列表"},
                "count": {"type": "integer", "description": "匹配数"}
            },
            "required": ["matches", "count"]
        }))
    }

    fn presentation_meta(&self) -> Option<PresentationMeta> {
        Some(PresentationMeta::new("搜索内容", "正则搜索文件内容。结果上限文件 500 个/匹配 200 条，截断时返回 truncated=true 与收窄建议。", "search-text"))
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    /// per-tool render 示范（契约⑥）：匹配行逐行平铺——比 JSON 更可读；
    /// canonical 值不变（模型面无损）；空匹配输出「（无匹配）」提示而非空串
    ///（评审 P2-4：空串会让零匹配在会话/日志里隐形）；错误/无 matches 键回落
    /// 统一渲染。
    fn render(&self, outcome: &ToolOutcome) -> String {
        match outcome {
            ToolOutcome::Ok(value) => match value.get("matches").and_then(|m| m.as_array()) {
                Some(matches) if matches.is_empty() => "（无匹配）".into(),
                Some(matches) => matches
                    .iter()
                    .filter_map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => render_outcome(outcome),
            },
            ToolOutcome::Err(_) => render_outcome(outcome),
        }
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd = self.cwd.clone();
        Box::pin(async move {
            let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 pattern 参数");
            };
            let regex = match regex::Regex::new(pattern) {
                Ok(regex) => regex,
                Err(err) => return ToolOutcome::err("tool/regex", format!("正则非法: {err}")),
            };
            let search_root = match args.get("subdir").and_then(|v| v.as_str()) {
                Some(subdir) if !subdir.is_empty() => {
                    let Ok(dir) = cwd.resolve_workspace(subdir) else {
                        return ToolOutcome::err_h(
                            "tool/path-outside-root",
                            format!("路径越界: {subdir}"),
                            HINT_PATH_OUTSIDE_ROOT,
                        );
                    };
                    dir
                }
                _ => cwd.workspace_root().to_path_buf(),
            };
            let canonical_root = std::fs::canonicalize(cwd.workspace_root())
                .unwrap_or_else(|_| cwd.workspace_root().to_path_buf());
            let mut files = Vec::new();
            collect_text_files(&search_root, &canonical_root, &mut files);
            // task-72：截断知情（文件数维度）——truncate 前计数，溢出即标注。
            let mut files_truncated = false;
            if files.len() > MAX_GREP_FILES {
                files.truncate(MAX_GREP_FILES);
                files_truncated = true;
            }
            let mut matches = Vec::new();
            let mut matches_truncated = false;
            'outer: for file in files {
                let Ok(bytes) = std::fs::read(&file) else { continue };
                if bytes.contains(&0) {
                    continue; // 二进制跳过
                }
                let Ok(text) = String::from_utf8(bytes) else { continue };
                let relative = file.strip_prefix(cwd.root()).unwrap_or(&file).to_string_lossy().into_owned();
                for (index, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        matches.push(format!("{relative}:{}:{}", index + 1, line.trim()));
                        if matches.len() >= MAX_GREP_MATCHES {
                            matches_truncated = true;
                            break 'outer;
                        }
                    }
                }
            }
            let truncated = files_truncated || matches_truncated;
            let mut out = serde_json::json!({"matches": matches, "count": matches.len(), "truncated": truncated});
            if truncated {
                out["summary"] = serde_json::json!(format!(
                    "已截断：上限文件 {} 个/匹配 {} 条（{}）——请收窄 pattern 或限定 subdir",
                    MAX_GREP_FILES,
                    MAX_GREP_MATCHES,
                    if files_truncated { "文件数超限" } else { "匹配数超限" }
                ));
            }
            ToolOutcome::ok(out)
        })
    }
}

/// 递归收集文本文件路径（跳过隐藏目录与常见产物目录；同步遍历——文件规模受
/// MAX_GREP_FILES 约束）。每个文件过 canonicalize+starts_with 运行时闸：
/// 遍历会跟进 symlink 目录，根外文件（内容外泄面）在此剔除。
fn collect_text_files(dir: &Path, canonical_root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            collect_text_files(&path, canonical_root, out);
        } else if out.len() < MAX_GREP_FILES {
            let inside = std::fs::canonicalize(&path)
                .map(|c| c.starts_with(canonical_root))
                .unwrap_or(false);
            if inside {
                out.push(path);
            }
        }
    }
}

use std::path::PathBuf;

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
