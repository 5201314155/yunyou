//! 渲染模块回归（task-31 自 render.rs 按场景族目录化拆分，纯搬移零变更）：
//! 以设计 mock（docs/设计/演示/yunyou）的帧序为蓝本，断言各视觉块输出与
//! 规范 §4 一致（含 YUNYOU_PLAIN/无色降级）。子模块分族见各文件头注；
//! 共享 helper（full/plain/assert_no_forbidden_ansi/strip_ansi）驻本文件，
//! 子模块经 `use super::*` 复用（tests 单测试目标不变：cargo test --test render）。

pub use yunyou_cli::render::{event::DiffHunk, DetailKind, Profile, Renderer, TurnEvent};
pub use yunyou_cli::render::terminal;


pub fn full() -> Renderer<Vec<u8>> {
    Renderer::new(Vec::new(), Profile::Full, true)
}

pub fn plain() -> Renderer<Vec<u8>> {
    Renderer::new(Vec::new(), Profile::Plain, true)
}

/// 输出里不应出现 alternate screen / 光标定位（规范 §5.1 禁用项；
/// 三期④分域表述：会话流内禁清屏不变——/new 会话边界清屏与 panel \e[J
/// 暂态行属各自规范域豁免（C3 裁定），不在本渲染层输出断言之列）。
pub fn assert_no_forbidden_ansi(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    assert!(!text.contains("\x1b[?1049"), "禁 alternate screen");
    assert!(!text.contains("\x1b[2J"), "会话流内禁全屏清屏");
    assert!(!text.contains("\x1b[r") || !text.contains(';'), "禁光标定位");
}

/// 剥 SGR 转义序列（混合流式帧断言用；CSI：ESC `[` … 终结字节 0x40-0x7E）。
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
        } else if chars.next() == Some('[') {
            for next in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&next) {
                    break; // 终结字节（如 m/K）
                }
            }
        }
    }
    out
}

mod banner;
mod degrade;
mod detail;
mod markdown_inline;
mod status_line;
mod stream_fence;
mod stream_width;
mod thinking;
mod tool_card;
