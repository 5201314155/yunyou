//! 行内层与 markdown 行级渲染族（§4.5 行内/标题 H4 兜底）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// 行内同定界符配对（任务30 四期⑦）：`*a_b_c*` 应整体成斜体（先搜同符——
/// 旧实现首遇异符定界符即判不配对整段原样）；异符不配对防跨样式粘连保留。
#[test]
fn 行内_同定界符嵌异符_整体配对() {
    let mut r = full();
    r.text_delta("强调 *a_b_c* 与 _p q r_ 以及不闭合 *ab\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    assert!(text.contains("\x1b[3ma_b_c\x1b[0m"), "*a_b_c* 应整体成斜体（SGR 查原始流）：{text:?}");
    assert!(text.contains("\x1b[3mp q r\x1b[0m"), "_p q r_ 应整体成斜体：{text:?}");
    assert!(clean.contains("a_b_c") && !clean.contains("*a_b_c*"), "星号定界符应去除：{clean:?}");
    assert!(clean.contains("不闭合 *ab"), "未闭合原样（fail-safe 不变）：{clean:?}");
}

/// §4.5 markdown 行级：H1 粗体云青、列表 •、引用 │、行内代码淡云青。
#[test]
fn markdown_行级渲染() {
    let mut r = full();
    r.text_delta("# 标题一\n").unwrap();
    r.text_delta("- 项目甲\n").unwrap();
    r.text_delta("> 引用一句\n").unwrap();
    r.text_delta("用 `epoch` 表达\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("\x1b[38;5;67m"), "H1 应云青");
    assert!(text.contains("•") && text.contains("项目甲"), "列表符应为 •（reset 中缀允许）：{text:?}");
    assert!(text.contains("│") && text.contains("引用一句"), "引用应 │ 前缀（reset 中缀允许）：{text:?}");
    assert!(text.contains("\x1b[38;5;110mepoch\x1b[0m"), "行内代码应淡云青 110");
}

/// task-51④：H4+ 兜底——剥全部前导 #，按 H3 同款云青渲染。
#[test]
fn 标题_h4兜底_剥全部井号() {
    let mut r = full();
    r.text_delta("#### 四级标题\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    assert!(clean.contains("四级标题"), "标题文本保留：{clean:?}");
    assert!(!clean.contains("####"), "前导 # 全剥（报告3 Bug-4）：{clean:?}");
    assert!(text.contains("\x1b[38;5;67m"), "H3 同款云青：{text:?}");
}

