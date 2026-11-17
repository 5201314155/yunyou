//! apply_patch 工具（task-22）：多文件/多区域批量补丁（与 edit 并存，分工见
//! description；经 builtin_tools 注册缝挂载，装配序列零改动）。
//!
//! # 三段流程（报告 §三：Codex 记账式 + DSH 预检轻量版）
//! ①解析（patch_parser）→ ②预检试算（patch_match，读原文+匹配，纯内存零副
//! 作用，任一失败整体拒绝）→ ③按序落盘（部分失败如实记账不回滚；写调用已发
//! 出而盘面未证实=不确定显式标注）。每个 hunk 写盘前检查取消。

use crate::builtin::cwd::CwdService;
use crate::builtin::patch_match::{apply_replacements, compute_replacements, SourceFile};
use crate::builtin::patch_parser::{parse_patch, Hunk};
use crate::builtin::HINT_PATH_OUTSIDE_ROOT;
use crate::contract::{PresentationMeta, ToolDefinition, ToolExec, ToolOutcome};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use yunyou_util::BoxFut;

/// 单文件预检产物（绝对路径+落盘计划；commit 阶段零再解析零再试算）。
struct Preflight {
    abs: PathBuf,
    op: Op,
    plan: Plan,
}

/// 落盘计划（预检阶段定稿；commit 阶段只执行）。
enum Plan {
    /// 写整份内容（Add 新文件 / Update 改写）。
    Write(String),
    /// 删除文件（Delete）。
    Remove,
    /// 移动：新路径写更新后内容 + 旧路径删除（Update File + Move to）。
    MoveTo { new_abs: PathBuf, content: String },
}

/// 操作类别（汇总行前缀：A 新增 / M 更新或移动 / D 删除）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Update,
    Delete,
    Move,
}

impl Op {
    fn tag(self) -> &'static str {
        match self {
            Op::Add => "A",
            Op::Update | Op::Move => "M",
            Op::Delete => "D",
        }
    }
}

pub(crate) struct ApplyPatchTool {
    pub cwd: Arc<CwdService>,
}

impl ToolDefinition for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "跨文件/同文件多区域/含新增删除移动的批量修改：提交 *** Begin Patch…*** End Patch 格式补丁（Add File 内容行 + 前缀；Update File 配 @@ 锚点与 -/+ 行；可 Move to 改名）。全部 hunk 先预检——任一不匹配整体拒绝、零副作用；通过后按序落盘，部分失败如实记账不回滚。结果回显绝对路径。单点小改用 edit。相对路径基于工作区根（bash 进程起始目录）解析，不随 bash cd 漂移；父目录须已存在。"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"patch": {"type": "string", "description": "补丁全文（含 *** Begin Patch / *** End Patch 包裹）"}},
            "required": ["patch"]
        })
    }

    fn output_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "applied": {"type": "array", "items": {"type": "string"}, "description": "已生效变更（A/M/D + 绝对路径）"},
                "summary": {"type": "string", "description": "git 风格汇总"}
            },
            "required": ["applied", "summary"]
        }))
    }

    fn presentation_meta(&self) -> Option<PresentationMeta> {
        Some(PresentationMeta::new("批量补丁", "应用 apply_patch 补丁", "patch"))
    }

    fn execute(&self, args: Value, exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let cwd = Arc::clone(&self.cwd);
        Box::pin(async move {
            let Some(patch) = args.get("patch").and_then(Value::as_str) else {
                return ToolOutcome::err("tool/args", "缺少 patch 参数（补丁全文）");
            };
            // ① 解析（错误=事实+示范，行号 1 起；code 族 tool/patch-*）
            let hunks = match parse_patch(patch) {
                Ok(hunks) => hunks,
                Err(e) => {
                    return ToolOutcome::err(e.code, format!("补丁第 {} 行：{}", e.line, e.message))
                }
            };
            // ② 预检（全部 hunk：路径解析+读原文+匹配试算，纯内存零副作用；
            // 任一失败整体拒绝——「整体拒绝的补丁不改任何文件」向模型明示）
            let mut pre: Vec<Preflight> = Vec::new();
            for hunk in &hunks {
                match hunk {
                    Hunk::Add { path, lines } => {
                        let abs = match resolve_or_reject(&cwd, path) {
                            Ok(abs) => abs,
                            Err(outcome) => return outcome,
                        };
                        if let Err(outcome) = reject_missing_parent(&abs) {
                            return outcome;
                        }
                        if abs.is_dir() {
                            return ToolOutcome::err(
                                "tool/io",
                                format!("写入目标 {} 是目录", abs.display()),
                            );
                        }
                        pre.push(Preflight {
                            abs,
                            op: Op::Add,
                            plan: Plan::Write(format!("{}\n", lines.join("\n"))),
                        });
                    }
                    Hunk::Delete { path } => {
                        let abs = match resolve_or_reject(&cwd, path) {
                            Ok(abs) => abs,
                            Err(outcome) => return outcome,
                        };
                        if abs.is_dir() {
                            return ToolOutcome::err(
                                "tool/io",
                                format!("{} 是目录：删除仅限文件", abs.display()),
                            );
                        }
                        if !abs.is_file() {
                            return ToolOutcome::err(
                                "tool/not-found",
                                format!("文件不存在：{}", abs.display()),
                            );
                        }
                        pre.push(Preflight { abs, op: Op::Delete, plan: Plan::Remove });
                    }
                    Hunk::Update { path, move_to, chunks } => {
                        let abs = match resolve_or_reject(&cwd, path) {
                            Ok(abs) => abs,
                            Err(outcome) => return outcome,
                        };
                        let bytes = match std::fs::read(&abs) {
                            Ok(bytes) => bytes,
                            Err(err) => {
                                return ToolOutcome::err(
                                    "tool/not-found",
                                    format!("文件不存在：{}（{err}）", abs.display()),
                                )
                            }
                        };
                        let text = match String::from_utf8(bytes) {
                            Ok(text) => text,
                            Err(_) => {
                                return ToolOutcome::err(
                                    "tool/binary",
                                    format!("{} 非 UTF-8 文本，拒绝补丁", abs.display()),
                                )
                            }
                        };
                        let file = SourceFile::parse(&text);
                        let replacements = match compute_replacements(&file, chunks) {
                            Ok(reps) => reps,
                            Err(failure) => {
                                return ToolOutcome::err(
                                    "tool/no-match",
                                    format!(
                                        "第 {} 个更新块（补丁行 {}）未在 {} 中匹配——前 80 字符预览：「{}」；hint：先 read 该文件核对缩进与行尾",
                                        failure.chunk_index + 1,
                                        failure.patch_line,
                                        abs.display(),
                                        failure.preview
                                    ),
                                )
                            }
                        };
                        let content = apply_replacements(&file, &replacements);
                        match move_to {
                            Some(dest) => {
                                let new_abs = match resolve_or_reject(&cwd, dest) {
                                    Ok(abs) => abs,
                                    Err(outcome) => return outcome,
                                };
                                if let Err(outcome) = reject_missing_parent(&new_abs) {
                                    return outcome;
                                }
                                if new_abs.is_dir() {
                                    return ToolOutcome::err(
                                        "tool/io",
                                        format!("移动目标 {} 是目录", new_abs.display()),
                                    );
                                }
                                pre.push(Preflight {
                                    abs: new_abs.clone(),
                                    op: Op::Move,
                                    plan: Plan::MoveTo { new_abs, content },
                                });
                                pre.push(Preflight { abs, op: Op::Delete, plan: Plan::Remove });
                            }
                            None => pre.push(Preflight { abs, op: Op::Update, plan: Plan::Write(content) }),
                        }
                    }
                }
            }
            // ③ 按序落盘（记账式：首错即停、不回滚，已生效项如实入账）
            let mut applied: Vec<String> = Vec::new();
            for p in &pre {
                if exec.cancel.is_cancelled() {
                    return ToolOutcome::err(
                        "tool/io",
                        format!(
                            "已取消：已生效 {} 项变更未回滚（{}）——先 read 相关文件核实现状再重试",
                            applied.len(),
                            applied.join("、")
                        ),
                    );
                }
                let outcome: Result<(), std::io::Error> = match &p.plan {
                    Plan::Write(content) => std::fs::write(&p.abs, content),
                    Plan::Remove => std::fs::remove_file(&p.abs),
                    Plan::MoveTo { new_abs, content } => {
                        std::fs::write(new_abs, content).and_then(|_| std::fs::remove_file(&p.abs))
                    }
                };
                if let Err(err) = outcome {
                    // 写调用已发出而盘面未证实：不确定显式标注（报告 §三）
                    let uncertain = match &p.plan {
                        Plan::Write(_) | Plan::MoveTo { .. } => {
                            "（不确定：截断写可能已发生，先 read 相关文件核对）"
                        }
                        Plan::Remove => "",
                    };
                    return ToolOutcome::err(
                        "tool/io",
                        format!(
                            "写盘失败 {}（{err}）{uncertain}。已生效 {} 项变更未回滚：{}",
                            p.abs.display(),
                            applied.len(),
                            applied.join("、")
                        ),
                    );
                }
                applied.push(format!("{} {}", p.op.tag(), p.abs.display()));
            }
            let summary = format!("Success. Updated the following files:\n{}", applied.join("\n"));
            ToolOutcome::ok(json!({ "applied": applied, "summary": summary }))
        })
    }
}

/// 路径解析统一闸（rev3：工作区根锚定+canonicalize 唯一化+starts_with 防穿越）。
fn resolve_or_reject(cwd: &CwdService, path: &str) -> Result<PathBuf, ToolOutcome> {
    cwd.resolve_workspace(path).map_err(|_| {
        ToolOutcome::err_h(
            "tool/path-outside-root",
            format!("路径越界: {path}"),
            HINT_PATH_OUTSIDE_ROOT,
        )
    })
}

/// 父目录存在性（task-50 同规：不静默建树，人话指路）。
fn reject_missing_parent(abs: &std::path::Path) -> Result<(), ToolOutcome> {
    let missing = abs
        .parent()
        .map(|p| !p.as_os_str().is_empty() && !p.exists())
        .unwrap_or(false);
    if missing {
        return Err(ToolOutcome::err_h(
            "tool/not-found",
            format!(
                "目录不存在：{}（写入目标：{}；先创建目录或用 bash mkdir 后重写）",
                abs.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                abs.display()
            ),
            "先创建目录（bash mkdir -p <目录>）后重写",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "patch_tool_tests.rs"]
mod tests;
