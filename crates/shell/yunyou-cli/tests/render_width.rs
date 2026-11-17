//! 定宽终端回归（D-UI-1 缺陷本体 · 从 tests/render.rs 按场景拆出）。
//!
//! 覆盖：定宽 80 列长回复多分片喂入 → **混合流式帧语义**（任务30 三期④
//! 升级断言口径）：
//! 1. 同一活动行生命周期内，重绘帧（`\r\e[K` 分隔）剥 ANSI 后**前缀单调
//!    扩展**——D-UI-1 残留的根因是折行后重复重绘产生内容错位帧（旧实测
//!    200 帧/167 对前缀残留，如「│ The user is asking me」→「│ …to
//!    introduce Rus」），错位帧违反单调性，本断言必红；
//! 2. 重绘帧可见宽恒 < 列宽（将溢出即落定，多视觉行永不重绘）；
//! 3. 落定行内容无损：各落定行最终帧拼接 = 原文（溢出拆行不增删字符）。
//!
//! 列宽口径：Renderer 交互构造默认 80（COLUMNS 未设），与本文件软折行
//! 复现口径一致；折行场景由溢出落定行的 80 列软折行承担。

use yunyou_cli::render::{Profile, Renderer, TurnEvent};
use yunyou_cli::render::tool;

fn full() -> Renderer<Vec<u8>> {
    Renderer::new(Vec::new(), Profile::Full, true)
}

/// D-UI-1 缺陷本体回归（业主实测·最高优先；三期④帧语义升级）：
/// 定宽 80 列 + 长回复多分片喂入，重绘帧单调扩展、帧宽不越界、落定无损。
#[test]
fn 定宽80列_长回复多分片_帧单调无残留() {
    const COLS: usize = 80; // 定宽口径（伪 TTY 复现）
    let long = "组件内核对标 cordis，装配、替换与生命周期管理全部重写，插件形态插在主板缝上，什么都可以换；落定行永不回改。";
    let mut r = full();
    // 多分片喂入（模拟 LLM 流式增量；按字符切，避免切断 UTF-8）
    for chunk in long.chars().collect::<Vec<_>>().chunks(4) {
        r.text_delta(&chunk.iter().collect::<String>()).unwrap();
    }
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(1500),
        prompt_tokens: Some(200),
        completion_tokens: Some(2000),
        context_tokens: None,
        context_window: None, // task-27
        model: "z-ai/glm-5.3-flash".into(),
    })
    .unwrap();
    let raw = String::from_utf8(r.into_output()).unwrap();
    assert!(raw.contains("\r\x1b[K"), "混合流式应存在单视觉行重绘帧：{raw:?}");
    // ① 帧单调：每个落定段（\n 分隔的行生命周期）内，渐进帧剥 ANSI 后
    //    前缀扩展——D-UI-1 错位残留帧违反单调性必红
    let mut settled: Vec<String> = Vec::new(); // 各落定行的最终内容
    for segment in raw.split('\n') {
        let frames: Vec<String> = segment.split("\r\x1b[K").map(strip_ansi).collect();
        for pair in frames.windows(2) {
            assert!(
                pair[1].starts_with(pair[0].as_str()),
                "帧应前缀单调扩展（错位帧=D-UI-1 残留）：前帧 {:?} 后帧 {:?}",
                pair[0],
                pair[1]
            );
        }
        if let Some(last) = frames.last() {
            settled.push(last.to_string());
        }
    }
    // ② 帧宽不越界：重绘帧（非落定段——落定段以 \n 结尾，可折行超宽）
    //    可见宽 < 80（将溢出已落定，多视觉行零重绘）
    for frame in raw.split("\r\x1b[K").skip(1) {
        if frame.ends_with('\n') {
            continue; // 落定行：允许折行，不属于重绘帧
        }
        let clean = strip_ansi(frame);
        assert!(
            tool::visible_width(&clean) < COLS,
            "重绘帧可见宽不得达到列宽（溢出应已落定）：{clean:?}"
        );
    }
    // ③ 落定无损：各落定行最终帧拼接 = 原文（溢出拆行不增删字符；
    //    落款行属回合同步聚合输出，非流式正文，剔除后比对）
    let joined: String = settled.into_iter().filter(|s| !s.contains("本回合")).collect();
    assert_eq!(joined, long, "落定行拼接应无损还原原文：{joined:?}");
    // ④ 折行场景确在：落定行按 80 列软折行 > 1 视觉行（未覆盖折行则用例无效）
    let rows = visual_rows(long, COLS);
    assert!(rows.len() >= 2, "长回复应在 80 列折行（未覆盖折行则用例无效）：{:?}", rows.len());
}

/// 落款行定宽（任务30 二期④）：80 列完整呈现；48 列模型名截断 … 收尾。
/// 两档都断言剥 ANSI 后恒为**单视觉行**（业主实测：48 列下长模型名把落款
/// 折成两行）——视觉行口径与本文件 D-UI-1 用例同源（全角 2 列软折行复现）。
#[test]
fn 落款行_80与48_恒单视觉行() {
    for cols in [80usize, 48] {
        let mut r = Renderer::new(Vec::new(), Profile::Full, true);
        r.set_width(cols);
        r.turn_done(&TurnEvent::TurnDone {
            elapsed: std::time::Duration::from_millis(3200),
            prompt_tokens: Some(1204),
            completion_tokens: Some(8931),
            context_tokens: None,
            context_window: None, // task-27
            model: "z-ai/glm-5.3-flash".into(),
        })
        .unwrap();
        let raw = String::from_utf8(r.into_output()).unwrap();
        assert!(!raw.contains('\r'), "落款零重绘：{raw:?}");
        let clean = strip_ansi(&raw);
        let line = clean.trim_end_matches('\n');
        let rows = visual_rows(line, cols);
        assert_eq!(rows.len(), 1, "{cols} 列下落款应恒单视觉行：{line:?}");
        if cols == 48 {
            // 任务30 四期 1.3：窄屏（<60 列）去「──」与「本回合」装饰，
            // 预算全给模型名——48 列下全名完整（旧断言「截断加 …」随新语义作废）
            assert!(line.contains("z-ai/glm-5.3-flash"), "48 列下模型名应全名完整：{line:?}");
            assert!(!line.contains("──") && !line.contains("本回合"), "窄屏应去装饰：{line:?}");
        } else {
            assert!(line.contains("z-ai/glm-5.3-flash"), "80 列下模型名应完整：{line:?}");
            assert!(line.contains("──") && line.contains("本回合"), "宽屏保留装饰形态：{line:?}");
        }
    }
}

/// 剥 ANSI 转义序列（CSI：ESC `[` … 终结字节 0x40-0x7E；两字节转义随 ESC 丢弃）。
fn strip_ansi(text: &str) -> String {
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

/// 按定宽列软折行为视觉行（模拟终端：全角 2 列，复用 [`tool::char_width`]）。
fn visual_rows(text: &str, cols: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    let mut width = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            rows.push(String::new());
            width = 0;
            continue;
        }
        let w = tool::char_width(ch);
        if width + w > cols {
            rows.push(String::new());
            width = 0;
        }
        rows.last_mut().unwrap().push(ch);
        width += w;
    }
    rows
}