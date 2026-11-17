//! creation_mode 开关与提示段注入（task-78/K-2-③）。
//!
//! 开关=`[creation_mode] enabled`（默认 false——显式启用律，与 76
//! auto_enable=false/team auto_winddown=false 三例同源：开发者能力一律
//! 显式开启）。启用=register_section ③缝注入提示段五条常驻。

/// creation_mode 开关（配置面；默认 false=显式启用律）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreationModeConfig {
    pub enabled: bool,
}

impl Default for CreationModeConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl CreationModeConfig {
    /// 从配置段构造（缺段=默认 false；显式启用律）。
    pub fn from_segment(segment: Option<bool>) -> Self {
        Self {
            enabled: segment.unwrap_or(false),
        }
    }

    /// 提示段是否应注入（启用=注入五条常驻）。
    pub fn should_inject(&self) -> bool {
        self.enabled
    }
}

/// register_section ③缝注入调用形态（task-19 先例同款三参数）。
///
/// 返回 (段名, 段文本, 常驻标志)——接入方按缝契约注册。
pub fn section_registration(cfg: &CreationModeConfig) -> Option<(&'static str, String, bool)> {
    if !cfg.should_inject() {
        return None; // 禁用=不注入（显式启用律）
    }
    Some((
        crate::prompt_section::SECTION_NAME,
        crate::prompt_section::render_section(),
        crate::prompt_section::SECTION_PERSISTENT,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 默认禁用_显式启用律() {
        let cfg = CreationModeConfig::default();
        assert!(!cfg.enabled, "默认 false=显式启用律（与 76/team 同源）");
        assert!(!cfg.should_inject());
        assert!(section_registration(&cfg).is_none(), "禁用=不注入");
    }

    #[test]
    fn 显式启用_注入三参数() {
        let cfg = CreationModeConfig::from_segment(Some(true));
        assert!(cfg.should_inject());
        let (name, text, persistent) = section_registration(&cfg).expect("启用应注入");
        assert_eq!(name, "creation_mode");
        assert!(persistent);
        assert!(text.contains("manifest.toml"), "段文本应含五条内容");
    }

    #[test]
    fn 缺段视为禁用() {
        let cfg = CreationModeConfig::from_segment(None);
        assert!(!cfg.enabled, "缺段=默认 false（显式启用律）");
    }
}
