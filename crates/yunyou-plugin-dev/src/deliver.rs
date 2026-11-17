//! plugin_deliver 工具核心（task-78/K-2-③）：交付三件套生成，限 workspace dist。
//!
//! 三件套=tgz 安装包+安装教程.md+介绍.md（插件交付规则，业主定）。
//! 输出仅限 `<workspace>/dist/<插件功能名>/`——越界拒绝（resolve_workspace 闸
//! 同款）。dry-run=落盘前预演（lead 批准③，防误写，30KB 守卫同族）。

use std::path::{Path, PathBuf};

/// 交付结果（三件套落点清单）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverResult {
    pub tgz_path: PathBuf,
    pub tutorial_path: PathBuf,
    pub intro_path: PathBuf,
    pub dry_run: bool,
}

/// 路径越界错误（人话；resolve_workspace 闸同款口径）。
#[derive(Debug, PartialEq, Eq)]
pub struct OutOfWorkspaceError(pub PathBuf);

/// dist 目标目录解析：`<workspace>/dist/<插件功能名>/`，越界拒绝。
///
/// `plugin_name` 取 manifest id；含 `..`/绝对路径片段即拒绝（人话口径）。
pub fn dist_dir(workspace: &Path, plugin_name: &str) -> Result<PathBuf, OutOfWorkspaceError> {
    if plugin_name.contains("..") || Path::new(plugin_name).is_absolute() || plugin_name.is_empty()
    {
        return Err(OutOfWorkspaceError(PathBuf::from(plugin_name)));
    }
    Ok(workspace.join("dist").join(plugin_name))
}

/// 三件套文件名推导（插件交付规则口径）。
pub fn artifact_names(plugin_name: &str, version: &str) -> [String; 3] {
    [
        format!("{plugin_name}-{version}.tgz"),
        "安装教程.md".to_string(),
        "介绍.md".to_string(),
    ]
}

/// 交付执行（dry_run=true=仅返回落点清单不落盘——lead 批准③防误写）。
///
/// 本函数为核心纯逻辑（路径推导+清单组装）；实际 npm pack 与文件写入由
/// 调用方（工具层）执行——单元测试零副作用口径。
pub fn plan_deliver(
    workspace: &Path,
    plugin_name: &str,
    version: &str,
    dry_run: bool,
) -> Result<DeliverResult, OutOfWorkspaceError> {
    let dir = dist_dir(workspace, plugin_name)?;
    let [tgz, tutorial, intro] = artifact_names(plugin_name, version);
    Ok(DeliverResult {
        tgz_path: dir.join(&tgz),
        tutorial_path: dir.join(&tutorial),
        intro_path: dir.join(&intro),
        dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dist_目录推导_合法名() {
        let ws = Path::new("/tmp/ws");
        let dir = dist_dir(ws, "hello").expect("合法名应通过");
        assert_eq!(dir, Path::new("/tmp/ws/dist/hello"));
    }

    #[test]
    fn 越界拒绝_相对上跳() {
        let err = dist_dir(Path::new("/tmp/ws"), "../escape").expect_err("应拒绝");
        assert_eq!(err, OutOfWorkspaceError(PathBuf::from("../escape")));
    }

    #[test]
    fn 越界拒绝_绝对路径() {
        assert!(dist_dir(Path::new("/tmp/ws"), "/abs").is_err());
    }

    #[test]
    fn 越界拒绝_空名() {
        assert!(dist_dir(Path::new("/tmp/ws"), "").is_err());
    }

    #[test]
    fn 三件套命名_交付规则口径() {
        let names = artifact_names("hello", "0.2.0");
        assert_eq!(names[0], "hello-0.2.0.tgz");
        assert_eq!(names[1], "安装教程.md");
        assert_eq!(names[2], "介绍.md");
    }

    #[test]
    fn plan_deliver_dry_run_只出清单() {
        let r = plan_deliver(Path::new("/tmp/ws"), "hello", "0.2.0", true)
            .expect("合法应通过");
        assert!(r.dry_run);
        assert!(r.tgz_path.ends_with("hello-0.2.0.tgz"));
        assert!(r.tutorial_path.ends_with("安装教程.md"));
        assert!(r.intro_path.ends_with("介绍.md"));
    }
}
