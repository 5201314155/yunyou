//! 编辑键解析（编辑行整族升级，键位盘点 §8 缺口 1 接入）：字节/ESC 序列 →
//! [`Key`]。自 lineedit.rs 按职责拆出（477 行超硬顶，家规四：新功能新文件+
//! 旧文件瘦身）；termios 工具（RawMode/flush_stdin_fd/pretype_hint）随批迁入。

use super::editor_core::Key;
use super::input_source::InputSource;

/// CSI/SS3 解析结果（自 lineedit.rs 迁入并扩展：编辑行整族新增 Left/Right/
/// Home/End/Del 与 SS3 形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CsiKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    Escape,
    Other,
}

/// ESC 后序列解析（票④ CSI 字节分类修复语义原样迁入；扩展见 [`CsiKey`]）：
///
/// * 首字节 50ms 内不到 → 孤立 Esc（`Escape`——收面板语义）；
/// * 引导符 `[` → 参数循环至 final：A/B/C/D=上/下/左/右，H/F=Home/End，
///   `3~`=Delete（参数字节后 `~` final），其余 Other；
/// * SS3（ESC O）→ O 后 A-D/H/F 同义（xterm 应用键盘态）；
/// * 序列中途 50ms 无字节 → `None`（孤立序列放弃防卡）。
pub(crate) async fn read_csi(source: &InputSource) -> Option<CsiKey> {
    let lead = read_byte_timeout(source).await?;
    if lead == b'O' {
        // SS3：单 final 字节（xterm 应用光标键）
        return read_byte_timeout(source).await.map(|f| match f {
            b'A' => CsiKey::Up,
            b'B' => CsiKey::Down,
            b'C' => CsiKey::Right,
            b'D' => CsiKey::Left,
            b'H' => CsiKey::Home,
            b'F' => CsiKey::End,
            _ => CsiKey::Other,
        });
    }
    if lead != b'[' {
        return Some(CsiKey::Other); // 非 CSI 引导序列（吞单字节）
    }
    let mut param: Option<u8> = None;
    loop {
        match read_byte_timeout(source).await {
            Some(b) if csi_is_final(b) => {
                return Some(match (b, param) {
                    (b'A', _) => CsiKey::Up,
                    (b'B', _) => CsiKey::Down,
                    (b'C', _) => CsiKey::Right,
                    (b'D', _) => CsiKey::Left,
                    (b'H', _) => CsiKey::Home,
                    (b'F', _) => CsiKey::End,
                    (b'~', Some(p)) if p == b'3' => CsiKey::Delete,
                    (b'~', Some(p)) if p == b'1' || p == b'7' => CsiKey::Home,
                    (b'~', Some(p)) if p == b'4' || p == b'8' => CsiKey::End,
                    _ => CsiKey::Other,
                });
            }
            // 参数（0x30..=0x3F）与中间（0x20..=0x2F）字节：继续读（记住参数）
            Some(b) if csi_continues(b) => {
                if (b'0'..=b'9').contains(&b) {
                    param.get_or_insert(b);
                }
                continue;
            }
            _ => return None, // 超时/EOF/非法字节：放弃防卡与死循环
        }
    }
}

/// 字节→Key（非 ESC 字节段；IO 层主循环逐键调用）。
pub(crate) fn plain_key(b: u8) -> Option<Key> {
    match b {
        b'\r' | b'\n' => Some(Key::Enter),
        0x7F => Some(Key::Backspace),
        0x01 => Some(Key::CtrlA),
        0x02 => Some(Key::CtrlB),
        0x04 => Some(Key::CtrlD),
        0x05 => Some(Key::CtrlE),
        0x06 => Some(Key::CtrlF),
        0x0B => Some(Key::CtrlK),
        0x12 => Some(Key::CtrlR),
        0x15 => Some(Key::CtrlU),
        0x17 => Some(Key::CtrlW),
        b if b >= 0x20 => None, // 可见字符：IO 层走 UTF-8 整字拼读后给 Char
        _ => None,
    }
}

/// ESC 第二字节 → Key（Alt 族；50ms 消歧窗内读到后续字节才成立，孤立 Esc
/// 由调用方先判）。
pub(crate) fn alt_key(second: u8) -> Option<Key> {
    match second {
        b'b' => Some(Key::AltB),
        b'f' => Some(Key::AltF),
        0x7F => Some(Key::AltBackspace),
        _ => None,
    }
}

/// 带超时读单字节（50ms：SSH 慢链路上 CSI 字节间隔可超 30ms，复核 nit）。
/// **与 repl::esc_interrupt 50ms 同值同因**——孤立 Esc 消歧口径，改动需
/// 两侧同步（云勘契约 B；grep「50ms 同值」双向可查）。
pub(crate) async fn read_byte_timeout(source: &InputSource) -> Option<u8> {
    tokio::time::timeout(std::time::Duration::from_millis(50), source.read_byte())
        .await
        .ok()
        .and_then(|res| res.ok())
        .flatten()
}

fn csi_is_final(b: u8) -> bool {
    (0x40..=0x7e).contains(&b)
}

fn csi_continues(b: u8) -> bool {
    (0x20..=0x3f).contains(&b)
}

// ===== 以下自 lineedit.rs 原样迁入（termios 工具，零行为变更；行号锚见
// 项目状态键位盘点条）=====

/// 清终端输入缓冲（业主真机缺陷①「输入泄漏」的修法）：对话期间终端处于
/// cooked 态，用户打的字滞留 TTY 行缓冲，回答结束后进入编辑态前必须丢弃，
/// 否则成为「删不掉的前缀」——cooked 退格按字节删，UTF-8 三字节字要按三下
///（中文必现、英文不现即此因）。TCIFLUSH 只清「已收未读」的输入，不动输出；
/// fd 非终端时 tcflush 失败，调用方忽略（非 TTY 场景无本缺陷，失败不是错误）。
pub(crate) fn flush_stdin_fd(fd: i32) -> std::io::Result<usize> {
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
pub(crate) fn pretype_hint(n: usize) -> String {
    format!("已忽略 {n} 字节预输入")
}

/// termios 原始态守卫：enable 关 ICANON/ECHO（ISIG 保留），Drop 恢复（panic 安全）。
/// task-17④ 起供非交互攒行路径复用（pub(crate)）：语义同为「绕开内核行规程
/// 的行缓冲/回显，由应用自管」——复用即同一套 RAII 恢复纪律。
pub(crate) struct RawMode {
    pub(crate) active: bool,
    original: Option<libc::termios>,
}

impl RawMode {
    pub(crate) fn enable() -> std::io::Result<Self> {
        use std::os::fd::AsRawFd;
        let fd = std::io::stdin().as_raw_fd();
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } != 0 {
            // 非 TTY（管道）：无原始态可设，inactive 守卫（攒行零变化）
            return Ok(Self { active: false, original: None });
        }
        let original = unsafe { termios.assume_init() };
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO); // ISIG 保留：Ctrl-C 走信号路径
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { active: true, original: Some(original) })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let (true, Some(original)) = (self.active, &self.original) {
            use std::os::fd::AsRawFd;
            let fd = std::io::stdin().as_raw_fd();
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, original) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CSI 扩展解析（字节序列离线喂入——解析纯逻辑经通道注入直测）：
    /// 方向键四向/Home/End/Del(`3~`)/SS3 形态。
    #[tokio::test]
    async fn csi扩展解析_编辑族键全通() {
        let feed = |bytes: &[u8]| {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            let src = InputSource::from_receiver(rx);
            async move {
                for b in bytes {
                    tx.send(*b).await.unwrap();
                }
                drop(tx);
                read_csi(&src).await
            }
        };
        assert_eq!(feed(&[b'[', b'D']).await, Some(CsiKey::Left));
        assert_eq!(feed(&[b'[', b'C']).await, Some(CsiKey::Right));
        assert_eq!(feed(&[b'[', b'H']).await, Some(CsiKey::Home));
        assert_eq!(feed(&[b'[', b'F']).await, Some(CsiKey::End));
        assert_eq!(feed(&[b'[', b'3', b'~']).await, Some(CsiKey::Delete));
        assert_eq!(feed(&[b'O', b'A']).await, Some(CsiKey::Up));
        assert_eq!(feed(&[b'O', b'H']).await, Some(CsiKey::Home));
        assert_eq!(feed(&[b'[', b'Z']).await, Some(CsiKey::Other));
    }

    /// plain_key 映射：Ctrl 族全表+回车/退格；可见字符返回 None（UTF-8 拼读
    /// 归 IO 层）。
    #[test]
    fn plain_key_ctrl族映射() {
        assert_eq!(plain_key(0x01), Some(Key::CtrlA));
        assert_eq!(plain_key(0x05), Some(Key::CtrlE));
        assert_eq!(plain_key(0x0B), Some(Key::CtrlK));
        assert_eq!(plain_key(0x12), Some(Key::CtrlR));
        assert_eq!(plain_key(0x15), Some(Key::CtrlU));
        assert_eq!(plain_key(0x17), Some(Key::CtrlW));
        assert_eq!(plain_key(b'\r'), Some(Key::Enter));
        assert_eq!(plain_key(0x7F), Some(Key::Backspace));
        assert_eq!(plain_key(b'a'), None);
    }

    /// alt_key：Alt-B/F/Backspace；其余 Alt 组合 None（吞弃归 IO 层）。
    #[test]
    fn alt_key映射() {
        assert_eq!(alt_key(b'b'), Some(Key::AltB));
        assert_eq!(alt_key(b'f'), Some(Key::AltF));
        assert_eq!(alt_key(0x7F), Some(Key::AltBackspace));
        assert_eq!(alt_key(b'x'), None);
    }
}
