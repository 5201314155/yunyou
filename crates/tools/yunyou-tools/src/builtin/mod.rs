//! 内置工具注册模块：bash / read / write / edit / glob / grep（方案 §2.5）。

pub mod bash;
pub(crate) mod bash_exec;
pub(crate) mod capture;
pub mod bash_guard;
pub mod cwd;
pub mod fs_tools;
pub mod line_diff;
pub mod search;
pub mod write_verify;
pub mod tasks_tool;
pub mod patch_match;
pub mod patch_parser;
pub mod patch_tool;

use crate::contract::ToolDefinition;
use yunyou::{CordisError, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// 路径约束：把相对路径锚定在 root 内，拒绝越界（`..` 逃逸 / 绝对路径越出）。
///
/// # 失败语义
/// 规范化后不在 `root` 内 → `Err(tool/path-outside-root)`。
pub fn path_guard(root: &Path, path: &str) -> Result<PathBuf, CordisError> {
    let candidate = Path::new(path);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    // 逐段折叠 ..（容忍 root 未存在的场景，不依赖 canonicalize）
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(CordisError::new(
                        ErrorKind::Internal,
                        format!("tool/path-outside-root: 路径 \"{path}\" 越出根目录"),
                    ));
                }
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if !normalized.starts_with(root) {
        return Err(CordisError::new(
            ErrorKind::Internal,
            format!("tool/path-outside-root: 路径 \"{path}\" 越出根目录 {}", root.display()),
        ));
    }
    Ok(normalized)
}

/// 读文件的大小上限（防御性：防超大文件撑爆上下文）。
pub(crate) const MAX_READ_BYTES: u64 = 256 * 1024;

/// path-outside-root 类错误的统一指路面（P0 三缺陷之三）：中文、可执行、
/// 不含内部路径（例句用根内相对形态，不暴露 root 真实位置）。
pub const HINT_PATH_OUTSIDE_ROOT: &str =
    "绝对路径受限；请用项目根内相对路径，如 test-page.html";

/// 重定向被门禁拦截的指路面：模型想用 `>` 写含特殊字符的文件时改走 write 工具。
pub const HINT_REDIRECT_BLOCKED: &str =
    "命令含被拦截的重定向；若要写包含 > 等特殊字符的文件，请改用 write 工具";

/// tee/cp/mv/install 写文件被门禁拦截的指路面（同族意图，同一去处）。
pub const HINT_WRITE_BLOCKED: &str =
    "写文件命令被门禁拦截；请改用 write 工具在项目根内写文件";

/// 六内置工具的统一构造入口（供 ToolsPlugin 注册）。
///
/// CWD 服务在本函数内构造并以同一 Arc 分发六工具——同一实例＝「统一目录
/// 视图」的机制本身（bash 执行后捕获 pwd，文件类工具基于该值解析相对路径，
/// 见 builtin/cwd.rs）：签名保持 (root) 单参数，装配与测试调用点零改动。
#[allow(clippy::type_complexity)]
pub fn builtin_tools(
    root: Arc<Path>,
    enforcer: Option<std::sync::Arc<dyn crate::sandbox::SandboxEnforcer>>,
    background: Option<(
        std::sync::Arc<crate::tasks::TaskRegistry>,
        u64,
    )>,
) -> Vec<Arc<dyn ToolDefinition>> {
    let cwd = Arc::new(cwd::CwdService::new(root.as_ref()));
    let background_registry = background.as_ref().map(|(r, _)| r.clone());
    let (tasks, background_after_secs) = background
        .map(|(_, s)| s)
        .map(|s| (background_registry.clone(), s))
        .unwrap_or((None, bash::BACKGROUND_AFTER_DEFAULT_SECS));
    let mut tools: Vec<Arc<dyn ToolDefinition>> = vec![
        Arc::new(bash::BashTool {
            cwd: cwd.clone(),
            enforcer,
            tasks,
            background_after_secs,
        }),
        Arc::new(patch_tool::ApplyPatchTool { cwd: cwd.clone() }),
        Arc::new(fs_tools::ReadTool { cwd: cwd.clone() }),
        Arc::new(fs_tools::WriteTool { cwd: cwd.clone() }),
        Arc::new(fs_tools::EditTool { cwd: cwd.clone() }),
        Arc::new(search::GlobTool { cwd: cwd.clone() }),
        Arc::new(search::GrepTool { cwd }),
    ];
    // 后台任务管理三工具（池 task-20）：注册表缺席=一并隐身（纯前台形态）
    if let Some(registry) = background_registry {
        tools.push(Arc::new(tasks_tool::TaskOutputTool { tasks: registry.clone() }));
        tools.push(Arc::new(tasks_tool::TaskStopTool { tasks: registry.clone() }));
        tools.push(Arc::new(tasks_tool::TaskListTool { tasks: registry }));
    }
    tools
}
