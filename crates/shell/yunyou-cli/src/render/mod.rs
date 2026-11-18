//! 渲染模块门面（任务 task-10 B 线；设计规范 docs/设计/CLI终端显示设计规范.md）。
//!
//! 职责：把 A 线外露事件（[`event::TurnEvent`]）按 §4 映射表分发到各视觉块，
//! 维护流式状态机（按行缓冲/块切换/活动行）。视觉块函数为纯函数
//! （Profile + 数据 → 字节），可按 mock 帧序做单元测试（tests/render.rs）。
//!
//! 硬约束执行点（规范 §5）：线性流式、唯一原位更新为当前活动行 `\r\e[K`、
//! 禁 alternate screen、降级开关见 [`terminal::Profile`]。

/// 列宽探测（任务30 二期④立 COLUMNS 单源；任务30 三期 B1 三层升级）。
///
/// 三层判定（[`width_from`] 纯函数）：①ioctl TIOCGWINSZ——真实终端唯一
/// 可信源：bash 不导出 COLUMNS 给子进程，48 列真机 SSH 下 env 探测失效
/// 恒按 80 预算=落款折行复活（业主实锤）；②COLUMNS env（交互 shell 显式
/// 导出时兜底）；③80 兜底——非交互/缺失/越界（<20 多为伪环境，>500 无
/// 意义）一律取 80 不猜：按常规宽兜底最坏只是不截断，猜窄值会误截正常
/// 落款。banner 分流与落款截断共用本单源（`Renderer::width`）。
fn detect_width(interactive: bool) -> usize {
    if !interactive {
        return 80;
    }
    width_from(terminal_cols(), env::var("COLUMNS").ok().as_deref())
}

/// 三层判定纯函数：ioctl 实测 → COLUMNS env → 80 兜底（单测即契约）。
fn width_from(ioctl: Option<usize>, columns: Option<&str>) -> usize {
    ioctl
        .or_else(|| {
            columns
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|&c| (20..=500).contains(&c))
        })
        .unwrap_or(80)
}

/// ioctl TIOCGWINSZ 取终端列宽（STDOUT，与渲染输出同流；非 TTY/取不到
/// 返回 None 走下层）。ioctl 壳不做单测（依赖真实 TTY），三层判定逻辑
/// 全部收敛在 [`width_from`] 纯函数覆盖。
/// ioctl 实测列宽（task-17③ 起 pub(crate)：panel_text 的面板截断基准复用
/// 本单源——面板候选行按真实终端宽截断，不再写死 80（40 列终端 D-UI-1
/// 同机理折行实测在案）。
pub(crate) fn terminal_cols() -> Option<usize> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // STDOUT_FILENO = 1
    let ok = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0;
    (ok && ws.ws_col > 0).then_some(ws.ws_col as usize)
}

pub mod banner;
pub(crate) mod tool_render; // 云络 P6 测试面（adapter 侧跨模块调用）（tool_render.rs 本体随批，署名披露）
pub mod sanitize;
pub mod detail;
pub mod event;
pub mod inline;
pub mod status;
pub mod stream;
pub mod terminal;
pub mod selfheal;
pub mod thinking;
pub mod tool;
pub mod tree;

pub use event::{DetailKind, DiffHunk, TurnEvent};
pub use thinking::ThinkingMode;
pub use terminal::Profile;

use std::env;
use std::io::Write;
use stream::StreamState;
use terminal::{ascii, SYM_FENCE_DOT, SYM_FOLD, SYM_PROMPT};

/// 渲染器：包输出流 + 能力档位 + 流式状态 + 思考流模式。
pub struct Renderer<W: Write> {
    out: W,
    profile: Profile,
    /// 交互终端判定（TTY 且 TERM≠dumb）：false 时 spinner 帧与活动行清理
    /// 静默（\r 在管道/dumb 终端不回位，刷帧会污染输出——审查契约第 4 条修正）。
    /// 流式落定语义与交互无关（D-UI-1：满行才落屏，未满行一律由收尾落定）。
    interactive: bool,
    /// 终端列宽单源（任务30 二期④）：落款行模型名截断与横幅版式分流共用，
    /// 不各探各的。构造时探测一次，测试/装配层经 [`Renderer::set_width`] 覆盖。
    width: usize,
    /// 思考流模式（任务 30-②，默认收敛；YUNYOU_THINKING=full 走全文流式）。
    thinking: ThinkingMode,
    stream: StreamState,
    /// 自愈检测器（task-48 接线）：原位重绘字节量滑窗→高延迟自动降级
    /// （YUNYOU_STATIC 语义=暂态静默，interactive 翻 false 复用全套非交互
    /// 静态路径）；显式 env 强制由 [`Self::set_selfheal_forced`] 接线。
    selfheal: selfheal::SelfHeal,
    /// interactive 初值（降级翻转后恢复用）。
    interactive_orig: bool,
}

impl<W: Write> Renderer<W> {
    /// 以输出流、档位、交互性构造（档位见 [`Profile::detect`]；
    /// interactive=false 时暂态（spinner/活动行清理）静默，只出落定行——流式落定
    /// 本身交互/非交互一致：满行才落屏，未满行由回合收尾统一落定）。
    pub fn new(out: W, profile: Profile, interactive: bool) -> Self {
        Self {
            out,
            profile,
            interactive,
            width: detect_width(interactive),
            thinking: ThinkingMode::default(),
            stream: StreamState::default(),
            selfheal: selfheal::SelfHeal::default(),
            interactive_orig: interactive,
        }
    }

    /// 显式静态档强制（task-48：YUNYOU_STATIC=1 接线——run.rs 读 env 后调用；
    /// 出厂即暂态静默且永不自动恢复）。
    pub fn set_selfheal_forced(&mut self) {
        self.selfheal = selfheal::SelfHeal::new(true, selfheal::DEFAULT_WINDOW_BYTES);
        self.interactive = false;
    }

    /// 自愈阈值注入（task-48 验收/调试逃生口接线：repl 装配层读
    /// YUNYOU_SELFHEAL_BYTES 后调用；默认 256KB/5s 不变）。
    pub fn set_selfheal_threshold(&mut self, bytes: usize) {
        let forced = self.selfheal.is_degraded() && self.selfheal.take_notice().is_none();
        let _ = forced; // forced 态由 set_selfheal_forced 管理，阈值注入不改其态
        self.selfheal = selfheal::SelfHeal::new(false, bytes);
    }

    /// 覆盖列宽（测试注入与装配层覆盖用；列宽只在构造时探测一次）。
    pub fn set_width(&mut self, cols: usize) {
        self.width = cols;
    }

    /// 设置思考流模式（任务 30-②：REPL 装配层按 YUNYOU_THINKING 接线）。
    pub fn set_thinking(&mut self, mode: ThinkingMode) {
        self.thinking = mode;
    }

    /// 输出流与档位的只读访问（REPL 驱动 spinner 循环等场景）。
    pub fn parts(&mut self) -> (&mut W, Profile) {
        (&mut self.out, self.profile)
    }

    /// 是否交互终端（spinner 会话是否值得 spawn 的判据）。
    pub fn interactive(&self) -> bool {
        self.interactive
    }

    /// 流式缓冲是否有未落定内容（task-51② 守卫缝：观察者静默恢复判定用——
    /// 缓冲非空=流式中（正文自己是活动行，不恢复等待行）；缓冲空=段落间隙/
    /// mid-text 思考（恢复等待行，报告3 Bug-2 的 30s+ 静默缝）。
    pub fn has_pending(&self) -> bool {
        self.stream.has_pending()
    }

    /// 取出底层输出流（测试断言输出串用）。
    pub fn into_output(self) -> W {
        self.out
    }

    /// 帧落盘（渲染硬约束「写即见」）：std Stdout 是行缓冲（LineWriter），
    /// 流式帧以 `\r\e[K` 原位重绘、无换行永不触发自动落盘——渲染三期回归
    /// 根因①（回复/落款字节滞留缓冲直到回合收尾，真机「回复不显示」）。
    /// 所有渲染入口末尾统一 commit；新增写入口禁止绕过本约定。
    fn commit(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }

    /// §4.1 启动横幅（一次性输出；版式按列宽/档位分流，见 banner::render）。
    /// `notice`：启动提示行（task-34②；None=不加行）。
    /// `tips`：退出指引行（task-38②；--task 单任务模式自退出传 false 撤噪音）。
    pub fn banner(&mut self, model: &str, session_dir: &str, notice: Option<&str>, tips: bool) -> std::io::Result<()> {
        banner::render(&mut self.out, self.profile, model, session_dir, self.width, notice, tips)?;
        self.commit()
    }

    /// §4.2 输入提示符（❯ 云青，行首不缩进；task-27①：前置换行移除——
    /// 与 [`crate::repl::prompt_string`] 同步改，形态漂移锁测试兜底一致）。
    pub fn prompt(&mut self) -> std::io::Result<()> {
        let profile = self.profile;
        write!(
            self.out,
            "{}{}{} ",
            profile.color(terminal::SGR_BRAND_PRIMARY_256),
            profile.symbol(SYM_PROMPT, ascii::PROMPT),
            profile.reset()
        )?;
        self.commit()
    }

    /// §4.3 用户消息回显（常规白；REPL 的 stdin 回显天然承担，此处仅落盘语义占位）。
    pub fn user_echo(&mut self, text: &str) -> std::io::Result<()> {
        writeln!(self.out, "{text}")?;
        self.commit()
    }

    /// §4.5 assistant 文本增量（混合流式：单视觉行原位重绘/溢出即落定，
    /// 未满行由收尾落定；列宽单源=self.width，任务30 三期④）。
    pub fn text_delta(&mut self, text: &str) -> std::io::Result<()> {
        let width = self.width;
        stream::text_delta(&mut self.out, &mut self.stream, self.profile, self.interactive, width, text)?;
        self.commit()?;
        // task-48 接线：重绘字节喂自愈检测器；超阈一次性降级（interactive 翻
        // false=暂态全静默，复用全套非交互静态路径）+告知交观察者层。
        let rewrite = self.stream.take_last_rewrite_bytes();
        self.selfheal.note_rewrite_wall(rewrite);
        if self.selfheal.is_degraded() && self.interactive {
            self.interactive = false; // 降级：后续增量按静态路径落定
            return Ok(());
        }
        Ok(())
    }

    /// 思考内容增量（任务 30-②：默认收敛为活动行+冻结单行；full=全文流式）。
    pub fn reasoning_delta(&mut self, text: &str) -> std::io::Result<()> {
        let width = self.width;
        stream::reasoning_delta(
            &mut self.out,
            &mut self.stream,
            self.profile,
            self.interactive,
            self.thinking,
            width,
            text,
        )?;
        self.commit()?;
        // task-48 接线：思考流重绘同规喂入（低频但同样占带宽）。
        let rewrite = self.stream.take_last_rewrite_bytes();
        self.selfheal.note_rewrite_wall(rewrite);
        if self.selfheal.is_degraded() && self.interactive {
            self.interactive = false;
        }
        Ok(())
    }

    /// §4.6 工具进行中单帧（按事件；REPL/观察者 80ms 驱动；非交互静默）。
    /// `elapsed_secs`=运行耗时秒级取整（帧驱动换算注入，任务47②）。
    pub fn tool_running_frame(
        &mut self,
        event: &TurnEvent,
        frame: usize,
        elapsed_secs: u64,
    ) -> std::io::Result<()> {
        let TurnEvent::ToolCall { name, summary, .. } = event else { return Ok(()) };
        if !self.interactive {
            return Ok(());
        }
        let profile = self.profile;
        // 批 4（T1-b）：spinner 进行时摘要走渲染器 activity 动词短语（未知
        // 工具兜底=原样，零行为）；进行时是暂态帧，与完成卡 summary 分轨。
        let activity = crate::render::tool_render::renderer(name).activity(name, Some(summary), summary);
        let view = tool::ToolCallView { name, summary: &activity };
        let width = self.width;
        tool::tool_running_frame(&mut self.out, profile, &view, frame, Some(elapsed_secs), width, 0)?;
        self.commit()
    }

    /// §4.6 便捷帧：直接按工具名与摘要渲染（观察者 spinner 任务用）。
    pub fn tool_call_frame(
        &mut self,
        name: &str,
        summary: &str,
        frame: usize,
        elapsed_secs: Option<u64>,
    ) -> std::io::Result<()> {
        if !self.interactive {
            return Ok(());
        }
        let profile = self.profile;
        // 批 4 同上：观察者 spinner 帧同人话化（单源=renderer activity）
        let activity = crate::render::tool_render::renderer(name).activity(name, Some(summary), summary);
        let view = tool::ToolCallView { name, summary: &activity };
        let width = self.width;
        tool::tool_running_frame(&mut self.out, profile, &view, frame, elapsed_secs, width, 0)?;
        self.commit()
    }

    /// §4.7/§4.8/§4.9–4.11 工具结果：完成卡 +（可选）详情展开。
    pub fn tool_done(&mut self, event: &TurnEvent) -> std::io::Result<()> {
        // 完成卡：交互下先清进行中活动行再落定；非交互无活动行，直接追加
        terminal::clear_active_line(&mut self.out, self.interactive)?;
        tool::tool_done(&mut self.out, self.profile, event, self.width, 0)?;
        // 详情块（新插入的行，线性流末尾追加）
        if let TurnEvent::ToolResult { detail: Some(kind), .. } = event {
            detail::render(&mut self.out, self.profile, kind, self.width)?;
        }
        self.commit()
    }

    /// 工具结果（树归集形态，任务 30-④）：带缩进前缀的完成卡 + 详情展开。
    /// 前缀由观察者的树归集器给出（[`tree::indent_prefix`]）；depth=0 走 [`Self::tool_done`]。
    pub fn tool_done_with_prefix(&mut self, event: &TurnEvent, prefix: &str) -> std::io::Result<()> {
        terminal::clear_active_line(&mut self.out, self.interactive)?;
        write!(self.out, "{prefix}")?;
        tool::tool_done(&mut self.out, self.profile, event, self.width, tool::visible_width(prefix))?;
        if let TurnEvent::ToolResult { detail: Some(kind), .. } = event {
            detail::render(&mut self.out, self.profile, kind, self.width)?;
        }
        // task-51 移交②（云刺指认 flush 纪律缺口）：树归集完成卡与其他写入口
        // 同规 commit，防字节滞留 LineWriter 缓冲。
        self.commit()
    }

    /// task-48：自愈降级一次性告知取出口（观察者层发；None=无待发告知）。
    pub fn take_selfheal_notice(&mut self) -> Option<&'static str> {
        self.selfheal.take_notice()
    }

    /// task-48：回合界（TurnComplete 处调用）——清窗重探+连续健康恢复；
    /// 返回 true=恢复交互态（观察者无需动作，renderer 自还原）。
    pub fn selfheal_new_turn(&mut self) -> bool {
        let recovered = self.selfheal.new_turn_wall();
        if recovered {
            self.interactive = self.interactive_orig;
        }
        recovered
    }

    /// §4.12 状态落款行（回合完结；列宽来自 [`Renderer::width`] 单源）。
    pub fn turn_done(&mut self, event: &TurnEvent) -> std::io::Result<()> {
        stream::close_block(&mut self.out, &mut self.stream, self.profile, self.interactive, self.width)?; // task-65 close_block 增 width 参调用点同步（云愈件漏同步，云刺代修披露）
        status::turn_done(&mut self.out, self.profile, event, self.width)?;
        self.commit()
    }

    /// §4.14 回合等待指示单帧（task-35a：首字节前/段间空窗的活体信号；
    /// 观察者 1s ticker 驱动；交互档专用，非交互静默——静态行走 [`Self::waiting_notice`]）。
    ///
    /// 宽度护栏：复用 [`tool::clip_width`] 既有截断（全角 2 列计），不新增第二份
    /// 判定（task-33③ 单源纪律）；截断对象是剥 ANSI 后的纯文本，色彩包裹在其后。
    pub fn waiting_frame(&mut self, elapsed: std::time::Duration) -> std::io::Result<()> {
        if !self.interactive {
            return Ok(());
        }
        // task-35②：文案符号走 symbol 降级体系——省略号复用 SYM_FOLD（…→...
        // 现成映射）、间隔符复用 SYM_FENCE_DOT（·→.，D3 声明映射的落地单点，
        // 余三符号 ▏−– 随 task-65 收编批）；Plain 档（YUNYOU_PLAIN=1 的 TTY，
        // TERM=dumb 时 interactive=false 本就不出帧）零非 ASCII 残留。
        let fold = self.profile.symbol(SYM_FOLD, ascii::FOLD);
        let dot = self.profile.symbol(SYM_FENCE_DOT, ascii::FENCE_DOT);
        let text = format!("{fold} 等待回复 ({}s {dot} Ctrl-C 中断)", elapsed.as_secs());
        let line = format!(
            "{}{}{}",
            self.profile.dim(),
            tool::clip_width(&text, self.width.saturating_sub(1)),
            self.profile.reset()
        );
        terminal::rewrite_active_line(&mut self.out, &line)?;
        self.commit()?;
        // task-48 接线：等待帧本身也是原位重绘流量（@1s×帧字节）——喂检测器；
        // 高延迟链路下等待期持续开销正是自愈标的（字节口径内）。降级即翻转
        // interactive=false：ticker 下一帧起自然停摆（静态行由 35b 路径接管）。
        let rewrite = line.len() + 8; // 帧字节+\r\e[K 允差
        self.selfheal.note_rewrite_wall(rewrite);
        if self.selfheal.is_degraded() && self.interactive {
            self.interactive = false;
        }
        Ok(())
    }

    /// 回合等待静态提示（task-35b：非交互/Plain 档无活动行能力——`\r\e[K`
    /// 不回位会刷屏，落一行静态文案替代逐秒重绘；交互档零输出，由 ticker 接管）。
    pub fn waiting_notice(&mut self) -> std::io::Result<()> {
        if self.interactive {
            return Ok(());
        }
        writeln!(
            self.out,
            "{}（等待回复中，Ctrl-C 可中断）{}",
            self.profile.dim(),
            self.profile.reset()
        )?;
        self.commit()
    }

    /// §4.13 会话恢复回放头部。
    pub fn resume_header(&mut self, session_id: &str, messages: usize) -> std::io::Result<()> {
        status::resume_header(&mut self.out, self.profile, session_id, messages)?;
        self.commit()
    }

    /// 错误行（绯红，错误不能藏）。
    pub fn error(&mut self, message: &str) -> std::io::Result<()> {
        status::error(&mut self.out, self.profile, message)?;
        self.commit()
    }

    /// 回合收尾（观察者 Ok/Err 两路统一调用，审查 P1-2）：
    /// 落定未满行（未满行从未上屏，收尾按块渲染补落定，见 [`stream::close_block`]），
    /// 交互下再清一次活动行（spinner/流式行残留防御；非交互无活动行）。
    pub fn finish(&mut self) -> std::io::Result<()> {
        stream::close_block(&mut self.out, &mut self.stream, self.profile, self.interactive, self.width)?; // task-65 close_block 增 width 参调用点同步（云愈件漏同步，云刺代修披露）
        terminal::clear_active_line(&mut self.out, self.interactive)?;
        self.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 渲染回归根因①锁测试：渲染入口写后必须立即 flush。std Stdout 行缓冲
    /// 对无换行的流式帧（\r\e[K 重绘）不自动落盘——字节滞留直到回合收尾，
    /// 真机表现为「回复不显示、Ctrl-C 后一口气涌出」（task-31 strace 三场实锤）。
    struct FlushCounter {
        buf: Vec<u8>,
        flushed: usize,
    }
    impl Write for FlushCounter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.buf.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.flushed += 1;
            Ok(())
        }
    }

    #[test]
    fn 渲染入口写后即flush不滞留缓冲() {
        let mut w = FlushCounter { buf: Vec::new(), flushed: 0 };
        let mut r = Renderer::new(&mut w, Profile::Plain, true);
        r.prompt().expect("提示符写入");
        assert_eq!(r.parts().0.flushed, 1, "prompt 写后即落盘");
        r.text_delta("你好").expect("文本增量写入");
        assert_eq!(r.parts().0.flushed, 2, "流式帧写后即落盘（回归根因①：无换行重绘不得滞留）");
        assert!(!r.parts().0.buf.is_empty(), "字节应已进入底层写入口");
    }

    /// B1 三层判定：ioctl 实测优先（真机 SSH 唯一可信源）；ioctl 失败退
    /// COLUMNS env（越界过滤）；两路皆空 80 兜底不猜。
    #[test]
    fn 列宽三层_ioctl优先_env次之_兜底八十() {
        assert_eq!(width_from(Some(48), Some("200")), 48, "ioctl 实测优先");
        assert_eq!(width_from(None, Some("48")), 48, "ioctl 失败退 env");
        assert_eq!(width_from(None, Some("  80 ")), 80, "env 容忍空白");
        assert_eq!(width_from(None, Some("10")), 80, "env 越界（<20 伪环境）不取");
        assert_eq!(width_from(None, Some("9999")), 80, "env 越界（>500 无意义）不取");
        assert_eq!(width_from(None, Some("abc")), 80, "env 非数值不取");
        assert_eq!(width_from(None, None), 80, "两路皆空 80 兜底不猜");
    }
}
