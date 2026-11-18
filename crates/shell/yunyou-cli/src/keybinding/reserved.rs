//! 保留键+和弦+平台常量三纪律（交互矩阵 A3，claude reservedShortcuts.ts:16-73
//! /defaultBindings.ts:64-65、:29-31 同构）。

use super::context::KeyCombo;

/// 保留键制度：**在表内定义但禁止用户重绑**（防用户覆盖掉安全/退出语义）。
const RESERVED: [&str; 2] = ["ctrl+c", "ctrl+d"];

/// 平台/终端差异常量化（claude MODE_CYCLE_KEY/IMAGE_PASTE_KEY 同构；Rust 侧
/// crossterm 统一上报，常量先落、分支逻辑随各批消费）。
pub mod platform {
    /// 模式循环键：终端支持 VT 系列上报用 shift+tab，否则 meta+m 兜底
    /// （claude defaultBindings.ts:29-31 同源语义）。
    pub const MODE_CYCLE_KEY: &str = "shift+tab";
    /// chord 前缀（claude ctrl+x 族：前缀避免遮蔽 readline 编辑键 ctrl+a/b/e/f）。
    pub const CHORD_PREFIX: &str = "ctrl+x";
}

/// 保留键校验器（覆盖入口统一过闸）。
#[derive(Debug, Default, Clone, Copy)]
pub struct ReservedKeys;

impl ReservedKeys {
    /// 重绑过闸：命中保留键=拒绝（错误信息带纪律出处，用户可读）。
    pub fn check(&self, combo: &KeyCombo) -> Result<(), String> {
        for spec in RESERVED {
            if *combo == KeyCombo::parse(spec) {
                return Err(format!(
                    "键 {spec} 为保留键（安全/退出语义），禁止重绑（交互矩阵 A3 纪律）"
                ));
            }
        }
        Ok(())
    }
}

/// 和弦状态机（A3：chord_started/chord_cancelled 两态；ctrl+x 前缀族。
/// escape 消歧=和弦等待中按 escape 即取消等待，不触发其他 escape 动作
/// ——claude resolver.ts:88 同源语义）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ChordState {
    #[default]
    Idle,
    /// 前缀已按（如 ctrl+x），等待第二键。
    Started,
}

#[derive(Debug, Default)]
pub struct ChordTracker {
    state: ChordState,
}

impl ChordTracker {
    /// 喂键：Idle 收到前缀→Started；Started 收到第二键→结算 (前缀,键) 回 Idle；
    /// Started 收到 escape→取消回 Idle（消歧）；其余态透传 None。
    pub fn feed(&mut self, combo: &KeyCombo) -> Option<(String, String)> {
        let prefix = platform::CHORD_PREFIX;
        match self.state {
            ChordState::Idle => {
                if format!("{prefix}+{}", combo.key) == format!("{prefix}+x") && combo.ctrl && !combo.shift && !combo.alt
                {
                    self.state = ChordState::Started;
                    None
                } else {
                    None
                }
            }
            ChordState::Started => {
                self.state = ChordState::Idle;
                if combo.key == "escape" {
                    None // 和弦取消（escape 消歧）
                } else {
                    Some((prefix.to_string(), combo.key.clone()))
                }
            }
        }
    }

    /// 当前是否在和弦等待态（调用方可用于提示行/超时回退）。
    pub fn pending(&self) -> bool {
        self.state == ChordState::Started
    }
}
