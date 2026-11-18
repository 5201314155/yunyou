//! 启动横幅视觉块（规范 §4.1：品牌仪式，一次性输出，不闪不动）。
//!
//! **字模=位图矩阵程序化拼装（业主令 2026-11-18 02:3x/02:5x 三连定稿，本批
//! 落码候选=业主过眼实心样稿）**：①每字母一个 bool 矩阵 const（数据与渲染
//! 分离——改字母/改字号只改数据，废弃手排字符串字模）；②容器宽=Σ字模宽+
//! 字间距×2 数学推导（零硬编码魔法数）；③基线底对齐（小写 x-height 上方
//! 留空）+末行 1→▄ 出脚（渲染层处理，见 [`GLYPH_U`]/[`GLYPH_N`] descender
//! 标注）；④带色=tokens 单源三级品牌蓝行级渐变（深→亮=云层透光既定语义），
//! Plain 档整体退化（块字符不可靠契约不变）。
//!
//! 三级降级矩阵（业主拍板「大版为主+并存分级」）：
//! - ≥60 列：大版字模（Y 大写 6×7 + u/n 小写）+ 品牌行 + 信息行 + Tips；
//! - 40~59 列：紧凑版字模（5 行档）+ 同上；
//! - <40 列或 Plain：单行品牌行（正式形态而非劣化形态）。
//!
//! 两版共用逐行落定输出形态（[`freeze_line`]）：零 `\r`、零 `\e[K`，
//! 落定即历史（§5 不闪不动不重绘，不因行数增多放宽）。
//!
//! 历史注记（违规在案更正）：task-69 B 案细块字模曾于业主 gate 前先行落码
//! （前版注释自称「定稿」）——业主判「笔画稀疏看不出来」作废；本批=业主
//! 看过实心样稿后明确拍板的落码授权，gate 先行问题自此更正。

use super::terminal::{
    ascii, freeze_line, Profile, SGR_BRAND_BRIGHT_256, SGR_BRAND_PRIMARY_256,
    SGR_BRAND_SECONDARY_256, SGR_TEXT_SECONDARY_256, SYM_BRAND, SYM_BULLET, SYM_FENCE_DOT,
};
use std::io::Write;

/// 大版起用列宽下限：字模容器 22 列+品牌行全句+信息行——60 列以下退紧凑版
/// （三级降级矩阵定稿，业主 2026-11-18）。
const WIDE_MIN_COLS: usize = 60;

/// 紧凑版起用列宽下限（40~59 列档）；<40 退单行版。
const COMPACT_MIN_COLS: usize = 40;

/// 'Y' 6×7（lead 亲画示范件原样采用，业主定稿链 2026-11-18）。
const GLYPH_Y: [[u8; 6]; 7] = [
    [1, 0, 0, 0, 1, 0],
    [1, 0, 0, 0, 1, 0],
    [1, 1, 1, 1, 1, 0],
    [0, 1, 1, 1, 0, 0],
    [0, 0, 1, 0, 0, 0],
    [0, 0, 1, 0, 0, 0],
    [0, 1, 1, 1, 0, 0],
];

/// 'u' 6×7（云穆同规格自产：小写 x-height=4 行，与 Y 基线底对齐；
/// `descender=true`=矩阵末行 1 渲染为 ▄ 出脚）。
const GLYPH_U: [[u8; 6]; 4] = [
    [1, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1],
    [1, 1, 1, 1, 1, 1],
];

/// 'n' 6×7（同规格自产：顶横=肩拱+双柱，底部开放与 u 的闭合底对比可辨）。
const GLYPH_N: [[u8; 6]; 4] = [
    [1, 1, 1, 1, 1, 1],
    [1, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1],
];

/// 紧凑版 5 行档字模（y 压 5 行、u/n x-height 3 行；同拼装同降级纪律）。
const GLYPH_Y5: [[u8; 5]; 5] = [
    [1, 0, 0, 0, 1],
    [1, 1, 1, 1, 1],
    [0, 0, 1, 0, 0],
    [0, 0, 1, 0, 0],
    [0, 1, 1, 1, 0],
];
const GLYPH_U5: [[u8; 5]; 3] = [[1, 0, 0, 0, 1], [1, 0, 0, 0, 1], [1, 1, 1, 1, 1]];
const GLYPH_N5: [[u8; 5]; 3] = [[1, 1, 1, 1, 1], [1, 0, 0, 0, 1], [1, 0, 0, 0, 1]];

/// 字间距（字母间空 2 列；容器宽推导用）。
const GLYPH_GAP: usize = 2;

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

/// 矩阵抽象（const 泛型——任意 W×H bool 矩阵同容器拼装，改字号只改数据）。
trait Matrix {
    fn dims(&self) -> (usize, usize);
    fn cell(&self, j: usize, i: usize) -> u8;
}

impl<const W: usize, const H: usize> Matrix for [[u8; W]; H] {
    fn dims(&self) -> (usize, usize) { (W, H) }
    fn cell(&self, j: usize, i: usize) -> u8 { self[j][i] }
}

/// 字母档：矩阵 + 是否出脚（descender——末行 1 渲染 ▄）。
struct Glyph {
    matrix: &'static dyn Matrix,
    descender: bool,
}

/// 矩阵拼装（渲染函数只做拼装，不识字母语义）：基线底对齐（以最高字母为
/// 基准，矮字母顶部留空）+字间距 [`GLYPH_GAP`]；返回（画布行文本, 容器宽）。
/// 容器宽=Σ矩阵宽+GAP×(n−1)，数学推导零魔法数。
fn compose(letters: &[Glyph]) -> (Vec<String>, usize) {
    let height = letters.iter().map(|g| g.matrix.dims().1).max().unwrap_or(0);
    let width =
        letters.iter().map(|g| g.matrix.dims().0).sum::<usize>() + GLYPH_GAP * (letters.len() - 1);
    let mut canvas = vec![vec![' '; width]; height];
    let mut x = 0;
    for glyph in letters {
        let (w, h) = glyph.matrix.dims();
        let offset = height - h; // 底对齐
        for j in 0..h {
            for i in 0..w {
                if glyph.matrix.cell(j, i) == 1 {
                    let feet = glyph.descender && j == h - 1;
                    canvas[offset + j][x + i] = if feet { '▄' } else { '█' };
                }
            }
        }
        x += w + GLYPH_GAP;
    }
    (canvas.iter().map(|r| r.iter().collect()).collect(), width)
}

/// 矩阵行 → 带渐变落定行：整块居中（两侧留白对称，余数归右），行级三级
/// 品牌蓝渐变（深→亮），Plain 档由调用方保证不进此路径（块字符契约）。
fn logo_line(profile: Profile, row: &str, row_idx: usize, total: usize, width: usize) -> String {
    let pad = (width.saturating_sub(row.chars().count())).max(2) / 2 + 2;
    let grad = LOGO_GRADIENT[(row_idx * 3 / total.max(1)).min(2)];
    let indent = " ".repeat(pad);
    format!("{indent}{}{row}{}", profile.color(grad), profile.reset())
}

/// Tips 行（大/紧凑两版共用）。
fn tips_line(profile: Profile, tip: &str) -> String {
    format!(
        "    {}{}{} {}{}{}",
        profile.color(SGR_BRAND_PRIMARY_256),
        profile.symbol(SYM_BULLET, ascii::BULLET),
        profile.reset(),
        profile.color(SGR_TEXT_SECONDARY_256),
        tip,
        profile.reset()
    )
}

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
    if profile != Profile::Plain && width >= WIDE_MIN_COLS {
        wordmark(
            out,
            profile,
            model,
            session_dir,
            width,
            notice,
            tips,
            &[Glyph { matrix: &GLYPH_Y, descender: false }, Glyph { matrix: &GLYPH_U, descender: true }, Glyph { matrix: &GLYPH_N, descender: true }],
        )
    } else if profile != Profile::Plain && width >= COMPACT_MIN_COLS {
        wordmark(
            out,
            profile,
            model,
            session_dir,
            width,
            notice,
            tips,
            &[
                Glyph { matrix: &GLYPH_Y5, descender: false },
                Glyph { matrix: &GLYPH_U5, descender: true },
                Glyph { matrix: &GLYPH_N5, descender: true },
            ],
        )
    } else {
        single_line(out, profile, model, session_dir, width, notice, tips)
    }
}

/// 大/紧凑两版共用渲染：字模渐变行 + 空行 + 中文主品牌行（☁ 云悠 yunyou
/// 粗体云青+定位语烟灰=业主定稿 D 案形态）+ 模型/会话行 + Tips + 提示行。
fn wordmark<W: Write>(
    out: &mut W,
    profile: Profile,
    model: &str,
    session_dir: &str,
    width: usize,
    notice: Option<&str>,
    tips: bool,
    glyphs: &[Glyph],
) -> std::io::Result<()> {
    let brand = profile.color(SGR_BRAND_PRIMARY_256);
    let dim = profile.color(SGR_TEXT_SECONDARY_256);
    let (reset, bold) = (profile.reset(), profile.bold());
    let (rows, container) = compose(glyphs);
    let total = rows.len();
    for (idx, row) in rows.iter().enumerate() {
        freeze_line(out, &logo_line(profile, row, idx, total, container))?;
    }
    freeze_line(out, "")?;
    // 中文主品牌行（业主定稿：☁+粗体「云悠 yunyou」+定位语——横幅第一眼可辨）
    freeze_line(
        out,
        &format!(
            "  {bold}{brand}{} 云悠 yunyou{reset} {dim}—— 终端里，云替你干活{reset}",
            profile.symbol(SYM_BRAND, ascii::BRAND)
        ),
    )?;
    freeze_line(out, &format!("    {dim}模型 {model}{reset}"))?;
    freeze_line(out, &format!("    {dim}会话 {session_dir}{reset}"))?;
    freeze_line(out, "")?;
    if tips {
        for tip in TIPS {
            freeze_line(out, &tips_line(profile, tip))?;
        }
    }
    if let Some(text) = notice {
        freeze_line(
            out,
            &format!(
                "    {}{}{} {}{}{}",
                brand,
                profile.symbol(SYM_BULLET, ascii::BULLET),
                reset,
                dim,
                super::tool::clip_width(text, width.saturating_sub(6)),
                reset
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
    // 第一行：display 层（中文主品牌行=业主定稿形态，与 D 案样稿一致）
    freeze_line(
        out,
        &format!("  {brand}{cloud}{reset} {bold}{brand}云悠 yunyou{reset} {dim}—— 终端里，云替你干活{reset}"),
    )?;
    // 第二三行：caption 层（烟灰 dim）；退出指引与大字版 Tips 同句式同源
    //（任务30 四期④结巴句并句）；task-38②：--task 模式自退出不渲染
    freeze_line(
        out,
        &format!(
            "    {dim}模型 {model} {} 会话 {session_dir}{reset}",
            profile.symbol(SYM_FENCE_DOT, ascii::FENCE_DOT)
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

    /// 三断言之一·容器宽：大版=6×3+2×2=22 列、紧凑=5×3+2×2=19 列（数学
    /// 推导，零魔法数——业主令②）。
    #[test]
    fn 容器宽_数学推导() {
        let (rows, w) = compose(&[
            Glyph { matrix: &GLYPH_Y, descender: false },
            Glyph { matrix: &GLYPH_U, descender: true },
            Glyph { matrix: &GLYPH_N, descender: true },
        ]);
        assert_eq!(w, 22, "大版容器宽=6×3+2×2");
        assert_eq!(rows.len(), 7, "大版 7 行（Y 满高）");
        let (rows2, w2) = compose(&[
            Glyph { matrix: &GLYPH_Y5, descender: false },
            Glyph { matrix: &GLYPH_U5, descender: true },
            Glyph { matrix: &GLYPH_N5, descender: true },
        ]);
        assert_eq!(w2, 19, "紧凑容器宽=5×3+2×2（5 宽原生族）");
        assert_eq!(rows2.len(), 5, "紧凑版 5 行");
    }

    /// 三断言之二·行数与基线：u/n x-height 4 行底对齐（前 3 行空）；
    /// 出脚=末行 1→▄（u 底横/n 双柱底），Y 无出脚全 █。
    #[test]
    fn 基线对齐_出脚() {
        let (rows, _) = compose(&[
            Glyph { matrix: &GLYPH_Y, descender: false },
            Glyph { matrix: &GLYPH_U, descender: true },
            Glyph { matrix: &GLYPH_N, descender: true },
        ]);
        let seg = |row: &str, _r: usize| -> String { row.chars().skip(8).take(6).collect() };
        // u 区（列 8..14）：行 3=u 矩阵首行双柱（底对齐 offset=3，x-height 顶）
        assert_eq!(seg(&rows[3], 3), "█    █", "u 顶行=双柱（底对齐 x-height 顶）");
        // 出脚：末行 u 区 ▄、n 区 ▄；Y 区（列 0..6）无出脚
        assert_eq!(seg(&rows[6], 6), "▄▄▄▄▄▄", "u 末行出脚 ▄");
        assert!(!rows[..6].iter().any(|r| r.chars().take(6).any(|c| c == '▄')), "Y 无出脚");
    }

    /// 三断言之三·居中对称：pad 公式=(容器宽−行宽).max(2)/2+2，两侧留白
    /// 对称性=公式单点决定（余数归右由整除承载）。
    #[test]
    fn 居中对称() {
        let line = logo_line(Profile::Full, "██████", 0, 7, 22);
        // 行首空格计数（SGR 在空格之后，不污染前导空白；禁字节长度法——SGR 字节不可见）
        let lead = line.chars().take_while(|&c| c == ' ').count();
        let expect = (22usize.saturating_sub(6)).max(2) / 2 + 2;
        assert_eq!(lead, expect, "pad=公式直算（居中单点，余数归右）");
        assert!(lead >= 2, "最小呼吸 2 列");
    }
}
