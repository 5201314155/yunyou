//! 降级矩阵族（Plain/非交互静默/finish 收尾/档位注入矩阵/色板单源）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// 降级：YUNYOU_PLAIN（Plain 档）→ ASCII 符号、无任何 ANSI。
#[test]
fn 纯文本档_ascii_降级无ansi() {
    let mut r = plain();
    r.prompt().unwrap();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c7".into(),
        name: "read".into(),
        ok: true,
        summary: "26 行".into(),
        detail: None,
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    // Plain 档：无 SGR 色彩/样式；\r\e[K 活动行控制与 mock 一致保留（TTY 下 \r 仍回位）
    assert!(
        !text.contains("\x1b[38;5") && !text.contains("\x1b[1m") && !text.contains("\x1b[2m"),
        "Plain 档应无 SGR 色彩：{text:?}"
    );
    assert!(text.contains('>') && !text.contains('❯'), "提示符应降级 >");
    assert!(text.contains(" v "), "成功符应降级 v");
    assert_no_forbidden_ansi(text.as_bytes());
}

/// 非交互终端（管道 / TERM=dumb → interactive=false）：
/// spinner 帧零输出、未满行不 \r 重绘（满行才落屏）——审查契约补充 1 的降级语义。
#[test]
fn 非交互_暂态全静默_满行才落屏() {
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    // spinner 帧：零输出
    let call = TurnEvent::ToolCall {
        call_id: "cx".into(),
        name: "bash".into(),
        summary: "echo".into(),
        parent_tool_call_id: None,
        batch_id: None,
    };
    r.tool_running_frame(&call, 3, 0).unwrap();
    r.tool_call_frame("read", "a.rs", 4, Some(7)).unwrap();
    // 流式：未满行不 \r 重绘
    r.text_delta("半行").unwrap();
    r.text_delta("继续").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert_eq!(text, "", "非交互下一切暂态静默：{text:?}");
    assert_no_forbidden_ansi(text.as_bytes());
    // 满行才落屏：未满行入缓冲，遇 \n 落定
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    r.text_delta("半行").unwrap();
    r.text_delta("凑满\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert_eq!(text, "半行凑满\n", "非交互流式=满行才落屏：{text:?}");
}

/// 审查 P1-1 回归：非交互（管道/TERM=dumb）下 LLM 回复的最后一段（未满行）
/// 不得丢失——turn_done 收尾时先补输出未满行再落定换行。
#[test]
fn 非交互终态不丢字_尾段落定() {
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    r.text_delta("开头完整行\n").unwrap();
    r.text_delta("尾段").unwrap(); // 未满行：非交互下未上屏
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(1200),
        prompt_tokens: None,
        completion_tokens: None,
        context_tokens: None,
        context_window: None, // task-27
        model: "z-ai/glm-5.3-flash".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("开头完整行\n"), "完整行应已落定：{text:?}");
    assert!(text.contains("尾段\n"), "未满行应由收尾补落定（不丢字）：{text:?}");
    assert_no_forbidden_ansi(text.as_bytes());

    // 同链路思考块（任务 30-② Collapse）：非交互下活动行静默（暂态零输出）。
    // 碎思考（任务30 三期⑥，4 字<20 阈值）：非交互同口径整体吞，零落定行
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    r.reasoning_delta("思考尾段").unwrap();
    r.finish().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert_eq!(text, "", "碎思考非交互整体吞（零输出）：{text:?}");

    // 达标思考（≥20 字）：非交互冻结单行照出（落定行非暂态）；
    // 原文不上屏，字数计数不丢
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    r.set_width(40); // T3-a：预览按列宽预算（40-20=20 列=10 CJK 字），截断确定性注入
    r.reasoning_delta("一段足够长的思考内容在非交互模式下也要冻结计数").unwrap();
    r.finish().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(!text.contains("非交互模式下也要冻结计数"), "预览外原文不上屏：{text:?}");
    assert!(text.contains('▏') && text.contains(" 想了 ") && text.contains("· 一段足够长的思考内…"), "非交互冻结单行照出（T3-a 列宽预算预览截断 9 字+…；ANSI 中缀允许）：{text:?}");
    assert!(text.ends_with('\n'), "收尾应落定换行：{text:?}");

    // 逃生口（YUNYOU_THINKING=full）：非交互下全文仍收尾补落（内容不丢语义保留）
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    r.set_thinking(yunyou_cli::render::ThinkingMode::Full);
    r.reasoning_delta("思考尾段").unwrap();
    r.finish().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("思考尾段"), "Full 模式思考未满行应收尾补落（ANSI 中缀允许）：{text:?}");
    assert!(text.contains('▏'), "Full 模式思考行应 ▏ 前缀（task-46①）：{text:?}");
    assert!(text.ends_with('\n'), "收尾应落定换行：{text:?}");
}

/// 审查 P1-2 回归：回合收尾 finish()——落定未满行（非交互补输出）并清活动行。
#[test]
fn 收尾_finish_落定未满行并清活动行() {
    // 非交互：补输出未满行后落定，无活动行控制符（\r\e[K）混入
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    r.text_delta("尾段").unwrap();
    r.finish().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert_eq!(text, "尾段\n", "非交互收尾应补落未满行：{text:?}");
    assert_no_forbidden_ansi(text.as_bytes());

    // 交互：未满行落定 + 活动行清理（收尾清场）
    let mut r = full();
    r.text_delta("尾段").unwrap();
    r.finish().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("尾段\n"), "交互收尾应落定未满行：{text:?}");
    assert!(text.ends_with("\r\x1b[K"), "交互收尾应清活动行：{text:?}");
}

/// Profile::detect_from 注入式环境矩阵（审查契约 P3：不碰真实进程 env）。
#[test]
fn 降级档位_注入式环境矩阵() {
    let g = |pairs: &[(&str, &str)]| {
        let owned: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |key: &str| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };
    let none = g(&[]);
    assert_eq!(Profile::detect_from(true, &none), Profile::Full, "TTY 无开关 = 全能力");
    assert_eq!(Profile::detect_from(false, &none), Profile::NoColor, "非 TTY 剥色");
    assert_eq!(Profile::detect_from(true, &g(&[("NO_COLOR", "1")])), Profile::NoColor);
    assert_eq!(Profile::detect_from(true, &g(&[("YUNYOU_PLAIN", "1")])), Profile::Plain);
    assert_eq!(Profile::detect_from(true, &g(&[("TERM", "dumb")])), Profile::Plain, "TERM=dumb 同 Plain");
    // 优先级：PLAIN 压过 NO_COLOR
    assert_eq!(
        Profile::detect_from(true, &g(&[("YUNYOU_PLAIN", "1"), ("NO_COLOR", "1")])),
        Profile::Plain
    );
    // task-19②：NO_COLOR presence 语义（no-color.org 惯例）——存在且非空即剥，
    // 不限 "1"（PTY 实证 NO_COLOR=true 曾 19 处 256 色全开）；空串=未设置不算。
    assert_eq!(Profile::detect_from(true, &g(&[("NO_COLOR", "true")])), Profile::NoColor, "任意非空值照剥");
    assert_eq!(Profile::detect_from(true, &g(&[("NO_COLOR", "0")])), Profile::NoColor, "presence 语义：0 也生效");
    assert_eq!(Profile::detect_from(true, &g(&[("NO_COLOR", "")])), Profile::Full, "空串=未设置不生效");
    assert_eq!(Profile::detect_from(false, &g(&[("NO_COLOR", "true")])), Profile::NoColor, "非 TTY 与 NO_COLOR 同档不叠加");
}

// 定宽 80 列回归（D-UI-1 缺陷本体）见 tests/render_width.rs（按场景拆出）。

/// 渲染层常量与 tokens 单源一致性抽查（palette 由 build.rs 生成）。
#[test]
fn 色板_来自_tokens_单源() {
    assert_eq!(terminal::SGR_BRAND_PRIMARY_256, "38;5;67", "云青 67（tokens.json adapters.cli）");
    assert_eq!(terminal::SGR_FEEDBACK_SUCCESS_256, "38;5;71");
    assert_eq!(terminal::SPINNER_INTERVAL_MS, 80);
    assert_eq!(terminal::SPINNER_FRAMES[2], "⠹");
    assert_eq!(terminal::ascii::SUCCESS, "v");
    assert_eq!(terminal::ascii::FENCE_DOT, ".", "task-35②：fence.dot ASCII 映射（D3 声明落地单点）");
}

/// task-35② 回归锁一：Plain 档等待帧零非 ASCII——省略号走 SYM_FOLD（…→...）、
/// 间隔符走 SYM_FENCE_DOT（·→.），em dash/省略号/间隔点残留即违约（§3 整体退化）。
#[test]
fn waiting帧_plain_零非ascii() {
    let mut r = plain();
    r.waiting_frame(std::time::Duration::from_secs(3)).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(
        text.contains("... 等待回复 (3s . Ctrl-C 中断)"),
        "Plain 等待帧应为 ASCII 降级形态：{text:?}"
    );
    assert!(!text.contains('\u{2026}'), "Plain 等待帧不得残留 …：{text:?}");
    assert!(!text.contains('\u{b7}'), "Plain 等待帧不得残留 ·：{text:?}");
}

/// task-35② 回归锁二：Plain 档落款 token 缺省出 ASCII「--」——em dash（U+2014）
/// 非 ANSI 亦非 ASCII，Plain 契约「整体退化为 ASCII」残留即违约（P2-3 点位一）。
#[test]
fn 落款_plain_token缺省_出ascii双横线() {
    let mut r = plain();
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(3200),
        prompt_tokens: None,
        completion_tokens: None,
        context_tokens: None,
        context_window: None, // task-27 融合补位
        model: "m".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("↑-- ↓--"), "Plain 落款 token 缺省应出 --：{text:?}");
    assert!(!text.contains('\u{2014}'), "Plain 落款不得残留 em dash：{text:?}");
}

