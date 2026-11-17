//! 横幅/提示符族（§4.1 大字版/字模快照/窄屏降级/纯文本档/凭据提示行/提示符符号）
//! （自 tests/render.rs 拆出，task-31 场景族分组；纯搬移零变更，helper 经 super 复用。）

use super::*;

/// §4.1 横幅大字版（任务30 二期①，设计稿定稿）：块艺术 LOGO 行级渐变
/// （上 67 中 110 下 74）、品牌/信息/Tips 结构、逐行落定零重绘、未上线命令零宣称。
#[test]
fn 横幅_大字版_宽终端() {
    let mut r = full();
    r.set_width(80);
    r.banner("z-ai/glm-5.3-flash", "/tmp/sessions/default.jsonl", None, true).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 11, "大字版 B 案 11 行=LOGO3+空+品牌1+信息2+空+Tips3：{text:?}");
    assert!(lines[0].contains('█') && lines[0].contains("\x1b[38;5;67m"), "LOGO 上段块字+主色 67：{}", lines[0]);
    assert!(lines[1].contains("\x1b[38;5;110m"), "LOGO 中段 110：{}", lines[1]);
    assert!(lines[2].contains("\x1b[38;5;74m"), "LOGO 下段亮青 74：{}", lines[2]);
    assert!(lines[4].contains("云悠 yunyou"), "品牌行（B 案精简无☁）：{}", lines[4]);
    assert!(lines[5].contains("模型 z-ai/glm-5.3-flash"), "模型行独立（拆行防窄屏折行）：{}", lines[5]);
    assert!(lines[6].contains("会话 /tmp/sessions/default.jsonl"), "会话行独立：{}", lines[6]);
    assert!(lines[8].contains("直接说需求"), "Tips 一：{}", lines[8]);
    assert!(lines[9].contains("会话自动落盘"), "Tips 二：{}", lines[9]);
    assert!(lines[10].contains("/quit") && lines[10].contains("Ctrl-C"), "Tips 三（退出可承诺）：{}", lines[10]);
    assert!(!text.contains("/help"), "未上线命令不得宣称：{text:?}");
    assert!(!text.contains('\r') && !text.contains("\x1b[K"), "横幅逐行落定零重绘（§5 不闪不动）：{text:?}");
    assert_no_forbidden_ansi(text.as_bytes());
}

/// LOGO 字模快照（task-69 B 案）：三行 27 列**独立字面**锁死——
/// 与 banner.rs 常量物理分离（测试引用常量则常量漂移测试同漂，失去锁的
/// 意义）；唯一底稿=docs/设计/演示/task-69-样稿/B-Full-100.txt 真字节。
#[test]
fn 横幅_logo字模快照_三行独立字面() {
    const SNAPSHOT: [&str; 3] = [
        "█▀█ █  █ █ ▀ █ █▀█ ▄▄▄ █  █",
        "█ █ █  █ ██ █ █▀▄ █   █ ██",
        "▀ ▀ ▀▀▀ █  █  ▀█▀  ▀▀▀ ▀██",
    ];
    let mut r = full();
    r.set_width(80);
    r.banner("z-ai/glm-5.3-flash", "/tmp/sessions/default.jsonl", None, true).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let clean = strip_ansi(&text);
    let lines: Vec<&str> = clean.lines().collect();
    for (idx, row) in SNAPSHOT.iter().enumerate() {
        assert_eq!(
            lines[idx].trim_end(),
            format!("  {row}").trim_end(),
            "LOGO 第 {} 行快照漂移（task-69 B 案防改线）：{}",
            idx + 1,
            lines[idx]
        );
    }
    // B 案三行宽实测锁死（27/26/27 列）——底稿真字节等宽
    let expect_widths = [27usize, 26, 26];
    for (idx, row) in SNAPSHOT.iter().enumerate() {
        assert_eq!(
            yunyou_cli::render::tool::visible_width(row),
            expect_widths[idx],
            "LOGO 第 {} 行宽漂移：{:?}",
            idx + 1,
            row
        );
    }
}

/// §4.1 横幅窄屏降级（设计稿降级矩阵）：列宽 <40 → 单行版三行，块字符不出。
#[test]
fn 横幅_窄屏降级_单行版() {
    let mut r = full();
    r.set_width(39);
    r.banner("z-ai/glm-5.3-flash", "/tmp/sessions/default.jsonl", None, true).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "单行版 3 行：{text:?}");
    assert!(lines[0].contains('☁') && lines[0].contains("云悠 yunyou"));
    assert!(lines[1].contains("模型 z-ai/glm-5.3-flash"));
    // 任务30 四期④：单行版与大字版 Tips 同句式同源（「Ctrl-C 退出 · /quit 退出」结巴句并句）
    assert!(lines[2].contains("/quit 或 Ctrl-C 退出"), "单行版退出指引与大字版 Tips 同句式：{}", lines[2]);
    assert!(!text.contains("Ctrl-C 退出 ·"), "结巴句不得残留：{text:?}");
    assert!(!text.contains('█'), "窄屏不出块字 LOGO：{text:?}");
    assert_no_forbidden_ansi(text.as_bytes());
}

/// §4.1 横幅纯文本档降级：即使 80 列也走单行版（块字符 █▀▄ 在纯文本档契约下不可靠）。
#[test]
fn 横幅_纯文本档_单行版() {
    let mut r = plain();
    r.set_width(80);
    r.banner("z-ai/glm-5.3-flash", "/tmp/sessions/default.jsonl", None, true).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert_eq!(text.lines().count(), 3, "Plain 档单行版 3 行：{text:?}");
    assert!(!text.contains('█'), "纯文本档不出块字符：{text:?}");
    assert_no_forbidden_ansi(text.as_bytes());
}

/// §4.2 提示符：❯ 云青。
#[test]
fn 提示符_云青_符号() {
    let mut r = full();
    r.prompt().unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    assert!(text.contains("\x1b[38;5;67m❯"), "提示符 ❯ 应为云青：{text:?}");
}

/// task-34②：凭据缺失提示行条件渲染——Some 在大字版（Tips 第 4 行）与单行版
/// （第 4 行）各追加一行，None 零增量；提示行同 bullet+dim 形态、逐行落定零重绘。
#[test]
fn banner_凭据提示行条件渲染() {
    const NOTICE: &str = "API key 未配置：在 yunyou.toml 写 api_key（详见 --help）";
    // 大字版（task-69 B 案 3 行 LOGO）：11 行 + 提示 1 行 = 12 行，且落在 Tips 之后
    let mut r = full();
    r.set_width(80);
    r.banner("z-ai/glm-5.3-flash", "/tmp/sessions/default.jsonl", Some(NOTICE), true).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 12, "大字版 11 行+提示 1 行：{text:?}");
    assert!(lines[11].contains(NOTICE), "提示行应含文案：{}", lines[11]);
    assert!(lines[10].contains("/quit"), "提示行应在 Tips 三之后：{}", lines[10]);
    assert!(!text.contains('\r') && !text.contains("\x1b[K"), "提示行零重绘（§5 不闪不动）：{text:?}");
    // 单行版（<40 列）：3 行 + 提示 1 行 = 4 行；窄幅截断护栏生效（宁截断不折行，头部保留）
    let mut r = plain();
    r.set_width(39);
    r.banner("m", "/tmp/s", Some(NOTICE), true).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "单行版 3 行+提示 1 行：{text:?}");
    assert!(lines[3].contains("API key 未配置"), "单行版提示行头部保留：{}", lines[3]);
    assert!(
        yunyou_cli::render::tool::visible_width(lines[3]) <= 39,
        "窄幅截断到列宽内：{}", lines[3]
    );
    // None：两版均零增量（大字版 B 案 11 行、单行版 3 行）
    let mut r = full();
    r.set_width(80);
    r.banner("z-ai/glm-5.3-flash", "/tmp/sessions/default.jsonl", None, true).unwrap();
    assert_eq!(String::from_utf8(r.into_output()).unwrap().lines().count(), 11);
    // 截断护栏：超长提示（含长路径）宁截断不折行——可见宽 ≤ 列宽
    let long_notice = format!("API key 未配置：在 {}.yunyou/yunyou.toml 写 api_key（详见 --help）", "x".repeat(120));
    let mut r = full();
    r.set_width(80);
    r.banner("m", "/tmp/s", Some(&long_notice), true).unwrap();
    let text = String::from_utf8(r.into_output()).unwrap();
    let last = text.lines().last().unwrap();
    assert!(
        yunyou_cli::render::tool::visible_width(last) <= 80,
        "超长提示应截断到列宽内：{last:?}"
    );
}

