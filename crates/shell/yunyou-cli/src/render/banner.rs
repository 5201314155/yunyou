//! 启动横幅视觉块（规范 §4.1：品牌仪式，一次性输出，不闪不动）。
//!
//! 双版式（任务30 二期①，定稿见 docs/设计/CLI横幅升格设计稿-任务30二期.md）：
//! - 宽终端大字版（列宽 ≥40 且非 Plain）：块艺术 LOGO + 品牌行 + 信息行 + Tips；
//! - 单行版（<40 列或 Plain 档）：三行文本——窄终端与纯文本档的**正式形态**
//!   而非劣化形态（块字符 █▀▄ 在纯文本档契约下不可靠，故整体退化）。
//!
//! 两版共用逐行落定输出形态（[`freeze_line`]）：零 `\r`、零 `\e[K`，
//! 落定即历史（§5 不闪不动不重绘，不因行数增多放宽）。

use super::terminal::{
    ascii, freeze_line, Profile, SGR_BRAND_BRIGHT_256, SGR_BRAND_PRIMARY_256,
    SGR_BRAND_SECONDARY_256, SGR_TEXT_SECONDARY_256, SYM_BRAND, SYM_BULLET, SYM_FENCE_DOT,
};
use std::io::Write;

/// 大字版起用列宽下限：LOGO 31 列 + 缩进 2 + 右侧呼吸空间——40 列以下排不下
/// 完整字形，与其挤压变形不如退单行版（设计稿降级矩阵定稿）。
const WIDE_MIN_COLS: usize = 40;

/// 块艺术 LOGO 字模（task-69 B 案定稿：3 行 27 列——唯一底稿=
/// docs/设计/演示/task-69-样稿/B-Full-100.txt 真字节，禁按记忆重排；
/// task-44③ 修形稿作废）。等宽契约由 banner 快照测试锁（三行 27 列）。
const LOGO: [&str; 3] = [
    "█▀█ █  █ █ ▀ █ █▀█ ▄▄▄ █  █",
    "█ █ █  █ ██ █ █▀▄ █   █ ██",
    "▀ ▀ ▀▀▀ █  █  ▀█▀  ▀▀▀ ▀██",
];

/// LOGO 行级渐变（上深下亮=云层透光：顶部品牌主色沉底、底部提亮衔接 Tips 区）；
/// 全部既有 SGR token（tokens.json 生成 palette 单源），零新增色号。
const LOGO_GRADIENT: [&str; 3] = [
    SGR_BRAND_PRIMARY_256,
    SGR_BRAND_SECONDARY_256,
    SGR_BRAND_BRIGHT_256,
];

/// Tips 文案（全部可承诺：/quit、Ctrl-C、会话恢复均已上线；/help 未上线
/// 不得提及——宣称不存在的命令即撒谎）。
const TIPS: [&str; 3] = ["直接说需求，云悠替你干活", "会话自动落盘，重启接着聊", "Esc 打断 · /quit 或 Ctrl-C 退出"];

/// 渲染启动横幅（模型名与会话目录来自装配配置；列宽来自 Renderer 单源）。
/// `notice`：启动提示行（task-34② 凭据缺失指路；None=零增量，不加行）。
pub fn render<W: Write>(
    out: &mut W,
    profile: Profile,
    model: &str,
    session_dir: &str,
    width: usize,
    notice: Option<&str>,
    tips: bool,
) -> std::io::Result<()> {
    if width >= WIDE_MIN_COLS && profile != Profile::Plain {
        wide(out, profile, model, session_dir, width, notice, tips)
    } else {
        single_line(out, profile, model, session_dir, width, notice, tips)
    }
}

/// 宽终端大字版：LOGO 渐变 5 行 + 品牌行 + 模型/会话行 + Tips 3 行（+提示行）。
/// `tips=false`（task-38②：--task 单任务模式自退出，退出指引是噪音）不渲染 Tips。
fn wide<W: Write>(
    out: &mut W,
    profile: Profile,
    model: &str,
    session_dir: &str,
    width: usize,
    notice: Option<&str>,
    tips: bool,
) -> std::io::Result<()> {
    let brand = profile.color(SGR_BRAND_PRIMARY_256);
    let dim = profile.color(SGR_TEXT_SECONDARY_256);
    let (reset, bold) = (profile.reset(), profile.bold());
    for (idx, row) in LOGO.iter().enumerate() {
        let rendered = format!("  {}{row}{}", profile.color(LOGO_GRADIENT[idx]), reset);
        freeze_line(out, &rendered)?;
    }
    freeze_line(out, "")?;
    // 品牌行（task-69 B 案精简版：无 ☁ 无版本号——业主选 B 即选精简）
    freeze_line(
        out,
        &format!("  {bold}{brand}云悠 yunyou{reset} {dim}—— 终端里，云替你干活{reset}"),
    )?;
    // 信息行：模型/会话拆两行——合排一行在 48 列必折行，拆行后各行全宽稳排
    freeze_line(out, &format!("    {dim}模型 {model}{reset}"))?;
    freeze_line(out, &format!("    {dim}会话 {session_dir}{reset}"))?;
    freeze_line(out, "")?;
    // Tips：bullet 云青（与正文列表同符号体系）、文本烟灰（次要信息不与正文抢）
    if tips {
        for tip in TIPS {
            freeze_line(
                out,
                &format!(
                    "    {brand}{}{reset} {dim}{tip}{reset}",
                    profile.symbol(SYM_BULLET, ascii::BULLET)
                ),
            )?;
        }
    }
    // 启动提示行（task-34②）：仅在有提示时追加，bullet+dim 与 Tips 同形态；
    // 截断护栏复用 clip_width（提示可能含长路径，宁截断不折行——§4 摘要口径）
    if let Some(text) = notice {
        freeze_line(
            out,
            &format!(
                "    {brand}{}{reset} {dim}{}{reset}",
                profile.symbol(SYM_BULLET, ascii::BULLET),
                super::tool::clip_width(text, width.saturating_sub(6)),
            ),
        )?;
    }
    Ok(())
}

/// 单行版（<40 列或 Plain 档）：三行文本原样保留（任务30 一期定稿形态；+提示行）。
/// `tips=false`（task-38②）不渲染退出指引行。
fn single_line<W: Write>(
    out: &mut W,
    profile: Profile,
    model: &str,
    session_dir: &str,
    width: usize,
    notice: Option<&str>,
    tips: bool,
) -> std::io::Result<()> {
    let brand = profile.color(SGR_BRAND_PRIMARY_256);
    let dim = profile.color(SGR_TEXT_SECONDARY_256);
    let (reset, bold) = (profile.reset(), profile.bold());
    let cloud = profile.symbol(SYM_BRAND, ascii::BRAND);
    // 第一行：display 层（task-69 B 案精简版——与 B-Full-48 样稿形态一致）
    freeze_line(
        out,
        &format!("  {brand}{cloud}{reset} {bold}{brand}云悠 yunyou{reset} {dim}—— 终端里，云替你干活{reset}"),
    )?;
    // 第二三行：caption 层（烟灰 dim）；退出指引与大字版 Tips 同句式同源
    //（任务30 四期④：「Ctrl-C 退出 · /quit 退出」结巴句并句）；task-38②：--task 模式自退出不渲染
    freeze_line(
        out,
        &format!(
            "    {dim}模型 {model} {} 会话 {session_dir}{reset}",
            profile.symbol(SYM_FENCE_DOT, ascii::FENCE_DOT) // task-35② 追加小刀：间隔符走 D3 降级映射（Plain 出 .）
        ),
    )?;
    if tips {
        freeze_line(out, &format!("    {dim}{}{reset}", TIPS[2]))?;
    }
    // 启动提示行（task-34②）：仅在有提示时追加，与大字版同文案同源（同截断护栏）
    if let Some(text) = notice {
        freeze_line(
            out,
            &format!("    {dim}{}{reset}", super::tool::clip_width(text, width.saturating_sub(4))),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task-69 B 案锁：LOGO 三行宽 27/26/27 列（唯一底稿 B-Full-100.txt 真字节）。
    #[test]
    fn logo_三行b案等宽() {
        let expect_widths = [27usize, 26, 26];
        for (idx, row) in LOGO.iter().enumerate() {
            let w = crate::render::tool::visible_width(row);
            assert_eq!(w, expect_widths[idx], "LOGO 行 {} 应等宽 B 案定稿", idx + 1);
        }
    }
}
