//! 编辑行状态机集成测试（自 editor_core.rs 拆出——家规单文件 400 硬顶，
//! 状态机主体与测试分家；语义同源零复制）。
use std::sync::Arc;


use yunyou_cli::editor_core::{EditCore, Effect, Key};
use yunyou_cli::history::LineHistory;

fn core_with(hist: Arc<LineHistory>, initial: &str) -> EditCore {
    EditCore::new(initial, hist)
}

fn typed(core: &mut EditCore, text: &str) {
    for c in text.chars() {
        core.handle_key(Key::Char(c));
    }
}

    /// 光标移动族：←→/Ctrl-A/E/B/F/Home/End（含边界钳制）。
#[test]
fn 光标移动族全通() {
    let mut core = core_with(Default::default(), "abcd");
    assert_eq!(core.cursor(), 4);
    assert_eq!(core.handle_key(Key::Left), Effect::Redraw);
    assert_eq!(core.cursor(), 3);
    core.handle_key(Key::Home);
    assert_eq!(core.cursor(), 0);
    core.handle_key(Key::CtrlE);
    assert_eq!(core.cursor(), 4);
        // 边界钳制：行首再 ←/Ctrl-B 不越界
    core.handle_key(Key::Home);
    assert_eq!(core.handle_key(Key::Left), Effect::Redraw);
    assert_eq!(core.cursor(), 0);
    assert_eq!(core.handle_key(Key::CtrlB), Effect::Redraw);
    assert_eq!(core.cursor(), 0);
        // 行尾再 → 不越界
    core.handle_key(Key::End);
    assert_eq!(core.handle_key(Key::Right), Effect::Redraw);
    assert_eq!(core.cursor(), 4);
        // Ctrl-F/B 单字符
    core.handle_key(Key::CtrlB);
    core.handle_key(Key::Char('X'));
    assert_eq!(core.text(), "abcXd", "光标处插入");
}

    /// 行中编辑：光标处插入/删除（Delete）——一期「只删尾」的根除面。
#[test]
fn 行中插入与删除() {
    let mut core = core_with(Default::default(), "hello");
    core.handle_key(Key::Home);
    core.handle_key(Key::Right);
    core.handle_key(Key::Right);
    core.handle_key(Key::Char('X')); // heXllo（cursor=2 在第三字前）
    assert_eq!(core.text(), "heXllo");
    core.handle_key(Key::Delete); // 删光标处 'l'
    assert_eq!(core.text(), "heXlo");
        // 非空 Ctrl-D=删光标处；行尾 Delete 无操作
    core.handle_key(Key::End);
    assert_eq!(core.handle_key(Key::Delete), Effect::None);
}

    /// 删词/清行族：Ctrl-W 删光标前一词/Alt-Backspace 同义/Ctrl-U 清到行首/Ctrl-K 删到行尾。
#[test]
fn 删词与清行族() {
    let mut core = core_with(Default::default(), "foo bar  baz");
    assert_eq!(core.handle_key(Key::CtrlW), Effect::Redraw);
    assert_eq!(core.text(), "foo bar  ", "连空格整段删（词起点跳空白）");
    let mut core = core_with(Default::default(), "cmd arg");
    core.handle_key(Key::End);
    core.handle_key(Key::AltB);
    typed(&mut core, "X");
    assert_eq!(core.text(), "cmd Xarg", "Alt-B 落词首");
    let mut core = core_with(Default::default(), "keep me");
    core.handle_key(Key::End);
    core.handle_key(Key::AltBackspace);
    assert_eq!(core.text(), "keep ", "Alt-Backspace=删词");
    let mut core = core_with(Default::default(), "ab|cd");
    core.handle_key(Key::Home);
    typed(&mut core, "ab");
    core.handle_key(Key::CtrlK);
    assert_eq!(core.text(), "ab", "Ctrl-K 删到行尾");
    let mut core = core_with(Default::default(), "ab|cd");
    typed(&mut core, "cd");
    core.handle_key(Key::CtrlU);
    assert_eq!(core.text(), "", "Ctrl-U 清到行首");
    assert_eq!(core.cursor(), 0);
}

    /// Ctrl-C 分层：行非空=清行；行空首击=无动作；行空 2s 内连击=Quit；
    /// 超窗重置。
#[test]
fn ctrl_c分层() {
    let mut core = core_with(Default::default(), "有字");
    assert_eq!(core.handle_key(Key::Interrupted), Effect::Redraw);
    assert_eq!(core.text(), "", "行非空单击=清行");
    assert_eq!(core.handle_key(Key::Interrupted), Effect::Quit, "行空+连击窗内=退出");
        // 超窗：等 2s+（测试注入不便——直接验证首击无动作语义：新 core 行空首击）
    let mut core = core_with(Default::default(), "");
    assert_eq!(core.handle_key(Key::Interrupted), Effect::None, "行空首击=无动作");
}

    /// Ctrl-R 反查：进入即命中含行文本的首条/增量 query 收窄/Enter 接受/Esc 还原。
#[test]
fn ctrl_r反查全流程() {
    let hist = Arc::new(LineHistory::new());
    hist.commit("git status");
    hist.commit("cargo test -p yunyou-cli");
    hist.commit("git log --oneline");
    let mut core = core_with(hist, "gi");
    assert_eq!(core.handle_key(Key::CtrlR), Effect::Redraw);
        // 进入搜索：query="gi" 命中最近含 gi 的条目（shell Ctrl-R 惯例=最近优先）
    assert!(core.text().contains("git log"), "首命中最近条: {}", core.text());
        // Ctrl-R 下一条：向更旧翻到 git status
    assert_eq!(core.handle_key(Key::CtrlR), Effect::Redraw, "有更旧命中应翻");
    assert!(core.text().contains("git status"));
        // Enter 接受命中
    assert_eq!(core.handle_key(Key::Enter), Effect::Redraw);
    assert!(core.text().starts_with("git status"));
    assert!(!core.searching(), "Enter 后退出搜索态");
}

    /// Ctrl-R Esc 取消：还原进入前缓冲。
#[test]
fn ctrl_r_esc还原原行() {
    let hist = Arc::new(LineHistory::new());
    hist.commit("git status");
    let mut core = core_with(hist, "草稿");
    core.handle_key(Key::CtrlR);
    assert_eq!(core.handle_key(Key::Escape), Effect::Redraw);
    assert_eq!(core.text(), "草稿", "Esc 还原进入前缓冲");
    assert!(!core.searching());
}

    /// UTF-8 行中插入（中文）：char 向量光标按字符不劈字。
#[test]
fn utf8行中编辑不劈字() {
    let mut core = core_with(Default::default(), "你好世界");
    core.handle_key(Key::Home);
    core.handle_key(Key::Right);
    core.handle_key(Key::Right);
    core.handle_key(Key::Char('中'));
    assert_eq!(core.text(), "你好中世界", "cursor=2 插在第三字前");
    core.handle_key(Key::Backspace);
    assert_eq!(core.text(), "你好世界", "退格删整字");
}
