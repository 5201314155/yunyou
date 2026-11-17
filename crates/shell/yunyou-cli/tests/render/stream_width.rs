use super::*;


/// task-65 P1-2 字节断言：rewrite 帧不含 \n（单视觉行假设——stream_chunk
/// 尾段 pending 含 \n 时由预切逻辑逐行落定，rewrite 帧永远单行）。
#[test]
fn rewrite帧不含换行() {
    let mut r = full();
    r.set_width(80);
    r.text_delta("第一行\n第二行未满").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    // 落定段（\n 前完整行）允许 \n；rewrite 帧（\r\x1b[K 后）禁止 \n
    for frame in text.split("\r\x1b[K").skip(1) {
        assert!(
            !frame.contains('\n'),
            "rewrite 帧含换行=多视觉行假设破坏：{frame:?}"
        );
    }
}

/// task-65 P1-4 复验锁：正文首帧不被 waiting 覆盖（task-35a 正文接管——
/// ev_assistant_delta 先 stop_waiting+waiting_painted 清零再 text_delta；
/// 字节断言：正文增量输出后无 waiting spinner 残迹拼接）。
#[test]
fn 正文首帧不被waiting覆盖() {
    let mut r = full();
    // 模拟 waiting 已绘制（先 ticker 帧），再正文增量（35a 接管撤等待）
    r.tool_call_frame("bash", "运行中", 1, Some(1)).unwrap();
    r.text_delta("正文首帧").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    // 正文首帧必须出现在 waiting 清行之后（\r\x1b[K 清行在前=覆盖被撤）
    let body_pos = text.find("正文首帧").expect("正文应上屏");
    let prefix = &text[..body_pos];
    assert!(
        prefix.rfind("\r\x1b[K").is_some(),
        "正文前应有清行（waiting 被接管撤除）：{text:?}"
    );
    // 正文行内不得拼接 spinner 帧（覆盖缺失=35a 回归）
    let body_line = text[body_pos..].lines().next().unwrap();
    assert!(
        !body_line.contains("⠹") && !body_line.contains("⠋"),
        "正文首帧拼接 spinner=waiting 覆盖回归：{body_line:?}"
    );
}
