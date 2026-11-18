//! 详情展开块族（§4.9-4.11 bash/read/diff）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// §4.9 bash 详情：非零退出 → stderr 尾部 ≤5 行绯红；退出 0 → 折叠阈值
/// 边界（任务30 二期②立、三期③ 8→10 且 bash/read 两处统一：≤10 行全展，
/// 11 行起折叠摘要——两例锁死单源阈值）。
#[test]
fn bash_详情_非零退出与阈值折叠() {
    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c3".into(),
        name: "bash".into(),
        ok: false,
        summary: "cargo build · 退出码 101".into(),
        detail: Some(DetailKind::Bash {
            exit_code: Some(101),
            output: "line1\nline2\nerror[E0432]: unresolved import".into(),
        }),
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("\x1b[38;5;167merror[E0432]"), "stderr 尾行应绯红：{text:?}");

    // 边界 10：≤10 行全展（旧版 8 行即折叠——小输出折叠反效率，三期③升 10）
    let ten = (1..=10usize).map(|i| format!("第{i}行")).collect::<Vec<_>>().join("\n");
    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c4".into(),
        name: "bash".into(),
        ok: true,
        summary: "ls".into(),
        detail: Some(DetailKind::Bash { exit_code: Some(0), output: ten }),
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("第10行"), "10 行应全展到底：{text:?}");
    assert!(!text.contains("折叠 +"), "≤10 行不折叠：{text:?}");
    assert!(text.contains("▎"), "详情行应带左缘束线（task-46③）：{text:?}");

    // 边界 11：>10 行折叠但**必带前 10 行预览**（任务30 四期 D-A 业主实锤：
    // 零预览折叠让用户无从判断工具动了什么）
    let eleven = (1..=11usize).map(|i| format!("第{i}行")).collect::<Vec<_>>().join("\n");
    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c4b".into(),
        name: "bash".into(),
        ok: true,
        summary: "ls".into(),
        detail: Some(DetailKind::Bash { exit_code: Some(0), output: eleven }),
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    for i in 1..=10usize {
        assert!(text.contains(&format!("第{i}行")), "预览应含第 {i} 行（D-A 折叠必带预览）：{text:?}");
    }
    assert!(!text.contains("第11行"), "预览外折叠：{text:?}");
    assert!(strip_ansi(&text).contains("… 折叠 +1 行"), "折叠句式单源（task-46④，ANSI 中缀剥除后判）：{text:?}");
    assert!(strip_ansi(&text).contains("▎"), "详情行应带左缘束线（task-46③）：{text:?}");

    // D-A 预览上限：12 行 → 恰好前 10 行可见 + 折叠 2（min(行数,10) 单源）
    let twelve = (1..=12usize).map(|i| format!("第{i}行")).collect::<Vec<_>>().join("\n");
    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c4c".into(),
        name: "bash".into(),
        ok: true,
        summary: "ls".into(),
        detail: Some(DetailKind::Bash { exit_code: Some(0), output: twelve }),
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("第10行") && !text.contains("第11行"), "预览恒 10 行：{text:?}");
    assert!(strip_ansi(&text).contains("… 折叠 +2 行"), "折叠计数=min(行数,10) 外的行数：{text:?}");
}

/// §4.10 文件读取：>10 行（任务30 三期③阈值单源统一）→ 首 3 行 + … 折叠；行号烟灰。
#[test]
fn read_详情_折叠规则() {
    let lines: Vec<String> = (0..26).map(|i| format!("第{i}行")).collect();
    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c5".into(),
        name: "read".into(),
        ok: true,
        summary: "fiber.rs · 26 行".into(),
        detail: Some(DetailKind::Read { path: "fiber.rs".into(), lines, from: 120 }),
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("120") && text.contains("第0行") && text.contains("│"), "首行应带行号预览：{text:?}");
    assert!(text.contains("第9行"), "预览应拉齐 10 行（任务30 四期 D-A）：{text:?}");
    assert!(!text.contains("第10行"), "预览外折叠：{text:?}");
    assert!(strip_ansi(&text).contains("… 折叠 +16 行"), "折叠提示应准确（task-46④ 句式单源）：{text:?}");
    assert!(strip_ansi(&text).contains("▎"), "read 详情行应带左缘束线（task-46③）：{text:?}");
}

/// task-46③ Plain 档：详情束线 ASCII `|` 降级（降级矩阵不回退的 ③ 侧证据）。
/// interactive=false：非交互无活动行清理，Plain 输出零 ANSI 可断言。
#[test]
fn bash_详情_plain档_束线出竖线() {
    let mut r = Renderer::new(Vec::new(), Profile::Plain, false);
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c9".into(),
        name: "bash".into(),
        ok: true,
        summary: "ls".into(),
        detail: Some(DetailKind::Bash { exit_code: Some(0), output: "第1行\n第2行".into() }),
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains('|'), "Plain 束线应出 ASCII 竖线：{text:?}");
    assert!(!text.contains('\x1b'), "Plain 档零 ANSI：{text:?}");
}

/// §4.11 diff：+ 苔绿 / − 绯红 / 上下文 ⋮ 折叠。
#[test]
fn diff_详情_着色与折叠() {
    let mut r = full();
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "c6".into(),
        name: "edit".into(),
        ok: true,
        summary: "fiber.rs · +1 −1".into(),
        detail: Some(DetailKind::Diff {
            added: 1,
            removed: 1,
            hunks: vec![DiffHunk {
                path: None,
                old_start: None,
                added: vec!["let x = recompute();".into()],
                removed: vec!["let x = old();".into()],
                context_lines: 2,
            }],
        }),
        parent_tool_call_id: None,
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    // task-45③（规范 §4.11 定谳）：diff hunk 上下文行 = 烟灰 ⋮ + 束线连续
    // （区别于 §4.9/4.10 通用折叠的秋香黄 …；演示壳 mock 同用 S_CONT=⋮）。
    assert!(
        text.contains("\x1b[2m⋮ 上下文 2 行\x1b[0m"),
        "上下文提示应为规范 ⋮ 形态（dim 跨度）：{text:?}"
    );
    assert!(text.contains("\x1b[2m▎\x1b[0m"), "diff 抽屉束线应连续：{text:?}");
    assert!(text.contains("\x1b[38;5;167m−"), "删行应绯红 −");
    assert!(text.contains("\x1b[38;5;71m+"), "增行应苔绿 +");
    // 批 5 行内高亮契约：增行拆「公共前缀+bold 变更段+后缀」三段 SGR——内容保留性按剥 SGR 后验证（字面 contains 会被行内转义隔断=旧单段契约）
    let clean = strip_ansi(&text);
    assert!(clean.contains("let x = recompute();"), "增行内容应保留（剥 SGR）：{clean:?}");
    assert!(clean.contains("let x = old();"), "删行内容应保留（剥 SGR）：{clean:?}");
}


/// task-65 P2 行宽预算：超长行截断不折行（rail 行序列对齐保持；完整内容
/// 在会话 JSONL 可回查）。
#[test]
fn 超长行截断不折行() {
    let mut r = full();
    let long = "字".repeat(120); // 240 显示列 >> 80 终端宽
    r.text_delta(&format!("```bash\n{}\n```\n", long)).unwrap();
    let out = String::from_utf8(r.into_output()).unwrap();
    // 每行字符数上限代理断言（折行使 rail 行数暴涨=对齐破坏）
    let total = out.lines().count();
    assert!(total <= 30, "rail 行折行：{total} 行\n{out:?}");
}
