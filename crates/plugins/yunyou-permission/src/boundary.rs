//! 工作区边界判定（task-16 S4，设计稿 §一 L5）。
//!
//! # 算法单源声明（跨域借力，lead 报备 2026-11-19）
//! canonicalize 三步=task-43 修复件 `yunyou-tools/builtin/cwd.rs::resolve_workspace`
//! 同款形态（②canonicalize 唯一化——本体在化本体，不在化父目录拼文件名；③
//! starts_with 规范化根校验，symlink 越界词法不可见运行时闸兜住）。此处借力
//! **算法形态**（零新依赖、不引 tools crate——permission 是 plugins 域纯策略卡）；
//! 算法演进以 cwd.rs 为真源同步。

use std::path::Path;

/// 区内判定（S4 防逃逸版）：三步 canonicalize 后 starts_with 规范化根。
///
/// 返回 None=任一路径不可判（root 未配置/解析失败→调用方保守区外）；
/// Err 语义收进 None（权限判定面不允许失败上抛中断链——宁问勿放）。
pub(crate) fn in_workspace(path: Option<&str>, root: Option<&str>) -> Option<bool> {
    let (path, root) = (path?, root?);
    let root_path = Path::new(root);
    let candidate = if Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        // 相对路径按根锚定（fs 工具族同语义：相对绑 WorkspaceRoot 不绑 cwd）
        root_path.join(path)
    };
    // ①规范化根（root 本体 canonicalize 失败=原样，与 cwd.rs 同容错）
    let canonical_root = std::fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
    // ②候选唯一化（本体存在化本体；不存在=化父目录拼文件名——新文件常态坑①同律）
    let canonical = if candidate.exists() {
        std::fs::canonicalize(&candidate).unwrap_or(candidate)
    } else {
        match candidate.parent().map(std::fs::canonicalize) {
            Some(Ok(dir)) => dir.join(candidate.file_name().unwrap_or_default()),
            _ => candidate,
        }
    };
    // ③starts_with 规范化根（越界=区外）
    Some(canonical.starts_with(&canonical_root))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // 测试断言惯例（同包既有测试同律）
mod tests {
    use super::*;

    /// 防逃逸矩阵：../ 逃逸/symlink 越界/新文件父目录化/区内正常/相对路径。
    #[test]
    fn 边界防逃逸矩阵() {
        let dir = std::env::temp_dir().join(format!("perm-s4-{}", std::process::id()));
        let root = dir.join("ws");
        let outside = dir.join("outside");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let r = root.to_str().unwrap();
        // 区内正常（绝对+相对）
        assert_eq!(in_workspace(Some(&format!("{r}/a.txt")), Some(r)), Some(true));
        assert_eq!(in_workspace(Some("sub/a.txt"), Some(r)), Some(true));
        // 新文件（本体不存在）=化父目录拼文件名（坑①同律）
        assert_eq!(in_workspace(Some("sub/new.txt"), Some(r)), Some(true));
        // ../ 逃逸（词法可见）
        assert_eq!(in_workspace(Some("../outside/x"), Some(r)), Some(false));
        // symlink 越界（词法不可见，canonicalize 运行时闸兜住——task-43 用例 d 同型）
        std::fs::write(outside.join("secret"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("secret"), root.join("evil")).unwrap();
        #[cfg(unix)]
        assert_eq!(in_workspace(Some("evil"), Some(r)), Some(false), "symlink 越界应拦");
        // root 未配置=不可判
        assert_eq!(in_workspace(Some("/a"), None), None);
        assert_eq!(in_workspace(None, Some(r)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
