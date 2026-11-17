//! 思考流收敛族（§4.5 活动行计数/碎思考吞/达标时间序/Full 逃生口）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// 思考流收敛（T3-a 大改，设计稿件三 2026-11-17）：原文不上屏，流式中活动行
/// 「▏ 思考中… Ns」计时原位重绘（字数计数退役），块终冻结「▏ 想了 X.Xs ·
/// {列宽预算首句预览}」。
#[test]
fn 思考收敛_活动行计时_块终冻结单行() {
    let mut r = full(); // interactive=true：活动行重绘生效
    r.set_width(100); // 列宽显式注入（防环境 ioctl/COLUMNS 漂移；预算=100-20=80）
    // 思考文本 ≥20 字（任务30 三期⑥碎思考阈值之上，冻结路径本体）
    r.reasoning_delta("先把装配序列与依赖图在脑内过一遍").unwrap();
    r.reasoning_delta("再回答。").unwrap();
    r.text_delta("正文来了\n").unwrap(); // 块切换 → 冻结思考单行
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("思考中… 0s"), "首增量应即绘活动行（秒数计时，0 秒起）：{text:?}");
    // 任务30 三期⑤节流（200ms）：窗口内第二增量不重绘——活动行帧恒一帧
    assert_eq!(text.matches("思考中… 0s").count(), 1, "节流窗口内不得重绘（活动行单帧）：{text:?}");
    assert!(text.contains('▏') && text.contains(" 想了 "), "块终应冻结耗时+首句单行（task-45 前缀 ▏对齐 46①；ANSI 中缀允许）：{text:?}");
    // T3-a：预览按列宽预算（80 可见列）——19 字（38 列）全文在预算内，
    // 全句照出不截断（旧行为 22 列硬顶已退役）
    assert!(text.contains("· 先把装配序列与依赖图在脑内过一遍再回答。"), "冻结行应含列宽预算内全句预览：{text:?}");
    assert!(text.contains("\x1b[2m"), "思考行应 dim");
    assert!(text.contains('▏'), "思考行应 ▏ 前缀（task-45 对齐 46① 行级拆分）");
}

/// 碎思考吞（任务30 三期⑥）：mid-text 推理碎屑（1 字）不落冻结行、
/// 正文缓冲保留无缝续拼——正文拦腰截断根除（业主实测病灶）。
#[test]
fn 碎思考_吞_正文无缝续拼() {
    let mut r = full();
    r.text_delta("前半句").unwrap();
    r.reasoning_delta("想").unwrap(); // 1 字推理碎屑
    r.text_delta("后半句\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    assert!(clean.ends_with("前半句后半句\n"), "正文应无缝续拼一次落定：{clean:?}");
    assert!(!clean.contains("想了"), "碎思考不得落冻结行：{clean:?}");
    // 暂态帧字节在流中存在但未落定（帧尾无 \n）；落定行只有正文拼接结果
    assert!(!clean.contains("思考中… 0s\n"), "碎屑帧不得落定成行（T3-a 活动行计时形态）：{clean:?}");
}

/// 达标思考 mid-text（任务30 三期⑥）：≥20 字正文先补落再冻结思考行
/// （时间序：前半 → 想了 → 后半），思考原文不上屏。
#[test]
fn 达标思考_midtext_正文先落时间序() {
    let mut r = full();
    r.set_width(40); // 预算=40-20=20 可见列（clip 按 19 列切=9 CJK 字+…；截断路径确定性注入）
    r.text_delta("前半").unwrap();
    r.reasoning_delta("这是一段足够长的思考内容超过阈值二十字了").unwrap();
    r.text_delta("后半\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    let front = clean.find("前半").unwrap();
    let think = clean.find(" 想了 ").unwrap();
    let back = clean.find("后半").unwrap();
    assert!(front < think && think < back, "应正文先落再冻结（时间序）：{clean:?}");
    assert!(clean.contains("· 这是一段足够长的思…"), "冻结行应含列宽预算截断预览（9 字+…，T3-a）：{clean:?}");
    assert!(!clean.contains("超过阈值二十字"), "预览外原文不上屏：{clean:?}");
}

/// 思考全文流式逃生口（任务 30-②）：YUNYOU_THINKING=full（set_thinking(Full)）
/// 保留烟灰 dim + │ 全文直出（旧行为，默认关闭）。
#[test]
fn 思考逃生口_full_全文流式() {
    let mut r = full();
    r.set_thinking(yunyou_cli::render::ThinkingMode::Full);
    r.reasoning_delta("先想一下。").unwrap();
    r.text_delta("正文来了\n").unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("（思考）"), "Full 模式块首应有提示：{text:?}");
    assert!(text.contains("▏ ") && text.contains("先想一下。"), "Full 模式思考行应 ▏ 前缀全文直出（task-46①，ANSI 中缀允许）：{text:?}");
    assert!(!text.contains("│ "), "思考行不得再与引用共用 │（task-46①）：{text:?}");
}

