//! 工具卡与树状前缀族（§4.6-4.8 进行中帧/三态/失败原因/树缩进）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// §4.6/4.7 工具卡：进行中帧（⠹+工具名+摘要）→ 完成卡（✓ 苔绿）。
#[test]
fn 工具卡_进行中到完成() {
    let mut r = full();
    let call = TurnEvent::ToolCall {
        call_id: "c1".into(),
        name: "read".into(),
        summary: "crates/yunyou/src/fiber.rs:120-145".into(),
        parent_tool_call_id: None,
        batch_id: None,
    };
    r.tool_running_frame(&call, 2, 3).unwrap(); // 第 3 帧 = ⠹（mock 帧 5 同款）；耗时 3s
    let running = String::from_utf8(r.into_output()).unwrap();
    assert!(running.contains("\r\x1b[K  \x1b[38;5;74m⠹"), "进行中行应 spinner 起头：{running:?}");
    assert!(running.contains("\x1b[1m\x1b[38;5;66mread\x1b[0m"), "工具名应粗体+青灰分色（tool.read）：{running:?}");
    assert!(running.contains("\x1b[2m·\x1b[0m crates/yunyou/src/fiber.rs:120-145 · 3s"), "摘要正文色（⑧D1 摘要不叠 dim）+· 结构符烟灰：{running:?}");

    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c1".into(),
        name: "read".into(),
        ok: true,
        summary: "26 行".into(),
        detail: None,
        parent_tool_call_id: None,
    })
    .unwrap();
    let done = String::from_utf8(r.into_output()).unwrap();
    assert!(done.contains("\r\x1b[K  \x1b[38;5;71m✓"), "完成卡应 ✓ 苔绿：{done:?}");
    assert!(done.contains("\x1b[1m\x1b[38;5;66mread\x1b[0m"), "工具名应粗体+青灰分色（read 类，task-47①）");
}

/// task-47 验收双标之二：Plain 档三态（running/done/cancelled）不炸——
/// 色彩剥离、耗时数字与工具名纯文本在位（YUNYOU_PLAIN 语义）。
#[test]
fn 工具卡_plain档三态不炸() {
    let mut r = Renderer::new(Vec::new(), Profile::Plain, true);
    let call = TurnEvent::ToolCall {
        call_id: "p1".into(),
        name: "bash".into(),
        summary: "echo hi".into(),
        parent_tool_call_id: None,
        batch_id: None,
    };
    r.tool_running_frame(&call, 2, 5).unwrap();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "p1".into(),
        name: "bash".into(),
        ok: true,
        summary: "0 行".into(),
        detail: None,
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert_no_forbidden_ansi(text.as_bytes());
    assert!(text.contains("· 5s"), "耗时数字纯文本在位（Plain 退化不炸）：{text:?}");
    assert!(text.contains("bash"), "工具名纯文本在位：{text:?}");
    // cancelled 卡：独立纯函数直渲（同 Plain 档断言）
    let mut buf = Vec::new();
    yunyou_cli::render::tool::tool_cancelled(&mut buf, Profile::Plain, "read").unwrap();
    let cancelled = String::from_utf8(buf).unwrap();
    assert_no_forbidden_ansi(cancelled.as_bytes());
    assert!(cancelled.contains("read") && cancelled.contains("已取消"), "{cancelled:?}");
}

/// §4.7 失败卡：✗ 绯红 + 原因（退出码）。
#[test]
fn 工具卡_失败必须带原因() {
    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c2".into(),
        name: "bash".into(),
        ok: false,
        summary: "cargo build · 退出码 101".into(),
        detail: None,
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("\x1b[38;5;167m✗"), "失败应 ✗ 绯红 167");
    assert!(text.contains("退出码 101"), "失败应带原因");
}

/// task-51⑤：树前缀计入摘要预算——depth=6 长摘要整行不超列宽
/// （折行孤字防复发锁；一期 A 线 depth 恒 0，本测试锁的是树字段启用那天）。
#[test]
fn 树前缀_深度6_摘要预算吃紧不折行() {
    let mut r = full();
    let prefix = yunyou_cli::render::tree::indent_prefix(yunyou_cli::render::Profile::Full, 6, true);
    let event = TurnEvent::ToolResult {
        call_id: "c12".into(),
        name: "bash".into(),
        ok: true,
        summary: "这是一个非常长的摘要文本用于验证树前缀计入预算后摘要被截断而不是整行超宽折行产生孤字残行的场景".into(),
        detail: None,
        parent_tool_call_id: Some("p".into()),
    };
    r.tool_done_with_prefix(&event, &prefix).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    for line in text.lines() {
        assert!(
            yunyou_cli::render::tool::visible_width(line) <= 80,
            "树前缀行不得超列宽（报告3 Bug-5）：{line:?}"
        );
    }
}

/// 树状分支（任务 30-④）：tool_done_with_prefix 拼树缩进前缀（indent_prefix
/// 真函数组合，观察者树归集器的渲染出口；demo ⑥ 同形态）。
#[test]
fn 树状缩进_子调用前缀渲染() {
    let done = TurnEvent::ToolResult {
        call_id: "c-t1".into(),
        name: "read".into(),
        ok: true,
        summary: "a.rs · 3 行".into(),
        detail: None,
        parent_tool_call_id: Some("c-parent".into()),
    };
    let mut r = full();
    r.tool_done_with_prefix(&done, &yunyou_cli::render::tree::indent_prefix(Profile::Full, 1, false))
        .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("├─"), "非最后子调用应 ├─（树干线烟灰）：{text:?}");
    assert!(text.contains("\x1b[38;5;71m✓"), "完成卡仍应 ✓ 苔绿：{text:?}");

    let mut r = full();
    r.tool_done_with_prefix(&done, &yunyou_cli::render::tree::indent_prefix(Profile::Full, 1, true))
        .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("└─"), "最后子调用应 └─：{text:?}");
}

