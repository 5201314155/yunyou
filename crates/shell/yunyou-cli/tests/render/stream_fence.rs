//! 流式按行缓冲与代码围栏族（§4.4/4.5 混合流式/围栏状态机）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// §4.5 混合流式（任务30 三期④）：分片喂入 → 单视觉行可容纳时 \r\e[K
/// 原位重绘逐段流出（零落定行）；\n 满行清帧落定；收尾（close_block）
/// 清帧补落定——D-UI-1 禁令不破（只重绘单视觉行）。
#[test]
fn 流式_混合_单视觉行重绘_满行落定_收尾补落定() {
    // 未满行：两分片在单视觉行内 → 渐进重绘帧，零落定（无 \n），
    // 帧前缀单调扩展（后帧含前帧，非交互旧语义的「零字节」升级为帧语义）
    let mut r = full();
    r.set_width(80);
    r.text_delta("好的，先读").unwrap();
    r.text_delta(" fiber.rs 的实现。").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("\r\x1b[K"), "单视觉行应原位重绘：{text:?}");
    assert!(!text.contains('\n'), "未满行不落定：{text:?}");
    let frames: Vec<String> = text.split("\r\x1b[K").map(strip_ansi).collect();
    assert_eq!(frames[1], "好的，先读", "首帧应含第一分片：{frames:?}");
    assert_eq!(frames[2], "好的，先读 fiber.rs 的实现。", "次帧应含全文：{frames:?}");

    // 完整行：\n 满行清帧落定，无重绘帧混入（单 delta 直达满行）
    let mut r = full();
    r.set_width(80);
    r.text_delta("第一行\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.ends_with("第一行\n"), "完整行应落定：{text:?}");
    assert!(!text.contains('\r'), "满行直达路径无重绘帧：{text:?}");

    // 溢出：窄列宽下将溢出立即落定（多视觉行永不重绘——一次性写整行）
    let mut r = full();
    r.set_width(8);
    r.text_delta("这句中文肯定超过八列宽").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.ends_with('\n'), "将溢出应立即落定：{text:?}");
    assert!(!text.contains("\r\x1b[K"), "溢出行不得重绘（多视觉行禁令）：{text:?}");

    // 收尾：重绘过的未满行由 close_block 清帧后落定（不丢字）
    let mut r = full();
    r.set_width(80);
    r.text_delta("尾段没有换行").unwrap();
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(900),
        prompt_tokens: None,
        completion_tokens: None,
        context_tokens: None,
        context_window: None, // task-27
        model: "z-ai/glm-5.3-flash".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("\r\x1b[K尾段没有换行\n"), "收尾应清帧后落定未满行：{text:?}");
    assert_no_forbidden_ansi(text.as_bytes());
}

/// task-51③：围栏内行内标记不解释——*ptr 不斜体、反引号保留、a_b_c 原样。
#[test]
fn 围栏内_行内标记不解释() {
    let mut r = full();
    r.text_delta("```rust\nlet p = *ptr; // a_b_c 与 `x`\n```\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    assert!(clean.contains("let p = *ptr; // a_b_c 与 `x`"), "围栏内原样直出：{clean:?}");
    assert!(!text.contains("\x1b[3m"), "斜体不得误染（报告3 Bug-3）：{text:?}");
}

/// task-51③：未闭合围栏到回合尾——后半原样直出（fail-safe 可接受）。
#[test]
fn 围栏未闭合_fail_safe_原样直出() {
    let mut r = full();
    r.text_delta("```python\nprint('hi')\n").unwrap();
    r.finish().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    assert!(clean.contains("· ─── python"), "开围栏边界：{clean:?}");
    assert!(clean.contains("print('hi')"), "内容原样：{clean:?}");
}

/// task-51③：闭合后恢复 markdown 渲染（状态翻转的另一半）。
#[test]
fn 围栏闭合后_恢复markdown渲染() {
    let mut r = full();
    r.text_delta("```\ncode\n```\n**粗体正文**\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    assert!(clean.contains("code"), "围栏内原样：{clean:?}");
    assert!(text.contains("\x1b[1m"), "闭合后粗体恢复渲染：{text:?}");
    assert!(!clean.contains("**粗体正文**"), "闭合后行内层接管：{clean:?}");
}

/// task-46②：代码围栏边界——`· ───` 短横标记（信息串保留），与思考 ▏/引用 │ 不撞。
#[test]
fn 围栏行_短横边界_信息串保留() {
    let mut r = full();
    r.text_delta("```rust\nfn main() {}\n```\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    assert!(clean.contains("· ─── rust"), "开围栏应带短横+信息串：{clean:?}");
    assert!(clean.matches("· ───").count() == 2, "开闭围栏各一行：{clean:?}");
    assert!(!clean.contains("```"), "反引号标记不再直出：{clean:?}");
    assert!(clean.contains("fn main() {}"), "围栏内容原样：{clean:?}");
}

