//! 键位架构模块（TUI 战役交互矩阵 A1-A3 骨架批，lead 批 2026-11-19 并行
//! 双席令：本模块=骨架，编辑行整族=yun-ya lineedit 域——文件面零交叠）。
//!
//! 三层（claude 同构，锚=〈claude-交互键位行为提取-20261119〉§一）：
//! ①动作名注册表（action.rs，动作名一等公民）→②默认绑定+context 优先级
//! 解析器（bindings.rs+context.rs，活跃 context 盖 Global）→③保留键/和弦/
//! 平台常量三纪律（reserved.rs）。用户覆盖入口=Registry::override_binding
//! （保留键过闸）。
//!
//! **零行为纪律**：本批只落注册/解析/校验结构，不接线任何既有键处理路径
//! （lineedit/repl 既有 if 分支归编辑行批，迁移另行立票）——现有交互零变化。

pub mod action;
pub mod bindings;
pub mod context;
pub mod reserved;

pub use action::Action;
pub use bindings::{default_registry, Registry};
pub use context::{Context, KeyCombo};
pub use reserved::{platform, ChordTracker, ChordState, ReservedKeys};

#[cfg(test)]
mod tests {
    use super::*;

    /// A2 锁：活跃 context 命中优先——同一 escape 三态三行为互不误触。
    #[test]
    fn context_优先级解析() {
        let reg = default_registry();
        let esc = KeyCombo::parse("escape");
        assert_eq!(reg.resolve(Context::Chat, &esc), Some(Action::Cancel));
        assert_eq!(reg.resolve(Context::Panel, &esc), Some(Action::PanelClose));
        assert_eq!(reg.resolve(Context::Confirmation, &esc), Some(Action::ConfirmNo));
        // Global 兜底：Chat 未注册的 ctrl+l 落 Global
        assert_eq!(reg.resolve(Context::Chat, &KeyCombo::parse("ctrl+l")), Some(Action::Redraw));
        // 双方都未注册=无动作（不 panic）
        assert_eq!(reg.resolve(Context::Panel, &KeyCombo::parse("ctrl+q")), None);
    }

    /// A3 锁：保留键拒重绑（ctrl+c/d 禁覆盖），非保留键覆盖成功且生效。
    #[test]
    fn 保留键_拒绝重绑() {
        let mut reg = default_registry();
        let reserved = ReservedKeys;
        assert!(reg.override_binding(&reserved, Context::Global, "ctrl+c", Action::Submit).is_err());
        assert!(reg.override_binding(&reserved, Context::Global, "ctrl+d", Action::Submit).is_err());
        // 非保留键：Chat 的 ctrl+s 改绑到 Global（覆盖语义）
        assert!(reg.override_binding(&reserved, Context::Global, "ctrl+s", Action::Stash).is_ok());
        assert_eq!(
            reg.resolve(Context::Chat, &KeyCombo::parse("ctrl+s")),
            Some(Action::Stash)
        );
    }

    /// A3 锁：和弦状态机——前缀进入等待、第二键结算、escape 消歧取消。
    #[test]
    fn 和弦_前缀_结算_消歧() {
        let mut t = ChordTracker::default();
        assert!(!t.pending());
        assert_eq!(t.feed(&KeyCombo::parse("ctrl+x")), None, "前缀键进入等待");
        assert!(t.pending());
        assert_eq!(
            t.feed(&KeyCombo::parse("k")),
            Some(("ctrl+x".to_string(), "k".to_string())),
            "第二键结算 (前缀,键)"
        );
        assert!(!t.pending());
        // escape 消歧：等待态按 escape=取消，不产出
        t.feed(&KeyCombo::parse("ctrl+x"));
        assert_eq!(t.feed(&KeyCombo::parse("escape")), None);
        assert!(!t.pending());
    }

    /// A3 锁：平台常量在位（键字面单源）。
    #[test]
    fn 平台常量() {
        assert_eq!(platform::MODE_CYCLE_KEY, "shift+tab");
        assert_eq!(platform::CHORD_PREFIX, "ctrl+x");
    }
}
