//! 编辑行状态机核心（编辑行整族升级，lead 批 2026-11-19「一次对表 claude
//! 不零敲」；键位盘点 §8 缺口 1/2/4 承接）：纯逻辑零 IO——行缓冲（char 向量
//! +光标）/光标移动/词操作/清行族/Ctrl-R 历史反查/Ctrl-C 单击清行双击退出
//! 分层，全部离线单测。
//!
//! # 家规落点
//! lineedit.rs 已 477 行超 400 硬顶——本卡新逻辑全部落本文件，lineedit.rs 只
//! 留 IO 壳（读字节→Key→handle_key→Effect 渲染），不再加重。
//!
//! # Ctrl-C 分层（键位盘点 §2 首要缺口）
//! 编辑态 Ctrl-C（SIGINT 路径，ISIG 保留）：行非空=**清行**（不是取消退出）；
//! 行空且 2s 内连击=Interrupted（退出倾向）；行空首击=只清行无动作（红提示
//! 免）。真源=SIGINT 信号（字节路径收不到，ISIG 保留为有意设计）。

use crate::history::LineHistory;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 编辑键（IO 解析层产出；解析与语义分离=状态机可离线全测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// 可见字符（UTF-8 整字）。
    Char(char),
    /// 回车提交。
    Enter,
    /// 退格（删光标前一字符）。
    Backspace,
    /// Delete（删光标处字符）。
    Delete,
    /// Tab（循环补全——交 IO 层面板处理，状态机只回 Effect::None 放行）。
    Tab,
    /// 方向/导航（面板开时 IO 层先喂面板，面板未消费才交状态机）。
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// 孤立 Esc（收面板语义归 IO 层；状态机=取消搜索/无动作）。
    Escape,
    /// Ctrl+A/B/D/E/F/K/R/U/W（0x01/02/04/05/06/0B/12/15/17）。
    CtrlA,
    CtrlB,
    CtrlD,
    CtrlE,
    CtrlF,
    CtrlK,
    CtrlR,
    CtrlU,
    CtrlW,
    /// Alt+B / Alt+F（词级移动，ESC b/f）。
    AltB,
    AltF,
    /// Alt+Backspace（删光标前一词，ESC 0x7F）。
    AltBackspace,
    /// SIGINT（Ctrl-C；信号路径，字节路径收不到）。
    Interrupted,
}

/// 状态机结算效果（IO 层按此渲染/返回）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// 缓冲变化，需重绘输入行。
    Redraw,
    /// 提交（含最终行文本）。
    Submit(String),
    /// EOF/退出（Ctrl-D 空行）。
    Eof,
    /// Ctrl-C 双击连击（行空+2s 内）。
    Quit,
    /// 面板键放行（Tab/面板导航——IO 层处理面板，状态机零动作）。
    None,
}

/// Ctrl-C 连击判定窗（双击退出语义的间隔上限）。
const INTERRUPT_WINDOW: Duration = Duration::from_secs(2);

/// Ctrl-R 反查态（进入后增量 query，Enter 接受命中/Esc 取消）。
struct SearchState {
    query: String,
    /// 当前命中下标（history 内部序；下一条 Ctrl-R 从此更旧）。
    hit: usize,
    /// 进入搜索前的缓冲（取消时还原）。
    origin: Vec<char>,
}

/// 行缓冲+光标状态机（单行编辑器的语义层）。
pub struct EditCore {
    chars: Vec<char>,
    cursor: usize,
    history: Arc<LineHistory>,
    search: Option<SearchState>,
    /// Ctrl-C 上次击键时刻（连击判定）。
    last_interrupt: Option<Instant>,
}

impl EditCore {
    /// 构造（`initial`=回填文本——展开类命令输入框回填，光标落行尾）。
    pub fn new(initial: &str, history: Arc<LineHistory>) -> Self {
        let chars: Vec<char> = initial.chars().collect();
        let cursor = chars.len();
        Self { chars, cursor, history, search: None, last_interrupt: None }
    }

    /// 键入分发（唯一语义入口；IO 层逐键调用）。
    pub fn handle_key(&mut self, key: Key) -> Effect {
        // Ctrl-R 反查态优先吞键（增量 query/Ctrl-R 下一条/Enter 接受/Esc 取消）
        if self.search.is_some() {
            return self.handle_search_key(key);
        }
        match key {
            Key::Char(c) => {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
                Effect::Redraw
            }
            Key::Enter => Effect::Submit(self.text()),
            Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.chars.remove(self.cursor);
                    Effect::Redraw
                } else {
                    Effect::None
                }
            }
            Key::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                    Effect::Redraw
                } else {
                    Effect::None
                }
            }
            Key::CtrlD => {
                if self.chars.is_empty() {
                    Effect::Eof
                } else {
                    // 非空 Ctrl-D=删光标处字符（claude 同语义；空行才是 EOF）
                    self.handle_key(Key::Delete)
                }
            }
            Key::Left | Key::CtrlB => {
                self.cursor = self.cursor.saturating_sub(1);
                Effect::Redraw
            }
            Key::Right | Key::CtrlF => {
                if self.cursor < self.chars.len() {
                    self.cursor += 1;
                }
                Effect::Redraw
            }
            Key::Home | Key::CtrlA => {
                self.cursor = 0;
                Effect::Redraw
            }
            Key::End | Key::CtrlE => {
                self.cursor = self.chars.len();
                Effect::Redraw
            }
            Key::CtrlU => {
                self.chars.drain(..self.cursor);
                self.cursor = 0;
                Effect::Redraw
            }
            Key::CtrlK => {
                self.chars.truncate(self.cursor);
                Effect::Redraw
            }
            Key::CtrlW | Key::AltBackspace => {
                let cut = self.word_start(self.cursor);
                self.chars.drain(cut..self.cursor);
                self.cursor = cut;
                Effect::Redraw
            }
            Key::AltB => {
                self.cursor = self.word_start(self.cursor);
                Effect::Redraw
            }
            Key::AltF => {
                self.cursor = self.word_end(self.cursor);
                Effect::Redraw
            }
            Key::CtrlR => self.begin_search(),
            Key::Interrupted => self.handle_interrupt(),
            // 以下键状态机不消费（IO 层面板/信号面处理）：Tab=补全放行、
            // ↑↓=历史/候选归 IO 层既有 LineHistory 路径、Esc=收面板、Home/End
            // 之外的导航（Up/Down）状态机不持面板态
            Key::Tab | Key::Up | Key::Down | Key::Escape => Effect::None,
        }
    }

    /// Ctrl-C 分层（编辑态）：行非空=清行；行空+连击窗内=Quit；行空首击=无动作。
    fn handle_interrupt(&mut self) -> Effect {
        let now = Instant::now();
        let within = self
            .last_interrupt
            .map(|t| now.duration_since(t) < INTERRUPT_WINDOW)
            .unwrap_or(false);
        self.last_interrupt = Some(now);
        if !self.chars.is_empty() {
            self.chars.clear();
            self.cursor = 0;
            Effect::Redraw
        } else if within {
            Effect::Quit
        } else {
            Effect::None
        }
    }

    /// 进入 Ctrl-R 反查态（空历史=无效果，状态机零感知）。
    fn begin_search(&mut self) -> Effect {
        if self.history.is_empty() {
            return Effect::None;
        }
        let query = self.text();
        self.search = Some(SearchState { query, hit: self.history.len(), origin: self.chars.clone() });
        self.search_step() // 立即以当前行文本反查第一条（shell Ctrl-R 惯例）
    }

    /// 反查态按键处理。
    fn handle_search_key(&mut self, key: Key) -> Effect {
        match key {
            Key::Char(c) => {
                if let Some(search) = self.search.as_mut() {
                    search.query.push(c);
                }
                self.search_step()
            }
            Key::Backspace => {
                if let Some(search) = self.search.as_mut() {
                    search.query.pop();
                }
                self.search_step()
            }
            Key::CtrlR => self.search_step(),
            Key::Enter => {
                let hit = self.search.as_ref().map(|s| s.hit).unwrap_or(self.history.len());
                if let Some(text) = self.history.entry_at(hit) {
                    self.chars = text.chars().collect();
                }
                self.cursor = self.chars.len();
                self.search = None;
                Effect::Redraw
            }
            Key::Escape => {
                // 取消：还原进入前缓冲（原行），退出搜索态
                if let Some(search) = self.search.take() {
                    self.chars = search.origin;
                }
                self.cursor = self.chars.len();
                Effect::Redraw
            }
            Key::Interrupted => {
                self.search = None;
                self.handle_interrupt()
            }
            _ => Effect::None,
        }
    }

    /// 反查步进：从当前 hit 向更旧找含 query 的条目，命中即回显该条。
    fn search_step(&mut self) -> Effect {
        let (query, from) = match self.search.as_ref() {
            Some(s) => (s.query.clone(), s.hit.min(self.history.len())),
            None => return Effect::None,
        };
        if query.is_empty() {
            return Effect::None;
        }
        if let Some((idx, text)) = self.history.search_before(&query, from) {
            if let Some(search) = self.search.as_mut() {
                search.hit = idx;
            }
            self.chars = text.chars().collect();
            self.cursor = self.chars.len();
            Effect::Redraw
        } else {
            Effect::None
        }
    }

    /// 词起点（光标左侧最近词首；跳过尾随空白再跳连续词字符——shell 惯例）。
    fn word_start(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    /// 词终点（光标右侧最近词尾）。
    fn word_end(&self, from: usize) -> usize {
        let mut i = from;
        while i < self.chars.len() && self.chars[i].is_whitespace() {
            i += 1;
        }
        while i < self.chars.len() && !self.chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    /// 当前行文本。
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// 光标前正文（渲染定位：左侧列宽计算用）。
    pub fn left_cols(&self, width_of: impl Fn(char) -> usize) -> usize {
        self.chars[..self.cursor].iter().copied().map(width_of).sum()
    }

    /// 行全文 char 切片（渲染遍历用）。
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    /// 光标 char 下标（渲染定位）。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 反查态提示行（IO 层渲染到输入行上方；None=非搜索态）。
    pub fn search_hint(&self) -> Option<String> {
        self.search.as_ref().map(|s| {
            let text: String = self.chars.iter().collect();
            format!("(反查 `{}`) {}", s.query, text)
        })
    }

    /// 是否在反查态（IO 层渲染分支）。
    pub fn searching(&self) -> bool {
        self.search.is_some()
    }
}
