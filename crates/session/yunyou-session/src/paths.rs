//! 会话/记忆共享路径约定：项目目录名净化单源（memory.v1 设计 §二「一处实现
//! 两处引用」——`projects/<san>/` 目录族同一净化函数，会话存储与记忆缝共用）。

use std::path::Path;

/// 项目目录净化：把项目 cwd 映射为单层目录名。
///
/// 规则（有意从简，测试锁定）：字母/数字/`-`/`_`/CJK 保留，其余（`/`、空格、
/// `.` 等）折叠为 `-`；结果为空时落 `_empty`。不做 hash——目录名保持人类可读
/// （memdir 同思路：排障时直接认得出是哪个项目）。
pub fn sanitize_project_dir(cwd: &Path) -> String {
    let raw = cwd.to_string_lossy();
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() { "_empty".to_string() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 常规路径净化：分隔符与点折叠，可读性保留。
    #[test]
    fn 常规路径折叠与可读() {
        assert_eq!(sanitize_project_dir(Path::new("/home/wu/agent/yunyou")), "home-wu-agent-yunyou");
        assert_eq!(sanitize_project_dir(Path::new("/home/wu/my_app.v2")), "home-wu-my_app-v2");
    }

    /// 中文保留（可读性），全非法输入落 `_empty`。
    #[test]
    fn 中文保留_空输入兜底() {
        assert_eq!(sanitize_project_dir(Path::new("/home/wu/小云")), "home-wu-小云");
        assert_eq!(sanitize_project_dir(Path::new("///")), "_empty");
    }
}
