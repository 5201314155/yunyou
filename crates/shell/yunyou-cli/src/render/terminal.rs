//! 终端能力探测与输出原语（规范 §3 降级 + §5 硬约束的执行点）。
//!
//! - 降级开关：`YUNYOU_PLAIN`（色+符号全退化 ASCII）/ `NO_COLOR`（存在且
//!   非空即剥，no-color.org 惯例——task-19②，值不限 "1"）/ `TERM=dumb`
//!   （同 PLAIN）/ 非 TTY（同 NO_COLOR：剥全部 ANSI——色彩与样式 SGR 都不进
//!   管道，规范 §5.4「剥离全部 ANSI」；符号保留便于管道处理——task-19③）；
//! - 原语：色包裹、当前活动行 `\r\e[K` 原位重绘（唯一允许的原位更新）。

use std::env;
use std::io::Write;

include!(concat!(env!("OUT_DIR"), "/palette.rs"));

/// 终端渲染能力档位（规范 §3/§5.4 降级矩阵）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// 全能力：256 色 + Unicode 符号（交互 TTY 默认）。
    Full,
    /// 无色档：剥全部 ANSI 色彩，保留 Unicode 符号（NO_COLOR / 非 TTY）。
    NoColor,
    /// 纯文本档：无色 + ASCII 符号降级（YUNYOU_PLAIN / TERM=dumb）。
    Plain,
}

impl Profile {
    /// 从进程环境探测当前档位（判定顺序见各分支注释）。
    pub fn detect(is_tty: bool) -> Self {
        Self::detect_from(is_tty, &|key| env::var(key).ok())
    }

    /// 注入式探测（测试用；判定顺序：PLAIN/dumb → Plain，NO_COLOR/非 TTY → NoColor。
    /// NO_COLOR 取 presence 语义（存在且非空即生效，不限 "1"）——no-color.org
    /// 惯例，CI/工具链设 true/yes 等任意非空值场景照剥（task-19②，PTY 实证
    /// NO_COLOR=true 19 处 256 色全开的违约面根除）。
    pub fn detect_from(is_tty: bool, get: &dyn Fn(&str) -> Option<String>) -> Self {
        let plain = get("YUNYOU_PLAIN").is_some_and(|v| v == "1")
            || get("TERM").is_some_and(|t| t == "dumb");
        if plain {
            return Self::Plain; // 色彩+符号全退化
        }
        let no_color = get("NO_COLOR").is_some_and(|v| !v.is_empty()) || !is_tty;
        if no_color {
            return Self::NoColor; // 仅剥色
        }
        Self::Full
    }

    /// 语义色 → 前景 SGR 前缀（如 `"\e[38;5;67m"`）；无色档返回空串。
    /// 一期只走 256 色主案；16 色降级档（`SGR_*_16`）预留，出现需求时扩 Profile 位。
    pub fn color(self, sgr256: &str) -> String {
        match self {
            Self::Full => format!("\x1b[{sgr256}m"),
            Self::NoColor | Self::Plain => String::new(),
        }
    }

    /// 语义符号 → 实际字符（纯文本档走 ASCII 降级表）。
    pub fn symbol(self, unicode: &'static str, ascii: &'static str) -> &'static str {
        match self {
            Self::Full | Self::NoColor => unicode,
            Self::Plain => ascii,
        }
    }

    /// spinner 帧序列（纯文本档走 ASCII 三帧）。
    pub fn spinner_frames(self) -> &'static [&'static str] {
        match self {
            Self::Full | Self::NoColor => &SPINNER_FRAMES,
            Self::Plain => &ascii::SPINNER_FRAMES,
        }
    }

    /// 粗体样式前缀（NoColor 档剥全 ANSI——task-19③：规范 §5.4「非 TTY 自动
    /// 剥离全部 ANSI」按字面执行，色彩与样式 SGR 都不进管道；NoColor 与 Plain
    /// 的分界只剩符号降级。粗体语义在无色档由符号形态自行承载）。
    pub fn bold(self) -> &'static str {
        match self {
            Self::Full => "\x1b[1m",
            Self::NoColor | Self::Plain => "",
        }
    }

    /// 暗淡样式前缀（NoColor 剥除，同 [`Self::bold`]）。
    pub fn dim(self) -> &'static str {
        match self {
            Self::Full => "\x1b[2m",
            Self::NoColor | Self::Plain => "",
        }
    }

    /// 样式复位（无色档零 ANSI——剥全 ANSI 后无样式可复位，输出纯净字节流）。
    pub fn reset(self) -> &'static str {
        match self {
            Self::Full => "\x1b[0m",
            Self::NoColor | Self::Plain => "",
        }
    }

    /// 斜体样式前缀（SGR 3；NoColor/Plain 档无样式降级——取舍见 inline.rs 模块注释）。
    pub fn italic(self) -> &'static str {
        match self {
            Self::Full => "\x1b[3m",
            Self::NoColor | Self::Plain => "",
        }
    }
}

/// 当前活动行原位重绘（规范 §5.1 唯一允许的原位更新形态：`\r\e[K`）。
pub fn rewrite_active_line<W: Write>(out: &mut W, line: &str) -> std::io::Result<()> {
    write!(out, "\r\x1b[K{line}")
}

/// 清当前活动行（`\r\e[K` 重绘原语的清除半边；任务30 四期 2.1 收敛单点）。
///
/// `when` 为调用方判据（交互态/有重绘痕迹），false 时零字节输出。收敛
/// 理由：清帧逻辑此前在 mod.rs 三处落定路径与 stream/thinking 的活动行
/// 处理中各自手写，新增落定路径时漏清/多清全靠人肉记忆——D-UI-1 即
/// 这类状态机缝隙的产物；收敛后全库只有本函数产出 `\r\e[K` 清除序列
/// （重绘序列在 [`rewrite_active_line`]），grep 单点可查。
pub fn clear_active_line<W: Write>(out: &mut W, when: bool) -> std::io::Result<()> {
    if when {
        write!(out, "\r\x1b[K")?;
    }
    Ok(())
}

/// 落定一行（追加换行；此后该行即历史，永不回改——规范 §1 铁律 1）。
pub fn freeze_line<W: Write>(out: &mut W, line: &str) -> std::io::Result<()> {
    writeln!(out, "{line}")
}

/// 输入行原位清写（`\r\e[K` + 内容）：光标已在行首时整行重绘——面板输入行/
/// 折叠回落共用。面板域直写逃逸的收编原语（task-54：`\r\e[K` 直写全仓仅
/// terminal.rs 原语可查）。
pub(crate) fn rewrite_input_line(
    out: &mut impl std::io::Write,
    content: &str,
) -> std::io::Result<()> {
    clear_active_line(out, true)?;
    write!(out, "{content}")
}

/// 换行+原位清写（`\n\r\e[K` + 内容）：面板候选区/提示行在输入行之下逐行
/// 展开——先换行再清行（上一帧可能更长，残留须清）。
pub(crate) fn append_cleared_line(
    out: &mut impl std::io::Write,
    content: &str,
) -> std::io::Result<()> {
    writeln!(out)?; // 先换行：候选区在输入行之下逐行展开
    clear_active_line(out, true)?;
    write!(out, "{content}")
}
