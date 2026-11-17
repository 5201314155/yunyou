//! 单行编辑态（任务29 C3/C2，CLI 壳面）：补全面板的字符级输入循环。
//!
//! 设计 §四「read_line 前的单行编辑态，不自研行编辑库」的落点：termios 关
//! ICANON/ECHO（保留 ISIG——Ctrl-C 仍走 SIGINT 信号路径，与既有 REPL 语义一致），
//! 逐字符读+自管回显；处理面一期最小=可见字符入缓冲/退格删尾/Tab 补全循环/
//! Enter 提交/Esc 收面板/Ctrl-D 结束；方向键 CSI 序列整段吞弃不污染缓冲
//!（历史/光标移动属二期）。UTF-8 多字节字符整字入缓冲。
//!
//! 交互性判据由调用方决定（与暂态判据同款）：非交互（管道）走
//! [`Editor::read_line_plain`]——补全面板整体静默，逐行读语义与既有 REPL 等价。

use crate::history::LineHistory;
use crate::panel::Panel;
use crate::render::Profile;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::input_source::InputSource;
use tokio_util::sync::CancellationToken;
use yunyou_commands::{CommandDescriptor, CommandsService};

/// 追加差分判定（task-42）：新缓冲以旧为前缀且确有新增 → 返回新增尾段；
/// 否则 None（退格/历史/面板替换/等长防御——走全行重绘）。
/// 字符边界护栏（lead 护栏①）：字节前缀之外再验 is_char_boundary——CJK 多
/// 字节若被劈半（防御性，prev 恒为完整快照实际不触发）一律 None 回退全行，
/// 绝不 panic；先 diff 后写（护栏②）：本函数为纯判定，refresh 在分支选定后
/// 才发生任何终端写入，无「先写后判」闪烁窗口。
fn appended_suffix<'a>(prev: &str, buffer: &'a str) -> Option<&'a str> {
    (buffer.len() > prev.len()
        && buffer.starts_with(prev)
        && buffer.is_char_boundary(prev.len()))
    .then(|| &buffer[prev.len()..])
}

/// CSI final 字节判定（ECMA-48：0x40..=0x7E；票④测试锚点）。
fn csi_is_final(byte: u8) -> bool {
    (0x40..=0x7E).contains(&byte)
}

/// CSI 参数/中间字节判定（0x20..=0x3F：数字/分号/修饰符，继续读）。
fn csi_continues(byte: u8) -> bool {
    (0x20..=0x3F).contains(&byte)
}

/// CSI 序列解析结果（面板交互升级票①②④）：方向键双语义（面板开=选候选、
/// 关=历史导航），其余序列吞弃。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CsiKey {
    /// ↑（final 'A'）。
    Up,
    /// ↓（final 'B'）。
    Down,
    /// 孤立 Esc（后续 50ms 无字节——收面板语义）。
    Escape,
    /// 其他 CSI/ESC 序列（吞弃）。
    Other,
}

/// 一行输入的处置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineInput {
    /// 用户回车提交（斜杠命令与普通消息的分发由 REPL 决定）。
    Submitted(String),
    /// Ctrl-C（SIGINT 路径）。
    Interrupted,
    /// Ctrl-D / 流结束。
    Eof,
    /// 退出信号（/quit 的 token）。
    ExitRequested,
}

/// 单行编辑器：提示符/能力档位/面板数据源/退出令牌/输入历史的持有者。
///（REPL 每轮从当前会话重建，history/hint_shown 以 Arc 跨轮共享。）
pub struct Editor {
    /// 已着色的提示符串（与 REPL prompt 同款形态）。
    pub prompt: String,
    /// 终端能力档位（面板渲染降级同源）。
    pub profile: Profile,
    /// 面板数据源（每帧 list() 实时查询，设计 §四无缓存）。
    pub commands: Arc<dyn CommandsService>,
    /// 命令目录快照（task-36③，挑刺 P3-3）：REPL 每轮重建 Editor 时 list()
    /// 一次，键入期零 list 调用——旧形态每键全量克隆目录+过滤双克隆，人手
    /// 速下 N 命令×2 份克隆/键；快照失效=Editor 重建（/new 换装配自动跟随，
    /// 会话内命令注册表装配期后不变）。设计 §四「无缓存」语义随本卡修订
    /// （缓存粒度=每轮一次，非跨轮）。
    pub commands_snapshot: Arc<Vec<CommandDescriptor>>,
    /// 壳退出令牌（/quit）。
    pub exit: CancellationToken,
    /// 输入历史（面板交互票①；跨轮共享同一份，REPL 层 Arc 持有）。
    pub history: Arc<LineHistory>,
    /// 终端列宽（task-66 P1-2 参数化：三层探测单源注入，面板帧渲染基准；
    /// 本字段曾两度被并行批同步吞没——重植第三次数损，见 66 卡浪潮记录）。
    pub width: usize,
    /// 面板操作提示是否已展示过（发现性票③：只提示一次，跨轮共享）。
    pub hint_shown: Arc<AtomicBool>,
}

impl Editor {
    /// 交互路径读一行（单行编辑态+补全面板）。
    ///
    /// `initial` 为回填初始缓冲（展开类命令的输入框回填，设计 §七）：
    /// 提示符+回填文本先上屏，用户可增删改后回车发送。
    pub async fn read_line(&self, initial: Option<String>) -> std::io::Result<LineInput> {
        // task-73 A1：会话级 cbreak（外层 repl_loop 持守卫）——此处 enable=
        // 幂等重入（tcsetattr 同值无害），Drop 只恢复到会话级形态而非 cooked
        // （防逐行守卫先 drop 把会话守卫的 cbreak 打回 cooked）。
        let _raw = RawMode::enable()?;
        let mut stdout = std::io::stdout();
        let mut panel = Panel::default();
        let mut buffer = initial.unwrap_or_default();
        // 提示符+初始缓冲上屏（ECHO 已关，回显自管）
        write!(stdout, "{}{buffer}", self.prompt)?;
        stdout.flush()?;
        // 追加差分基准（task-42）：首绘后的缓冲快照，随每次 refresh 同步
        let mut prev = buffer.clone();
        if Self::panel_relevant(&buffer) {
            panel.update(&self.commands_snapshot, &buffer);
            if panel.is_open() {
                // 发现性提示（票③）：首次开面板随帧画一行操作提示（只一次）
                if !self.hint_shown.swap(true, Ordering::Relaxed) {
                    panel.set_hint(true);
                }
                panel.render_frame(&mut stdout, self.profile, &self.prompt, &buffer, self.width)?; // task-66 P1-2 第 5 参补齐（E0061×2 收口）
            }
        }
        // 输入源（task-43）：长命读线程+通道——tokio::io::stdin() 的 blocking
        // read 不可取消，Runtime 关停干等（编辑态 Ctrl-C 退出拖 14s+ 真根）。
        let source = InputSource::global();
        loop {
            tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => return Ok(LineInput::Interrupted),
                _ = self.exit.cancelled() => return Ok(LineInput::ExitRequested),
                read = source.read_byte() => {
                    let Some(b) = read? else {
                        return Ok(LineInput::Eof);
                    };
                    match b {
                        // 回车：收面板→落定输入行→提交；面板开着时执行**选中项**
                        //（设计 §四「Enter 执行选中项」——交互升级票②的另一半：
                        // 用户输入 /he 面板显示 /help 时，回车提交的是 /help 而非 /he）
                        b'\r' | b'\n' => {
                            if let Some(replacement) = panel.selected_replacement() {
                                buffer = replacement;
                            }
                            if panel.is_open() {
                                panel.collapse(&mut stdout, &self.prompt, &buffer)?;
                            }
                            writeln!(stdout)?;
                            stdout.flush()?;
                            return Ok(LineInput::Submitted(buffer));
                        }
                        // 退格（DEL）：删尾字符
                        0x7F => {
                            buffer.pop();
                            self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                        }
                        // 迟到 CSI 残段吸收（task-34 云勘 S2 打回）：回合期
                        // Esc 监视臂把「分批到达的方向键」的 0x1B 判孤立吃掉
                        // 后，迟到的 0x5B('[') 0x41('A') 失去引导符会以字面
                        // 进缓冲（实测 [A 污染+/quit 被拼接提交）——行首
                        // （缓冲空）遇 '[' 时 50ms 内探测后继：A-D 跟上=迟到
                        // 残段整对吞弃；否则按普通字符入缓冲不吞。
                        b'[' if buffer.is_empty() => {
                            match tokio::time::timeout(
                                std::time::Duration::from_millis(50),
                                source.read_byte(),
                            )
                            .await
                            {
                                // 迟到残段：'['+A-D 整对吞弃（无光标语义可执行
                                // ——历史召回已在 0x1B 被吃前不可恢复，吞弃优于
                                // 字面污染）
                                Ok(Ok(Some(next)))
                                    if (b'A'..=b'D').contains(&next) => {}
                                // 非残段：'[' 按普通字符入缓冲（后继字节留
                                // 通道走正常路径，不额外抢读）
                                _ => {
                                    buffer.push('[');
                                    self.refresh(
                                        &mut panel,
                                        &mut stdout,
                                        &buffer,
                                        &mut prev,
                                    )?;
                                }
                            }
                            self.refresh(
                                &mut panel,
                                &mut stdout,
                                &buffer,
                                &mut prev,
                            )?;
                        }
                        // Tab：循环补全选中项到输入行
                        b'\t' => {
                            if let Some(completed) = panel.tab_next() {
                                buffer = completed;
                                self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                            }
                        }
                        // Ctrl-D：结束（管道 EOF 等价形态）
                        0x04 => return Ok(LineInput::Eof),
                        // Esc/方向键（交互升级票①②④）：单发 Esc=收面板；
                        // CSI 序列按 final 字节分发——↑↓ 面板开=选候选、面板关=
                        // 历史导航；其余序列吞弃。P2-2 根因修复：引导符 '['
                        // (0x5B) 先单独消费，不再落入 0x40..=0x7E 终止区间被
                        // 误判为 final（旧实现 'A' 残字入缓冲即此因）。
                        0x1B => match Self::read_csi(source).await {
                            Some(CsiKey::Up) => {
                                if panel.is_open() {
                                    panel.move_up();
                                    self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                                } else if let Some(entry) = self.history.up(&buffer) {
                                    buffer = entry;
                                    self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                                }
                            }
                            Some(CsiKey::Down) => {
                                if panel.is_open() {
                                    panel.move_down();
                                    self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                                } else if let Some(next) = self.history.down() {
                                    buffer = next;
                                    self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                                }
                            }
                            Some(CsiKey::Escape) if panel.is_open() => {
                                panel.collapse(&mut stdout, &self.prompt, &buffer)?;
                            }
                            _ => {} // 其他 CSI/孤立序列：吞弃
                        },
                        // 可见字符：UTF-8 整字入缓冲（含中文别名输入）
                        b if b >= 0x20 => {
                            if let Some(ch) = Self::read_char(source, b).await? {
                                buffer.push(ch);
                                self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                            }
                        }
                        _ => {} // 其余控制字符忽略
                    }
                }
            }
        }
    }

    /// 非交互路径读一行（补全面板静默；逐行读语义与既有 REPL 等价）。
    pub async fn read_line_plain(&self) -> std::io::Result<LineInput> {
        let source = InputSource::global();
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => Ok(LineInput::Interrupted),
            _ = self.exit.cancelled() => Ok(LineInput::ExitRequested),
            line = source.read_line() => {
                match line? {
                    None => Ok(LineInput::Eof),
                    Some(text) => Ok(LineInput::Submitted(text)),
                }
            }
        }
    }

    /// 缓冲是否值得开面板（斜杠开头）。
    fn panel_relevant(buffer: &str) -> bool {
        buffer.starts_with('/')
    }

    /// 刷新一帧：更新候选+重绘；缓冲不以 / 开头时收起面板只留输入行。
    fn refresh(
        &self,
        panel: &mut Panel,
        stdout: &mut std::io::Stdout,
        buffer: &str,
        prev: &mut String,
    ) -> std::io::Result<()> {
        // 先 diff 后写（lead 护栏②）：appended_suffix 纯判定先选定分支，
        // 任何终端写入都发生在分支内，无「先写后判」闪烁窗口。
        if Self::panel_relevant(buffer) {
            *prev = buffer.to_string();
            panel.update(&self.commands_snapshot, buffer);
            if panel.is_open() {
                return panel.render_frame(stdout, self.profile, &self.prompt, buffer, self.width); // task-66 P1-2 第 5 参补齐
            }
        }
        if panel.is_open() {
            panel.collapse(stdout, &self.prompt, buffer)?;
            *prev = buffer.to_string();
        } else if let Some(added) = appended_suffix(prev, buffer) {
            // 追加快速路径（task-42 O(n²) 回显爆炸修复）：打字/粘贴场景新缓冲
            // 以旧缓冲为前缀且光标已在行尾——只写新增尾段，免 \r\e[K 全行重绘
            // （n 字符输入 n² 字节输出的根因；30KB 粘贴 153.8MB→O(n)）。
            write!(stdout, "{added}")?;
            stdout.flush()?;
            *prev = buffer.to_string();
        } else {
            // 非前缀（退格/历史/面板选中替换）：全行重绘（既有语义）
            // 清行原语单点收敛（task-33②）：terminal::clear_active_line 同构
            // 替代手写 \r\e[K——编辑态恒为交互（RawMode 生效前提）。
            crate::render::terminal::clear_active_line(stdout, true)?;
            write!(stdout, "{}{buffer}", self.prompt)?;
            stdout.flush()?;
            *prev = buffer.to_string();
        }
        Ok(())
    }

    /// 读一个 UTF-8 字符：按首字节判 continuation 数并读齐；非法首字节返回 None。
    async fn read_char(
        source: &InputSource,
        first: u8,
    ) -> std::io::Result<Option<char>> {
        let extra = match first {
            0x00..=0x7F => 0,
            0xC0..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF7 => 3,
            _ => return Ok(None),
        };
        let mut bytes = vec![first];
        for _ in 0..extra {
            // continuation 字节逐个过通道；EOF 截断视作非法序列（返回 None）
            match source.read_byte().await? {
                Some(b) => bytes.push(b),
                None => return Ok(None),
            }
        }
        Ok(std::str::from_utf8(&bytes).ok().and_then(|s| s.chars().next()))
    }

    /// ESC 后序列解析（票④ CSI 字节分类修复）：
    ///
    /// * 首字节 50ms 内不到 → **孤立 Esc**（`Escape`——收面板语义）；
    /// * 引导符 `[` (0x5B) → 进入参数循环：参数/中间字节（0x20..=0x3F）继续读，
    ///   final 字节（0x40..=0x7E）按值分发（'A'=Up 'B'=Down 其余 Other）；
    /// * 非 `[` 引导（如 SS3 的 ESC O）→ 吞该单字节按 Other；
    /// * 序列中途 50ms 无字节 → `None`（孤立序列放弃，防 SSH 慢链路卡输入；
    ///   数据不丢原则：放弃的序列不再回灌输入）。
    ///
    /// 根因对照（审三 P2-2）：旧 `drop_escape_sequence` 把 ESC 后**第一个**字节
    /// 直接按 0x40..=0x7E 判 final——引导符 '[' (0x5B) 恰落该区间被误判终止，
    /// 真 final（如方向键的 'A'）反被当普通键打进输入缓冲（方向键残字污染）。
    async fn read_csi(source: &InputSource) -> Option<CsiKey> {
        let lead = match Self::read_byte_timeout(source).await {
            Some(byte) => byte,
            None => return Some(CsiKey::Escape), // ESC 后无后续 = 孤立 Esc
        };
        if lead != b'[' {
            return Some(CsiKey::Other); // 非 CSI 引导序列（吞单字节）
        }
        loop {
            match Self::read_byte_timeout(source).await {
                Some(b) if csi_is_final(b) => {
                    return Some(match b {
                        b'A' => CsiKey::Up,
                        b'B' => CsiKey::Down,
                        _ => CsiKey::Other,
                    });
                }
                // 参数（0x30..=0x3F）与中间（0x20..=0x2F）字节：继续读
                Some(b) if csi_continues(b) => continue,
                _ => return None, // 超时/EOF/非法字节：放弃防卡与死循环
            }
        }
    }

    /// 带超时读单字节（50ms：SSH 慢链路上 CSI 字节间隔可超 30ms，复核 nit）。
    /// **与 repl::esc_interrupt 50ms 同值同因**——孤立 Esc 消歧口径，改动需
    /// 两侧同步（云勘契约 B；grep「50ms 同值」双向可查）。
    async fn read_byte_timeout(source: &InputSource) -> Option<u8> {
        tokio::time::timeout(std::time::Duration::from_millis(50), source.read_byte())
            .await
            .ok()
            .and_then(|res| res.ok())
            .flatten()
    }
}

/// 清终端输入缓冲（业主真机缺陷①「输入泄漏」的修法）：对话期间终端处于
/// cooked 态，用户打的字滞留 TTY 行缓冲，回答结束后进入编辑态前必须丢弃，
/// 否则成为「删不掉的前缀」——cooked 退格按字节删，UTF-8 三字节字要按三下
///（中文必现、英文不现即此因）。TCIFLUSH 只清「已收未读」的输入，不动输出；
/// fd 非终端时 tcflush 失败，调用方忽略（非 TTY 场景无本缺陷，失败不是错误）。
fn flush_stdin_fd(fd: i32) -> std::io::Result<usize> {
    // task-39 批 A 三步：FIONREAD 前置计数（tcflush 内核级丢弃不可感——计数
    // 必须先行）→ TCIFLUSH（原缺陷①修法保留不回退）→ 回报丢弃字节数。
    // 计数失败=N=0=调用方不出提示行（不伪造计数）。
    let mut pending: libc::c_int = 0;
    let counted = unsafe { libc::ioctl(fd, libc::FIONREAD, &mut pending) } == 0;
    let rc = unsafe { libc::tcflush(fd, libc::TCIFLUSH) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(if counted && pending > 0 { pending as usize } else { 0 })
}

/// task-39 批 A 提示行文案（字节口径锁）：FIONREAD 计数为**字节**，UTF-8
/// 多字节下按「字符」报数即伪造计数（报告5 P1-3 退出码 -1 同族病，不再犯）
/// ——测试锁死「字节」用词与 N=0 静默语义。
fn pretype_hint(n: usize) -> String {
    format!("已忽略 {n} 字节预输入")
}

/// termios 原始态守卫：enable 关 ICANON/ECHO（ISIG 保留），Drop 恢复（panic 安全）。
/// task-17④ 起供非交互攒行路径复用（pub(crate)）：语义同为「绕开内核行规程
/// 的行缓冲/回显，由应用自管」——复用即同一套 RAII 恢复纪律。
pub(crate) struct RawMode {
    saved: libc::termios,
    /// 是否真激活（TTY）；非 TTY inactive 守卫——回显逻辑据此守卫（管道
    /// 场景零回显，task-34 修正）。
    pub(crate) active: bool,
}

impl RawMode {
    /// 对 stdin(fd 0) 启用原始态；非 TTY 场景返回 inactive 守卫（无操作）。
    /// task-17④ 起 pub(crate)：非交互攒行路径（input_source::read_line）复用
    pub(crate) fn enable() -> std::io::Result<Self> {
        let mut term = std::mem::MaybeUninit::<libc::termios>::uninit();
        let rc = unsafe { libc::tcgetattr(0, term.as_mut_ptr()) };
        if rc != 0 {
            // 非 TTY（ENOTTY 等）：无编辑态可言；saved 不被使用（active=false）
            return Ok(Self { saved: unsafe { std::mem::zeroed() }, active: false });
        }
        let mut term = unsafe { term.assume_init() };
        term.c_lflag &= !(libc::ICANON | libc::ECHO);
        term.c_cc[libc::VMIN] = 1;
        term.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // 进编辑态前清掉对话期间滞留的 cooked 缓冲（缺陷票①）；失败忽略见上
        // task-39 批 A：预输入知情取舍——丢弃数>0 时 stderr 一行提示（30-⑤
        // 惯例：观测信息走 stderr 让位用户内容）；N=0 不出提示行。
        if let Ok(n) = flush_stdin_fd(0) {
            if n > 0 {
                eprintln!("{}", pretype_hint(n));
            }
        }
        Ok(Self { saved: term, active: true })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if self.active {
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.saved) };
        }
    }
}

#[cfg(test)]
#[path = "lineedit_tests.rs"]
mod tests;
