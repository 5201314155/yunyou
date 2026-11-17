//! 状态落款与等待指示族（§4.12 水位/累计语义分流/等待帧）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// task-51① 修复序列锁（本体报告3 Bug-1）：waiting 帧在屏时，收尾须先清
/// 活动行（\r\e[K）再写落款——观察者 TurnComplete 分支（A 案 waiting_painted
/// 守卫）按此序列调用渲染层；端到端字节证据见 capture/51-复验-mock空补全.typescript
/// （mock 空补全 1.5s 伪 TTY，确定性快回合）。
#[test]
fn 快回合_落款不拼接等待行尾() {
    let mut r = full();
    r.waiting_frame(std::time::Duration::from_secs(1)).unwrap();
    // 观察者 A 案序列：painted → clear_active_line → turn_done
    {
        let (out, _) = r.parts();
        yunyou_cli::render::terminal::clear_active_line(out, true).unwrap();
    }
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_secs(1),
        prompt_tokens: Some(10),
        completion_tokens: Some(2),
        context_tokens: None,
        context_window: None, // task-27
        model: "m".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(
        text.contains("中断)\x1b[0m\r\x1b[K"),
        "落款前应有清行（Bug-1 修复锁，task-51①）：{text:?}"
    );
    assert!(!text.contains("中断)  ── "), "落款拼接等待行尾即 Bug-1 本体：{text:?}");
}

/// §4.12 状态落款：烟灰随流、耗时与 token 计数格式。
#[test]
fn 状态落款_随流滚动() {
    let mut r = full();
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(3200),
        prompt_tokens: Some(1204),
        completion_tokens: Some(8931),
        context_tokens: None,
        context_window: None, // task-27
        model: "z-ai/glm-5.3-flash".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("3.2s"), "耗时格式：{text:?}");
    assert!(text.contains("本回合 3.2s"), "「本回合」与耗时之间应有空格（任务 30-⑥）：{text:?}");
    assert!(text.contains("↑1,204"), "千分位 ↑（任务30 二期④紧凑：单位由箭头语义承担）");
    assert!(text.contains("↓8,931"), "千分位 ↓");
    assert!(text.contains("☁"), "落款应带品牌符");
}

/// task-35a：等待指示单帧——交互档 \r\e[K 原位重绘+秒数进文案；非交互零字节
/// （静态提示由 waiting_notice 承担，单帧不刷管道）。
#[test]
fn 等待指示_单帧与交互门控() {
    let mut r = full();
    r.waiting_frame(std::time::Duration::from_secs(5)).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("\r\x1b[K"), "活动行原位重绘：{text:?}");
    assert!(text.contains("… 等待回复 (5s · Ctrl-C 中断)"), "文案+秒数：{text:?}");
    let mut r = Renderer::new(Vec::new(), Profile::Full, false);
    r.waiting_frame(std::time::Duration::from_secs(5)).unwrap();
    assert!(r.into_output().is_empty(), "非交互单帧静默");
}

/// task-35b：等待静态提示——非交互/Plain 落一行零 ANSI；交互档零输出（ticker 接管）。
#[test]
fn 等待静态提示_非交互落行_交互静默() {
    let mut r = Renderer::new(Vec::new(), Profile::Plain, false);
    r.waiting_notice().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("（等待回复中，Ctrl-C 可中断）"), "{text:?}");
    assert!(!text.contains('\x1b'), "Plain 档零 ANSI：{text:?}");
    let mut r = full();
    r.waiting_notice().unwrap();
    assert!(r.into_output().is_empty(), "交互档零输出（ticker 接管）");
}

/// task-26：落款语义分流——水位主位（context_tokens=最后一步 usage.prompt，
/// 端点侧测量的当前上下文）+累计明示次位（业主实证：累计 1,725,916 被误读
/// 成当前上下文——「本回合」标签配累计数字系双重撒谎）。
#[test]
fn 落款_水位主位与累计明示次位() {
    let mut r = full();
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(3200),
        prompt_tokens: Some(1_725_916),
        completion_tokens: Some(12_307),
        context_tokens: Some(84_232),
        context_window: None, // task-27
        model: "m".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let ctx_pos = text.find("上下文 84,232 tok").expect("水位主位（端点侧测量）: {text:?}");
    let acc_pos = text
        .find("（累计 ↑1,725,916 ↓12,307）")
        .expect("累计明示次位（显式「累计」标签）: {text:?}");
    assert!(ctx_pos < acc_pos, "水位应先于累计（主位/次位序）: {text:?}");
}

/// task-26：usage 未报（context_tokens=None）→ 只出「累计」明示段——
/// 诚实：没数据不假装有水位（旧形态「本回合 ↑累计」的撒谎标签不再出现）。
#[test]
fn 落款_usage未报只出累计明示() {
    let mut r = full();
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(3200),
        prompt_tokens: Some(1_725_916),
        completion_tokens: Some(12_307),
        context_tokens: None,
        context_window: None, // task-27
        model: "m".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("累计 ↑1,725,916 ↓12,307"), "累计明示标签: {text:?}");
    assert!(!text.contains("上下文"), "无水位数据不得假装有水位: {text:?}");
}

/// task-26：窄屏（<60 列）一律省 usage 段（水位与累计都不出）——48 列预算下
/// 「累计 」标签或水位段都会挤掉模型名全名（render_width「落款行_80与48_恒单
/// 视觉行」既有定谳：模型名优先）；不显示≠撒谎显示（诚实性不受损）。
#[test]
fn 落款_窄屏省usage段保模型名() {
    let mut r = full();
    r.set_width(48);
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: std::time::Duration::from_millis(3200),
        prompt_tokens: Some(1_725_916),
        completion_tokens: Some(12_307),
        context_tokens: Some(84_232),
        context_window: None, // task-27
        model: "z-ai/glm-5.3-flash".into(),
    })
    .unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("z-ai/glm-5.3-flash"), "窄屏模型名全名优先: {text:?}");
    assert!(!text.contains("上下文") && !text.contains("累计"), "窄屏省 usage 段: {text:?}");
}
