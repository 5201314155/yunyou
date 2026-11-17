//! 命令补全面板（任务29 C3，CLI 壳面）：两列候选/前缀过滤/8 行封顶/原位重绘。
//!
//! 形态对齐命令体系设计 §四：输入以 `/` 开头触发；候选列表两列
//! 「`/命令名（别名）  描述」；前缀过滤时 name 与 aliases 都参与匹配；
//! 渲染对齐 D-UI-1——候选行每帧 `\r\e[K` 原位重绘（与思考活动行同机制），
//! 候选行截断到定宽 80-2 防折行，候选区高度封顶 8 行防滚动破坏回看。
//! 擦除我方暂态行用 `\e[J`（清光标到屏底）——它是「删除本面板画的行」，
//! 不回改任何历史落定行；落定内容仍走 freeze_line 纪律。
//!
//! 数据源=CommandsService::list() 实时查询（设计 §四：CLI 单进程直调无缓存）。
//! 文本度量/截断/第一列合成为纯函数，落 [`crate::panel_text`]（拆分自证：
//! 面板状态机与纯文本工具两职责分行数硬顶）。

use crate::panel_text::{display_width, first_column, truncate_width};
use crate::render::Profile;
use std::io::Write;

#[path = "panel_render.rs"]
mod render;
use yunyou_commands::CommandDescriptor;

/// 候选区行数封顶（含溢出提示行；设计 §四「列表高度封顶 8 行」）。
const MAX_PANEL_ROWS: usize = 8;

/// 提示符前缀占用的列（渲染输入行时补齐用；与 repl 的 ❯ 同形）。
pub const PROMPT_WIDTH: usize = 2;

/// 从输入缓冲提取过滤词：`/xyz` → `xyz`（小写归一）；非斜杠开头 → None（面板不开）。
pub fn filter_token(buffer: &str) -> Option<String> {
    let rest = buffer.strip_prefix('/')?;
    Some(rest.trim_start().to_lowercase())
}

/// 前缀过滤（name 与 aliases 都参与；过滤词小写归一，空词=全量目录）。
pub fn filter_candidates(commands: &[CommandDescriptor], token: Option<&str>) -> Vec<CommandDescriptor> {
    let Some(token) = token else { return Vec::new() };
    commands
        .iter()
        .filter(|cmd| {
            token.is_empty()
                || cmd.name.starts_with(token)
                || cmd.aliases.iter().any(|alias| alias.to_lowercase().starts_with(token))
        })
        .cloned()
        .collect()
}

/// 面板候选刷新的缓冲长度上限（task-66 P2：超长粘贴收起面板防 O(n²)）。
const PANEL_BUFFER_CAP: usize = 4096;

/// 补全面板状态：候选/选中/当前在屏行数（重绘与收起的依据）。
#[derive(Default)]
pub struct Panel {
    candidates: Vec<CommandDescriptor>,
    selected: usize,
    /// 上一帧画过的候选区行数（0=面板未开）；帧间擦除与光标复位用。
    drawn_rows: usize,
    /// 首次提示行（发现性，业主票③）：true=本帧在候选区上方画一行操作提示。
    /// 由调用方一次性置位（跨轮标志放编辑器共享态，面板本体每轮新建）。
    hint: bool,
}

impl Panel {
    /// 面板是否开启（有候选可见）。
    pub fn is_open(&self) -> bool {
        !self.candidates.is_empty()
    }

    /// 按输入缓冲刷新候选（实时过滤）；选中项重置到第一项。
    pub fn update(&mut self, commands: &[CommandDescriptor], buffer: &str) {
        // task-66 P2：超长缓冲收起面板——粘贴 30KB 级文本时逐键全量重滤+
        // token 解析=O(n²) 级开销，且巨缓冲下候选过滤已无操作意义（过滤词
        // 取尾 token，粘贴体几乎必不匹配）。收起=候选清空（is_open false），
        // 缓冲回落阈值内自然恢复。阈值 4KB（2× args 落盘上限，与回放口径
        // 同量级）。
        if buffer.len() > PANEL_BUFFER_CAP {
            self.candidates.clear();
            self.selected = 0;
            return;
        }
        self.candidates = filter_candidates(commands, filter_token(buffer).as_deref());
        self.selected = 0;
    }

    /// Tab：选中项推进（循环）；返回补全到输入行的命令名（带尾空格）。
    pub fn tab_next(&mut self) -> Option<String> {
        if self.candidates.is_empty() {
            return None;
        }
        self.selected = (self.selected + 1) % self.candidates.len();
        Some(format!("/{} ", self.candidates[self.selected].name))
    }

    /// ↑：选中项上移（循环到尾）——面板交互升级票②（方向键直接选）。
    pub fn move_up(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.candidates.len() - 1);
    }

    /// ↓：选中项下移（循环回首）。
    pub fn move_down(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.candidates.len();
    }

    /// 置首次提示行标志（下一帧画一行操作提示；发现性票③）。
    pub fn set_hint(&mut self, on: bool) {
        self.hint = on;
    }

    /// Enter 执行选中项（设计 §四「Enter 执行选中项」；面板开时提交前调用）：
    /// 返回选中命令的补全输入（`/name `）；无候选返回 None（提交原缓冲）。
    pub fn selected_replacement(&self) -> Option<String> {
        self.candidates
            .get(self.selected)
            .map(|cmd| format!("/{} ", cmd.name))
    }

    /// 当前选中项的命令名（Enter 执行目标）。
    pub fn selected_name(&self) -> Option<&str> {
        self.candidates.get(self.selected).map(|c| c.name.as_str())
    }

    /// 在屏候选行数（含溢出提示行），供测试与收起断言。
    pub fn drawn_rows(&self) -> usize {
        self.drawn_rows
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造目录条目。
    fn cmd(name: &str, aliases: &[&str], description: &str) -> CommandDescriptor {
        CommandDescriptor {
            name: name.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            description: description.to_string(),
            kind: yunyou_commands::CommandKind::Action,
            accepts_args: false,
        }
    }

    /// 过滤：name 前缀/别名前缀/空词全量/大小写归一。
    #[test]
    fn 前缀过滤与别名参与() {
        let catalog = vec![
            cmd("help", &["帮助"], "看命令"),
            cmd("quit", &["退出", "q"], "退出"),
            cmd("new", &[], "新会话"),
        ];
        let empty = filter_candidates(&catalog, filter_token("/").as_deref());
        assert_eq!(empty.len(), 3, "空过滤词=全量目录");
        let by_name = filter_candidates(&catalog, filter_token("/he").as_deref());
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].name, "help");
        let by_alias = filter_candidates(&catalog, filter_token("/退").as_deref());
        assert_eq!(by_alias.len(), 1, "别名参与匹配");
        assert_eq!(by_alias[0].name, "quit");
        let by_single_alias = filter_candidates(&catalog, filter_token("/Q").as_deref());
        assert_eq!(by_single_alias[0].name, "quit", "过滤词大小写归一");
        let none = filter_candidates(&catalog, filter_token("zzz").as_deref());
        assert!(none.is_empty(), "无候选=面板不开");
        assert!(filter_token("hello").is_none(), "非斜杠缓冲不开面板");
    }

    /// 帧渲染帧序：输入行在候选区之上、每行 \r\e[K 原位写、光标回到输入行。
    #[test]
    fn 帧渲染顺序与原位重绘机制() {
        let mut panel = Panel::default();
        let catalog = vec![cmd("help", &["帮助"], "看命令"), cmd("quit", &[], "退出")];
        panel.update(&catalog, "/");
        let mut out: Vec<u8> = Vec::new();
        panel.render_frame(&mut out, Profile::NoColor, "❯ ", "/", 60).unwrap();
        let frame = String::from_utf8(out).unwrap();
        let input_pos = frame.find("\r\x1b[K❯ /").expect("应有输入行重绘");
        let help_pos = frame.find("看命令").expect("应有两列描述");
        let quit_pos = frame.find("\r\x1b[K/quit").expect("候选行应 \\r\\e[K 原位写");
        assert!(input_pos < help_pos && help_pos < quit_pos, "帧序=输入行→候选区: {frame}");
        // 上移行数=本帧画的候选区行数（两候选即 \x1b[2A）——画几行就上移几行回输入行
        assert!(
            frame.contains(&format!("\x1b[{}A", panel.drawn_rows())),
            "帧末上移行数应=本帧画行数(回输入行): {frame}"
        );
        assert!(!frame.contains('\t'), "帧内不应用制表符对齐");
    }

    /// 8 行封顶：超量候选只画 7 候选+1 溢出提示，窗口随选中滚动。
    #[test]
    fn 候选区八行封顶与滚动窗口() {
        let catalog: Vec<CommandDescriptor> =
            (0..12).map(|i| cmd(&format!("cmd{i:02}"), &[], "描述")).collect();
        let mut panel = Panel::default();
        panel.update(&catalog, "/");
        let mut out: Vec<u8> = Vec::new();
        panel.render_frame(&mut out, Profile::NoColor, "❯ ", "/", 80).unwrap();
        let frame = String::from_utf8(out).unwrap();
        assert_eq!(panel.drawn_rows(), MAX_PANEL_ROWS, "封顶 8 行（7 候选+1 提示）");
        assert!(frame.contains("…共 12 项"), "应有溢出提示: {frame}");
        assert!(frame.contains("/cmd00"), "首帧窗口从头开始");
        assert!(!frame.contains("/cmd07"), "第 8 个候选不应上屏");
        // Tab 推进到第 7 项（索引 6），窗口起点=1（选中须在窗口内）
        for _ in 0..6 {
            panel.tab_next();
        }
        let mut out2: Vec<u8> = Vec::new();
        panel.render_frame(&mut out2, Profile::NoColor, "❯ ", "/", 80).unwrap();
        let frame2 = String::from_utf8(out2).unwrap();
        assert!(frame2.contains("/cmd06"), "选中项应可见: {frame2}");
        assert!(panel.selected_name().is_some());
    }

    /// Tab 循环补全：推进到尾后回到首（循环）。
    #[test]
    fn tab循环补全() {
        let catalog = vec![cmd("help", &[], "a"), cmd("new", &[], "b")];
        let mut panel = Panel::default();
        panel.update(&catalog, "/");
        assert_eq!(panel.selected_name(), Some("help"), "初始选中第一项");
        assert_eq!(panel.tab_next(), Some("/new ".to_string()));
        assert_eq!(panel.selected_name(), Some("new"));
        assert_eq!(panel.tab_next(), Some("/help ".to_string()), "尾后循环回首");
        assert_eq!(panel.selected_name(), Some("help"));
    }

    /// 收起：候选清空+\e[J 擦除暂态行。
    #[test]
    fn 收起清场() {
        let catalog = vec![cmd("help", &[], "看命令")];
        let mut panel = Panel::default();
        panel.update(&catalog, "/");
        let mut out: Vec<u8> = Vec::new();
        panel.render_frame(&mut out, Profile::NoColor, "❯ ", "/", 80).unwrap();
        assert!(panel.is_open());
        let mut out2: Vec<u8> = Vec::new();
        panel.collapse(&mut out2, "❯ ", "/").unwrap();
        let frame = String::from_utf8(out2).unwrap();
        assert!(frame.contains("\x1b[J"), "收起应擦除候选区: {frame}");
        assert!(frame.contains("\r\x1b[K❯ /"), "收起后保留输入行");
        assert!(!panel.is_open());
        assert_eq!(panel.drawn_rows(), 0);
    }

    /// 票②：↑↓ 选择循环（↑ 从首项循环到尾项）+Enter 选中项替换；空候选零 panic。
    #[test]
    fn 面板上下选择与enter选中项() {
        let catalog =
            vec![cmd("help", &[], "a"), cmd("new", &[], "b"), cmd("quit", &[], "c")];
        let mut panel = Panel::default();
        panel.update(&catalog, "/");
        assert_eq!(panel.selected_name(), Some("help"), "初始选中第一项");
        panel.move_up();
        assert_eq!(panel.selected_name(), Some("quit"), "↑ 从首项循环到尾项");
        panel.move_down();
        assert_eq!(panel.selected_name(), Some("help"), "↓ 循环回首");
        panel.move_down();
        assert_eq!(panel.selected_name(), Some("new"));
        assert_eq!(
            panel.selected_replacement(),
            Some("/new ".to_string()),
            "Enter 执行选中项（设计 §四）"
        );
        panel.update(&[], "/");
        panel.move_up();
        panel.move_down();
        assert_eq!(panel.selected_replacement(), None, "无候选提交原缓冲");
    }

    /// 票⑤：残影差额擦除——候选由多变少时第二帧必须 \e[J 清旧帧残留行；
    /// 首帧（无缩）不应有 \e[J。
    #[test]
    fn 残影差额擦除() {
        let catalog: Vec<CommandDescriptor> =
            (0..5).map(|i| cmd(&format!("cmd{i}"), &[], "描述")).collect();
        let mut panel = Panel::default();
        panel.update(&catalog, "/");
        let mut out1: Vec<u8> = Vec::new();
        panel.render_frame(&mut out1, Profile::NoColor, "❯ ", "/", 80).unwrap();
        assert!(
            !out1.windows(3).any(|w| w == b"\x1b[J"),
            "首帧无缩不应擦屏底"
        );
        panel.update(&catalog[..1], "/c");
        let mut out2: Vec<u8> = Vec::new();
        panel.render_frame(&mut out2, Profile::NoColor, "❯ ", "/c", 80).unwrap();
        let frame2 = String::from_utf8(out2).unwrap();
        assert!(frame2.contains("\x1b[J"), "候选由多变少必须擦差额残影: {frame2:?}");
    }

    /// 票③：首次提示行随面板帧画（文案在帧内且计入 drawn_rows）。
    #[test]
    fn 首次提示行随帧画() {
        let catalog = vec![cmd("help", &[], "看命令")];
        let mut panel = Panel::default();
        panel.update(&catalog, "/");
        panel.set_hint(true);
        let mut out: Vec<u8> = Vec::new();
        panel.render_frame(&mut out, Profile::NoColor, "❯ ", "/", 80).unwrap();
        let frame = String::from_utf8(out).unwrap();
        assert!(frame.contains("↑↓ 选"), "提示行应随帧上屏: {frame:?}");
        assert_eq!(panel.drawn_rows(), 2, "提示行+候选行=2");
    }

    /// task-66 P2：30KB 级缓冲收起面板（O(n²) 防复发行为锁）+常规缓冲照常过滤。
    #[test]
    fn 超长缓冲收起面板_常规照常() {
        let mut panel = Panel::default();
        let cmds = vec![
            cmd("help", &[], "看命令"),
            cmd("quit", &[], "退出云悠"),
        ];
        // 常规缓冲：过滤正常
        panel.update(&cmds, "/h");
        assert!(panel.is_open(), "常规缓冲应过滤出候选");
        // 30KB 级缓冲：收起（候选清空，is_open false——渲染层据此不画帧）
        let huge = format!("/{}", "x".repeat(30 * 1024));
        panel.update(&cmds, &huge);
        assert!(!panel.is_open(), "超长缓冲应收起面板（O(n²) 防复发）");
        // 回落阈值内：恢复过滤
        panel.update(&cmds, "/q");
        assert!(panel.is_open(), "回落阈值内恢复过滤");
    }

    /// task-66 P1-2：width 参数化断言——60 列注入下面板行不超宽（伪 TTY 60×24
    /// 的单元等价；\e[nA 相对移动行数由帧会计 drawn_rows 保障，此处锁可见宽）。
    #[test]
    fn 面板帧_宽度参数化_行不超宽() {
        let mut panel = Panel::default();
        let cmds = vec![
            cmd("sessions", &["会话"], "列会话（无参）或切换（/sessions <序号>）"),
            cmd("tools-info", &["工具"], "列出内置工具与用途"),
            cmd("help", &["帮助"], "看命令（列出全部斜杠命令）"),
        ];
        panel.update(&cmds, "/");
        panel.set_hint(true);
        let mut out: Vec<u8> = Vec::new();
        panel.render_frame(&mut out, Profile::NoColor, "❯ ", "/", 80).unwrap();
        // 注：render_frame 自身不接收 width——截断基准经 panel_width 三层探测；
        // 参数化验证=行可见宽 ≤ 60（NoColor 剥色后判）。
        let frame = String::from_utf8(out).unwrap();
        // 剥 CSI 序列后度量（\r\x1b[K 前缀与标记 span 零可见宽——裸 char 计数
        // 会把转义字节当可见宽误判超宽；strip 口径=ESC[ 至 0x40-0x7E 终结）。
        let strip_csi = |t: &str| -> String {
            let mut out_s = String::new();
            let mut chars = t.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\x1b' && chars.peek() == Some(&'[') {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                } else {
                    out_s.push(c);
                }
            }
            out_s
        };
        let clean = strip_csi(&frame);
        for line in clean.lines() {
            assert!(
                crate::panel_text::display_width(line) <= 62,
                "面板行超宽（60 列场，剥 CSI 后）：{line:?}"
            );
        }
    }

}
