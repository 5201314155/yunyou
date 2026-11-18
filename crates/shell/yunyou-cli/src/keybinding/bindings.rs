//! 默认绑定表+context 优先级解析器（交互矩阵 A1 第二层+A2，claude
//! defaultBindings.ts:37-230/useKeybinding.ts:33-54 同构）。
//!
//! 解析序（A2 核心）：**活跃 context 命中优先，未命中落 Global 兜底**——
//! 同一 escape 在 Chat=取消、Panel=收面板、Confirmation=否，三处互不误触。

use super::action::Action;
use super::context::{Context, KeyCombo};
use std::collections::HashMap;

/// 默认绑定表：注册（动作, context, 键字面）→ 查询表。
pub struct Registry {
    bindings: HashMap<(Context, KeyCombo), Action>,
}

impl Registry {
    /// 由绑定清单构建（重复 (context, combo) 后者覆盖前者=用户覆盖语义的
    /// 同构；保留键校验见 reserved 纪律，构建入口是 [`Self::with_overrides`]）。
    pub fn build(entries: &[(Context, &str, Action)]) -> Self {
        let mut bindings = HashMap::new();
        for (ctx, key, action) in entries {
            bindings.insert((*ctx, KeyCombo::parse(key)), *action);
        }
        Self { bindings }
    }

    /// 解析（A2）：活跃 context 命中优先，未命中落 Global 兜底。
    pub fn resolve(&self, ctx: Context, combo: &KeyCombo) -> Option<Action> {
        self.bindings
            .get(&(ctx, combo.clone()))
            .or_else(|| self.bindings.get(&(Context::Global, combo.clone())))
            .copied()
    }

    /// 用户覆盖（loadUserBindings 同构：后者覆盖；保留键拒绝见 reserved）。
    pub fn override_binding(
        &mut self,
        reserved: &super::reserved::ReservedKeys,
        ctx: Context,
        key: &str,
        action: Action,
    ) -> Result<(), String> {
        let combo = KeyCombo::parse(key);
        reserved.check(&combo)?;
        self.bindings.insert((ctx, combo), action);
        Ok(())
    }
}

/// 默认绑定清单（claude defaultBindings 四态映射；键字面与 claude 同源，
/// 平台差异见 reserved 常量；只注册不接线=零行为）。
pub fn default_entries() -> Vec<(Context, &'static str, Action)> {
    use Context::*;
    vec![
        // —— Global 兜底（任何态生效的最小集）——
        (Global, "ctrl+c", Action::Interrupt),
        (Global, "ctrl+d", Action::Exit),
        (Global, "ctrl+l", Action::Redraw),
        // —— Chat/编辑态 ——
        (Chat, "enter", Action::Submit),
        (Chat, "escape", Action::Cancel),
        (Chat, "shift+tab", Action::CycleMode),
        (Chat, "up", Action::HistoryPrev),
        (Chat, "down", Action::HistoryNext),
        (Chat, "ctrl+r", Action::HistorySearch),
        (Chat, "ctrl+s", Action::Stash),
        // —— Panel（斜杠补全面板）——
        (Panel, "tab", Action::CompleteNext),
        (Panel, "up", Action::PanelPrev),
        (Panel, "down", Action::PanelNext),
        (Panel, "enter", Action::PanelExec),
        (Panel, "escape", Action::PanelClose),
        // —— Confirmation（批 7 权限对话框直连；规格 B 三段式键位最小集）——
        (Confirmation, "y", Action::ConfirmYes),
        (Confirmation, "enter", Action::ConfirmYes),
        (Confirmation, "n", Action::ConfirmNo),
        (Confirmation, "escape", Action::ConfirmNo),
        (Confirmation, "a", Action::ConfirmAlways),
        (Confirmation, "shift+tab", Action::ConfirmCycleMode),
        (Confirmation, "tab", Action::ConfirmNextField),
        (Confirmation, "space", Action::ConfirmToggle),
    ]
}

/// 缺省注册表（默认绑定，无用户覆盖）。
pub fn default_registry() -> Registry {
    Registry::build(&default_entries())
}
