//! 文件式人设装载（池 task-6，业主令「改人设这些不应该不需要动代码吗，就改个
//! agent 的 md」）：persona.md 改完即生效，全程零代码配置。
//!
//! # 优先级（md 存在即 shadow toml，向后兼容）
//!
//! 1. **项目根** `.yunyou/persona.md`（当前工作目录——仓库级人设随项目走）；
//! 2. **yunyou-home** `persona.md`（`--home`/`$YUNYOU_HOME`/`~/.yunyou`——用户级
//!    全局人设；home 目录本身已是 `~/.yunyou`，故文件直接在 home 根下）；
//! 3. yunyou.toml `[agent_loop]`/`[system_prompt]` persona 字符串（既有
//!    [`crate::assemble::bridge_persona`] 桥接槽——md 命中时直接把内容写进同一
//!    槽，**不新增第二机制**）。
//!
//! # 语义边界
//!
//! - 文件存在但 trim 后为空＝**视为不存在**，继续向下一级回落（空文件不是
//!   「显式无人设」，防误存空文件把人设悄悄清掉）；
//! - md 文本里 `#` 顶层标题切段由 system-prompt 组件侧完成（见
//!   `yunyou_system_prompt::persona_md`），本模块只负责文件发现与读取。

use serde_json::Value;
use std::path::Path;

/// 装配接线（复用既有 shadow 人设槽，不新增第二机制）：md 命中即把内容写进
/// `agent_loop.persona`——[`crate::assemble::bridge_persona`] 桥接只在槽空时填
/// toml 值，故 md 存在即 shadow toml（两级文件人设+字符串人设向后兼容）。
pub(crate) fn apply_to_raw(mut raw: Value, project_root: &Path, home: &Path) -> Value {
    if let Some(persona) = resolve_persona_md(project_root, home) {
        raw["agent_loop"]["persona"] = Value::String(persona);
    }
    raw
}

/// 装配用项目根：进程当前工作目录（仓库级人设随项目走；取不到＝无项目级人设）。
pub(crate) fn project_root() -> Option<std::path::PathBuf> {
    std::env::current_dir().ok()
}

/// 项目根人设文件的固定相对路径（.yunyou 目录由本约定新引入，与仓库配置共居）。
pub const PROJECT_PERSONA_REL: &str = ".yunyou/persona.md";

/// yunyou-home 下人设文件名。
pub const HOME_PERSONA_FILE: &str = "persona.md";

/// 按优先级解析生效人设文件内容：项目根 > yunyou-home；两级都未命中或命中空
/// 文件 → None（回落 toml 字符串人设）。读取失败（存在但不可读）按未命中处理
/// ——人设是增强面，不因它 fail-loud 阻断启动。
pub fn resolve_persona_md(project_root: &Path, home: &Path) -> Option<String> {
    [project_root.join(PROJECT_PERSONA_REL), home.join(HOME_PERSONA_FILE)]
        .into_iter()
        .find_map(|path| {
            std::fs::read_to_string(&path)
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// pid 隔离临时目录。
    fn temp_dir(tag: &str) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("yunyou-persona-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn write(dir: &Path, rel: &str, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().ok_or("缺父目录")?)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// 三级优先：项目根 > home；两者都缺 → None（回落 toml）。
    #[test]
    fn 三级优先与回落() -> TestResult {
        let root = temp_dir("prio")?;
        // 两级都缺 → None
        assert_eq!(resolve_persona_md(&root, &root.join("home")), None);
        // 只有 home 级 → home 内容
        write(&root.join("home"), "persona.md", "home 人设")?;
        assert_eq!(
            resolve_persona_md(&root, &root.join("home")).as_deref(),
            Some("home 人设")
        );
        // 项目根出现 → shadow home
        write(&root, ".yunyou/persona.md", "project 人设")?;
        assert_eq!(
            resolve_persona_md(&root, &root.join("home")).as_deref(),
            Some("project 人设")
        );
        Ok(())
    }

    /// 空文件＝不存在（继续回落），trim 后非空才生效。
    #[test]
    fn 空文件视为不存在() -> TestResult {
        let root = temp_dir("empty")?;
        let home = root.join("home");
        write(&home, "persona.md", "home 人设")?;
        // 项目根空文件：不 shadow，回落 home
        write(&root, ".yunyou/persona.md", "  \n\n")?;
        assert_eq!(resolve_persona_md(&root, &home).as_deref(), Some("home 人设"));
        // home 也空：两级全空 → None
        write(&home, "persona.md", "")?;
        assert_eq!(resolve_persona_md(&root, &home), None);
        Ok(())
    }

    /// 段落切分与装配联动（组件侧 split_md_sections 消费 md 文本）：
    /// md 人设经 compose 后按 `#` 标题切段、段序保文件顺序。
    #[test]
    fn md人设切段进装配() -> TestResult {
        let md = "你是一个严谨的助手。\n# 红线\n永不删除用户数据。";
        let sections = yunyou_system_prompt::persona_md::split_md_sections(md);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0], "你是一个严谨的助手。");
        assert_eq!(sections[1], "# 红线\n永不删除用户数据。");
        // 装配后两段按人设槽档位依次出现
        let prompt = yunyou_system_prompt::DefaultSystemPrompt::with_persona(md.to_string())?;
        let text = yunyou_system_prompt::SystemPrompt::compose(&prompt, "", &[]);
        assert!(text.contains("你是一个严谨的助手。"), "{text}");
        assert!(text.contains("# 红线"), "{text}");
        let first = text.find("你是一个严谨的助手。").ok_or("段一应出现")?;
        let second = text.find("# 红线").ok_or("段二应出现")?;
        assert!(first < second, "段序应保文件顺序：{text}");
        Ok(())
    }
}
