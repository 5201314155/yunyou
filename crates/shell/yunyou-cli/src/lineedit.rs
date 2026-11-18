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
use crate::editor_core;
use crate::editor_keys;
use crate::editor_keys::{flush_stdin_fd, pretype_hint};
use crate::editor_render;
use crate::panel::Panel;
// RawMode 旧路径 re-export（input_source.rs/repl.rs 消费方零改动，家规五：
// 改复用模块影响面=调用方不变）
pub(crate) use editor_keys::RawMode;

use crate::input_source::InputSource;
use crate::render::Profile;
use std::io::{Write, IsTerminal};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use yunyou_commands::{CommandDescriptor, CommandsService};

/// CSI final 字节判定（ECMA-48：0x40..=0x7E；票④测试锚点）。
fn csi_is_final(byte: u8) -> bool {
    (0x40..=0x7E).contains(&byte)
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
    pub async fn read_line(
        &self,
        initial: Option<String>,
        scheduled: Option<&mut tokio::sync::mpsc::Receiver<String>>,
    ) -> std::io::Result<LineInput> {
        // task-73 A1：会话级 cbreak（外层 repl_loop 持守卫）——此处 enable=
        // 幂等重入（tcsetattr 同值无害），Drop 只恢复到会话级形态而非 cooked
        // （防逐行守卫先 drop 把会话守卫的 cbreak 打回 cooked）。
        let _raw = RawMode::enable()?;
        // task-39 批 A 提示行（原语义保留，调用点随编辑行手术回植）：预输入
        // 字节计数>0 时提示（迁入件 editor_keys 供数）
        if std::io::stdin().is_terminal() {
        if let Ok(n) = flush_stdin_fd(std::io::stdin().as_raw_fd()) {
            if n > 0 {
                eprintln!("{}", pretype_hint(n));
            }
        }
        }
        let mut stdout = std::io::stdout();
        let mut panel = Panel::default();
        let mut buffer = initial.unwrap_or_default();
        // 编辑行整族升级：语义层状态机（光标/删词/清行/反查/Ctrl-C 分层）
        let mut core = editor_core::EditCore::new(&buffer, self.history.clone());
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
        // 定时注入臂（proto v1.1 真投递，task-14 续）：到期消息=合成用户输入
        // （回合边界排队投递同律——回合中到期则回合结束回到读行时注入）
        let mut scheduled = scheduled;
        loop {
            tokio::select! {
                biased;
                // Ctrl-C 分层（编辑行整族升级）：行非空=清行（非退出）；
                // 行空+2s 连击=退出倾向（core.handle_key 统一裁决）
                _ = tokio::signal::ctrl_c() => {
                    match core.handle_key(editor_core::Key::Interrupted) {
                        editor_core::Effect::Redraw => editor_render::redraw(&mut stdout, &self.prompt, core.chars(), core.cursor())?,
                        editor_core::Effect::Quit => {
                            writeln!(stdout)?;
                            return Ok(LineInput::Interrupted);
                        }
                        _ => {}
                    }
                }
                _ = self.exit.cancelled() => return Ok(LineInput::ExitRequested),
                msg = async {
                    match scheduled.as_deref_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    // 上屏注入痕迹（提示行换行防撕裂；与回合输出互斥由单线程终端写保证）
                    writeln!(stdout, "
[定时触发] 到期——即将执行注入任务")?;
                    stdout.flush()?;
                    if panel.is_open() {
                        panel.collapse(&mut stdout, &self.prompt, &buffer)?;
                    }
                    writeln!(stdout)?;
                    stdout.flush()?;
                    let text = msg.unwrap_or_default();
                    return Ok(LineInput::Submitted(format!("[定时触发] {text}")));
                }
                read = source.read_byte() => {
                    let Some(b) = read? else {
                        return Ok(LineInput::Eof);
                    };
                    // 字节→Key（编辑行整族升级：解析在 editor_keys，语义在
                    // editor_core——IO 壳只做分发与渲染）
                    let key = match b {
                        0x1B => {
                            // 孤立 Esc（50ms 无后续）=收面板/取消搜索；有后续=
                            // CSI/SS3/Alt 族（解析统一在 editor_keys）
                            match editor_keys::read_escape_key(source).await {
                                None => continue,
                                Some(editor_core::Key::Escape) => {
                                    if panel.is_open() {
                                        panel.collapse(&mut stdout, &self.prompt, &buffer)?;
                                    }
                                    continue;
                                }
                                Some(k) => k,
                            }
                        }
                        _ => match editor_keys::plain_key(b) {
                            Some(key) => key,
                            None => {
                                // 可见字符：UTF-8 整字拼读
                                let Some(ch) = Self::read_char(&source, b).await? else {
                                    continue;
                                };
                                editor_core::Key::Char(ch)
                            }
                        },
                    };
                    // 面板优先消费的键（面板开才有意义）：Tab/↑↓/Enter/Esc——
                    // 既有面板语义零变更；其余键交状态机
                    let panel_consumed = match (&key, panel.is_open()) {
                        (editor_core::Key::Tab, true) => {
                            if let Some(completed) = panel.tab_next() {
                                buffer = completed;
                            }
                            true
                        }
                        (editor_core::Key::Up, true) => {
                            panel.move_up();
                            true
                        }
                        (editor_core::Key::Down, true) => {
                            panel.move_down();
                            true
                        }
                        (editor_core::Key::Enter, true) => {
                            // Enter 执行选中项（/he 提交 /help 防误发，既有语义）
                            if let Some(replacement) = panel.selected_replacement() {
                                buffer = replacement;
                            }
                            panel.collapse(&mut stdout, &self.prompt, &buffer)?;
                            writeln!(stdout)?;
                            stdout.flush()?;
                            return Ok(LineInput::Submitted(buffer));
                        }
                        _ => false,
                    };
                    if panel_consumed {
                        self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                        continue;
                    }
                    // 状态机裁决（光标/删词/清行/反查/Ctrl-C 分层）
                    let effect = core.handle_key(key);
                    match effect {
                        editor_core::Effect::None => {}
                        editor_core::Effect::Redraw => {
                            buffer = core.text();
                            if core.searching() {
                                if let Some(hint) = core.search_hint() {
                                    editor_render::search_hint(&mut stdout, &hint)?;
                                }
                                editor_render::redraw(&mut stdout, &self.prompt, core.chars(), core.cursor())?;
                                prev = buffer.clone();
                            } else if panel.is_open() {
                                self.refresh(&mut panel, &mut stdout, &buffer, &mut prev)?;
                            } else if editor_render::tail_extend(&mut stdout, &prev, &buffer)? && core.cursor() == core.chars().len() {
                                prev = buffer.clone();
                            } else {
                                editor_render::redraw(&mut stdout, &self.prompt, core.chars(), core.cursor())?;
                                prev = buffer.clone();
                            }
                        }
                        editor_core::Effect::Submit(text) => {
                            buffer = text;
                            if panel.is_open() {
                                panel.collapse(&mut stdout, &self.prompt, &buffer)?;
                            }
                            writeln!(stdout)?;
                            stdout.flush()?;
                            return Ok(LineInput::Submitted(buffer));
                        }
                        editor_core::Effect::Eof => {
                            writeln!(stdout)?;
                            stdout.flush()?;
                            return Ok(LineInput::Eof);
                        }
                        editor_core::Effect::Quit => {
                            writeln!(stdout)?;
                            stdout.flush()?;
                            return Ok(LineInput::Interrupted);
                        }
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
}

fn appended_suffix<'a>(prev: &str, buffer: &'a str) -> Option<&'a str> {
    (buffer.len() > prev.len()
        && buffer.starts_with(prev)
        && buffer.is_char_boundary(prev.len()))
    .then(|| &buffer[prev.len()..])
}
