//! 文件类内置工具：read / write / edit（路径约束与相对路径解析统一走共享
//! 路径语义（task-50 rev3，业主定调）：相对路径一律锚定**工作区根**
//! （CwdService::workspace_root，三段链解析见 resolve_workspace_root），不随 bash cd 漂移；
//! 解析经 resolve_workspace 唯一化（canonicalize+starts_with 运行时防穿越闸）。
//! 结果回显绝对路径——写歪当场可见，模型按报错/回显自纠（诚实闭环）。

use crate::builtin::cwd::CwdService;
use crate::builtin::{HINT_PATH_OUTSIDE_ROOT, MAX_READ_BYTES};
use crate::contract::{PresentationMeta, ToolDefinition, ToolExec, ToolOutcome};
use std::sync::Arc;
use yunyou_util::BoxFut;

/// read 工具：读文本文件（带行号，大小上限防御）。
pub struct ReadTool {
    /// 工具域共享 CWD 服务：相对路径基于其当前值解析；root 约束在其内部恒守
    /// （cwd 恒在 root 内，越界 fail-loud）。
    pub cwd: Arc<CwdService>,
}

impl ToolDefinition for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "读取项目内文本文件，返回带行号的内容；相对路径基于工作区根解析（git 项目根；非 git 目录=启动目录；可 [workspace] root 配置指定），不随 bash cd 漂移；需要 bash 当前目录时先用 bash pwd 取绝对路径。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string", "description": "相对当前目录的文件路径"}},
            "required": ["path"]
        })
    }

    /// S4 后新工具必填（契约⑥）：canonical value 结构声明。
    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {"content": {"type": "string", "description": "带行号的文件内容"}},
            "required": ["content"]
        }))
    }

    /// 卡片呈现声明（静态模板；插值归渲染层）。
    fn presentation_meta(&self) -> Option<PresentationMeta> {
        Some(PresentationMeta::new("读取文件", "读取 path 的内容", "file"))
    }

    /// 读类工具可并发（官方显式选入语义；默认 false 需作者声明）。
    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd = self.cwd.clone();
        Box::pin(async move {
            let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 path 参数");
            };
            let Ok(full) = cwd.resolve_workspace(path) else {
                return ToolOutcome::err_h(
                    "tool/path-outside-root",
                    format!("路径越界: {path}"),
                    HINT_PATH_OUTSIDE_ROOT,
                );
            };
            let meta = match tokio::fs::metadata(&full).await {
                Ok(meta) => meta,
                Err(_) => {
                    return ToolOutcome::err(
                        "tool/not-found",
                        format!("文件不存在：{}（相对路径 {path}）", full.display()),
                    )
                }
            };
            if meta.len() > MAX_READ_BYTES {
                return ToolOutcome::err(
                    "tool/too-large",
                    format!("文件 {} 共 {} 字节超过读取上限 {MAX_READ_BYTES}", full.display(), meta.len()),
                );
            }
            match tokio::fs::read(&full).await {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => {
                        let numbered: String = text
                            .lines()
                            .enumerate()
                            .map(|(index, line)| format!("{:>4}\t{line}\n", index + 1))
                            .collect();
                        ToolOutcome::ok(serde_json::json!({"content": numbered}))
                    }
                    Err(_) => ToolOutcome::err("tool/binary", "非 UTF-8 文本，拒绝读取"),
                },
                Err(err) => ToolOutcome::err(
                    "tool/io",
                    format!("读取失败：{}（{err}）", full.display()),
                ),
            }
        })
    }
}

/// write 工具：写入（或覆盖）项目内文本文件。
pub struct WriteTool {
    pub cwd: Arc<CwdService>,
}

impl ToolDefinition for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "把文本写入项目内的文件（覆盖写）。父目录必须已存在——缺失即报错，不会自动创建（先建目录或用 bash mkdir 后重写）。相对路径基于工作区根解析（git 项目根；非 git 目录=启动目录；可 [workspace] root 配置指定），不随 bash cd 漂移；结果回显绝对路径。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        })
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "written_bytes": {"type": "integer", "description": "写入字节数"},
                "path": {"type": "string", "description": "落盘文件的绝对路径"},
                "summary": {"type": "string", "description": "人话结算：成功写入 X 字节到：<绝对路径>"}
            },
            "required": ["written_bytes", "path", "summary"]
        }))
    }

    fn presentation_meta(&self) -> Option<PresentationMeta> {
        Some(PresentationMeta::new("写入文件", "写入 path", "file-write"))
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd = self.cwd.clone();
        Box::pin(async move {
            let (Some(path), Some(content)) = (
                args.get("path").and_then(|v| v.as_str()),
                args.get("content").and_then(|v| v.as_str()),
            ) else {
                return ToolOutcome::err("tool/args", "缺少 path 或 content 参数");
            };
            let Ok(full) = cwd.resolve_workspace(path) else {
                return ToolOutcome::err_h(
                    "tool/path-outside-root",
                    format!("路径越界: {path}"),
                    HINT_PATH_OUTSIDE_ROOT,
                );
            };
            // ①缺父目录拒绝不建（业主定调「缺目录就该报错」）：静默
            // create_dir_all 会把「写错地方」伪装成「写成功」——模型按人话
            // 报错自纠（先建目录再写），诚实闭环。
            if let Some(parent) = full.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    return ToolOutcome::err_h(
                        "tool/not-found",
                        format!(
                            "目录不存在：{}（写入目标：{}；先创建目录或用 bash mkdir 后重写）",
                            parent.display(),
                            full.display()
                        ),
                        "先创建目录（bash mkdir -p <目录>）后重写",
                    );
                }
            }
            match crate::builtin::write_verify::write_verified(&full, content.as_bytes()).await {
                Ok(on_disk) => ToolOutcome::ok(serde_json::json!({
                    "written_bytes": on_disk,
                    "path": full.display().to_string(),
                    "summary": format!("成功写入 {} 字节到：{}", on_disk, full.display()),
                })),
                Err(err) => ToolOutcome::err(
                    "tool/io",
                    format!("写入失败：{}（{err}）", full.display()),
                ),
            }
        })
    }
}

/// edit 工具：唯一匹配替换（多匹配/零匹配报错，防误伤）。
pub struct EditTool {
    pub cwd: Arc<CwdService>,
}

impl ToolDefinition for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "把文件中唯一出现的 old 文本替换为 new；出现零次或多次都会报错。相对路径基于工作区根解析（git 项目根；非 git 目录=启动目录；可 [workspace] root 配置指定），不随 bash cd 漂移；结果回显绝对路径。成功 outcome 形状（task-55 阶段 B 加法演进）：{replaced, path, old, new}——old/new 供 CLI diff 详情卡渲染（adapter Diff 分支消费）；缺键的旧数据零影响（v1.1 同款加法）。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old": {"type": "string"},
                "new": {"type": "string"}
            },
            "required": ["path", "old", "new"]
        })
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "replaced": {"type": "integer", "description": "实际替换次数（恒为 1）"},
                "path": {"type": "string", "description": "被编辑文件的绝对路径"}
            },
            "required": ["replaced", "path"]
        }))
    }

    fn presentation_meta(&self) -> Option<PresentationMeta> {
        Some(PresentationMeta::new("编辑文件", "替换 path 中的唯一匹配", "file-edit"))
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd = self.cwd.clone();
        Box::pin(async move {
            let (Some(path), Some(old), Some(new)) = (
                args.get("path").and_then(|v| v.as_str()),
                args.get("old").and_then(|v| v.as_str()),
                args.get("new").and_then(|v| v.as_str()),
            ) else {
                return ToolOutcome::err("tool/args", "缺少 path/old/new 参数");
            };
            let Ok(full) = cwd.resolve_workspace(path) else {
                return ToolOutcome::err_h(
                    "tool/path-outside-root",
                    format!("路径越界: {path}"),
                    HINT_PATH_OUTSIDE_ROOT,
                );
            };
            let text = match tokio::fs::read_to_string(&full).await {
                Ok(text) => text,
                Err(err) => {
                    return ToolOutcome::err(
                        "tool/io",
                        format!("读取失败：{}（{err}）", full.display()),
                    )
                }
            };
            let count = text.matches(old).count();
            match count {
                0 => ToolOutcome::err("tool/no-match", "old 文本在文件中出现 0 次"),
                1 => {
                    let updated = text.replacen(old, new, 1);
                    match crate::builtin::write_verify::write_verified(&full, updated.as_bytes()).await {
                        Ok(_on_disk) => {
                            // 批 5（gutter 数据面）：hunk 行号=文件内 1-based——
                            // 定位 old 唯一匹配前的行数（文件偏移）+ 行内 diff 行号
                            let structured_patch: Vec<serde_json::Value> = {
                                let offset = text
                                    .find(old)
                                    .map(|pos| text[..pos].lines().count())
                                    .unwrap_or(0);
                                crate::builtin::line_diff::diff_hunks(old, new)
                                    .iter()
                                    .map(|h| {
                                        serde_json::json!({
                                            "old_start": offset + h.old_start,
                                            "context": h.context,
                                            "removed": h.removed,
                                            "added": h.added,
                                        })
                                    })
                                    .collect()
                            };
                            ToolOutcome::ok(serde_json::json!({
                            "replaced": 1,
                            "path": full.display().to_string(),
                            // task-55 阶段 B：diff 数据面上抛——旧文/新文随 outcome
                            // 进 canonical（adapter Diff 分支消费 → render_diff 首次
                            // 真数据）。加法演进（v1.1 同款）：旧消费者缺键零影响；
                            // 超大体量由 8KB content 上限降级路径兜底。
                            "old": old,
                            "new": new,
                            // 批 2-T2a（TUI 大改，lead 授权 2026-11-17）：真 hunk
                            // 上抛——行级 LCS diff（line_diff 纯函数），替代 cli
                            // 侧「旧整段删/新整段加」伪 diff。加法演进：旧消费者
                            // 缺键零影响；超限退 Fallback 由 line_diff 护栏承担。
                            "structuredPatch": structured_patch,
                            }))
                        }
                        Err(err) => ToolOutcome::err(
                            "tool/io",
                            format!("写回失败：{}（{err}）", full.display()),
                        ),
                    }
                }
                n => ToolOutcome::err("tool/ambiguous", format!("old 文本出现 {n} 次，拒绝替换")),
            }
        })
    }
}

#[cfg(test)]
#[path = "fs_tools_tests.rs"]
mod tests;
