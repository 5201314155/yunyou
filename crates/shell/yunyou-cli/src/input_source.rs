//! 进程级 stdin 输入源（task-43）：专用长命阻塞读线程 + tokio mpsc 通道。
//!
//! 为什么不直接 `tokio::io::stdin()`：tokio 的 Stdin 读走 spawn_blocking，
//! `read(0)` 一旦阻塞便不可取消——`select!` 取消 read future 后 blocking 线程
//! 仍挂 `read(0)`，Runtime 关停无限等它（官方文档明示「can make shutdown hang
//! until the user presses a key」）。这是编辑态 Ctrl-C 后退出拖 14s+ 的真根
//! （task-42 验收 PASTE 场 strace：主线程 futex 14s + read EIO 唤醒链）。
//!
//! 方案 a（lead 预批）：std::thread 专用读线程常驻——Runtime drop 不 join
//! std 线程，进程退出（main 返回）时线程随进程终止，零孤儿零等待；字节经
//! tokio mpsc 单字节通道喂给异步层——`select!` 取消 recv future 天然安全
//! （通道缓冲保留已到字节不丢，下次进入编辑态继续消费）。

use std::io::{Read, Write};
use std::sync::OnceLock;
use tokio::sync::mpsc;

/// 通道容量：编辑态逐字节取，64 深度足够吸收打字节奏；粘贴突发时读线程
/// 背压在 send（TTY 行缓冲兜底），只延迟不丢字节。
const CHANNEL_CAP: usize = 64;

/// 全局单例：进程级唯一读线程（多次进入编辑态共享同一线程与通道，
/// std stdin 本身也是进程级唯一，双份读线程会互抢字节）。
static SOURCE: OnceLock<InputSource> = OnceLock::new();

/// 字符级删尾（task-17④ 攒行自管退格）：从字节缓冲尾部回退到 UTF-8 字符
/// 边界，删除一个完整字符并返回其显示列数（供回显擦列）——粘贴中文（3
/// 字节/字）退格一下删整字，不做 cooked 时代「按字节删」的退化行为。
fn pop_char(buf: &mut Vec<u8>) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let mut start = buf.len() - 1;
    while start > 0 && buf[start] & 0xC0 == 0x80 {
        start -= 1; // continuation 字节（0b10xxxxxx）向前并入同一字符
    }
    let width = std::str::from_utf8(&buf[start..])
        .map(crate::panel_text::display_width)
        .unwrap_or(1); // 非法序列按 1 列兜底（结尾统一 InvalidData 报错）
    buf.truncate(start);
    width
}

/// task-73 A1 回显旗：回合期 on（读线程回显可打印字节）/行编辑期 off（编辑器
/// 自绘，防双回显）。cbreak（ISIG 保留）下内核不回显，回显责任归应用侧。
static ECHO_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 回显旗设置（REPL 回合边界调用；pub(crate) 供 repl.rs 编排）。
pub(crate) fn set_echo(on: bool) {
    ECHO_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub struct InputSource {
    rx: tokio::sync::Mutex<mpsc::Receiver<u8>>,
    /// 回退队列（task-34）：回合期 Esc 监视臂读到的「非孤立 Esc 形态」字节
    /// 与回合期预打字节一律回退排队（FIFO），留给下一轮 lineedit 消费——
    /// 监视臂只吃「0x1B+50ms 孤立」形态，**禁吞字**。pending 恒前于通道。
    pending: std::sync::Mutex<std::collections::VecDeque<u8>>,
}

impl InputSource {
    /// 取全局输入源（首次调用懒起读线程；非 TTY/管道场景同样可用——
    /// EOF 时读线程发送终止语义后退出，见 [`Self::read_byte`]）。
    pub fn global() -> &'static Self {
        SOURCE.get_or_init(Self::spawn)
    }

    fn spawn() -> Self {
        let (tx, rx) = mpsc::channel(CHANNEL_CAP);
        // 专用长命线程：std::thread（非 spawn_blocking）——Runtime drop 不等
        // 它；接收端全 drop（进程收尾）或读源死亡（EOF/EIO）时自然退出。
        std::thread::Builder::new()
            .name("yunyou-stdin".into())
            .spawn(move || {
                let mut stdin = std::io::stdin();
                let mut byte = [0u8; 1];
                let mut out = std::io::stdout();
                loop {
                    match stdin.read(&mut byte) {
                        Ok(0) => return, // EOF：管道关闭/终端流结束
                        Ok(_) => {
                            // task-73 A1：回合期回显——cbreak 下内核不回显，可打印
                            // 字节由读线程即时回写（回显旗=回合期 on，行编辑期 off
                            // ——行编辑期由编辑器自绘，双回显防呆）。回显失败静默
                            // （tty 写失败=场景异常，不影响缓冲语义）。
                            if ECHO_ON.load(std::sync::atomic::Ordering::Relaxed)
                                && (byte[0].is_ascii_graphic() || byte[0] == b' ')
                            {
                                let _ = out.write_all(&byte);
                                let _ = out.flush();
                            }
                            if tx.blocking_send(byte[0]).is_err() {
                                return; // 接收端全 drop：进程收尾，随退
                            }
                        }
                        Err(_) => return, // EIO（pty master 关闭等）：读源已死
                    }
                }
            })
            .expect("stdin 读线程启动失败");
        Self {
            rx: tokio::sync::Mutex::new(rx),
            pending: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// 读一个字节（异步；`select!` 友好——取消 recv 即安全，已到字节留通道
    /// 不丢）。返回 None = 读线程已退出（EOF/EIO），流结束语义。
    pub async fn read_byte(&self) -> std::io::Result<Option<u8>> {
        // 回退队列恒前于通道（task-34）：监视臂回退的字节先于任何新输入。
        if let Some(b) = self.pending.lock().expect("回退队列锁").pop_front() {
            return Ok(Some(b));
        }
        let mut rx = self.rx.lock().await;
        Ok(rx.recv().await)
    }

    /// 字节回退排队（task-34）：回合期监视臂读到的非目标形态字节（0x1B+后
    /// 续、预打字符）原序塞回队列——下一轮 lineedit 逐字节消费，语义等同
    /// 「字节从未被读过」（禁吞字红线）。
    pub fn pushback(&self, bytes: &[u8]) {
        self.pending.lock().expect("回退队列锁").extend(bytes.iter().copied());
    }

    /// 直读通道（跳过回退队列，task-34 监视臂专用）：监视臂 pushback 后
    /// continue 若再走 read_byte 会取回自己刚回退的字节=无限循环——本方法
    /// 供监视臂消费通道新字节，回退队列字节只留给 lineedit。
    pub async fn read_channel_byte(&self) -> std::io::Result<Option<u8>> {
        let mut rx = self.rx.lock().await;
        Ok(rx.recv().await)
    }

    /// 非交互路径读一行（攒到 LF/CR 为止；EOF 返回 None）。
    /// 无效 UTF-8 保持既有语义：报 InvalidData（不静默替换）。
    ///
    /// task-17④（4095 cooked 残留）：stdin 为 TTY 时经 [`crate::lineedit::
    /// RawMode`] 关 ICANON/ECHO 绕开内核行规程的 4096 字节单行硬顶（cooked
    /// 模式下超长粘贴静默截断，实测 9000 字节落盘恰 4095）——攒行循环自管
    /// 回显与退格（ECHO 已关）：可打印字节原样回显、退格按字符级删尾并擦列。
    /// ISIG 保留：Ctrl-C 仍走 read_line_plain 的 ctrl_c 分支（future 被
    /// select 取消时 RawMode 随 drop 恢复终端，RAII 在 future 内）。非 TTY
    /// （管道）场景 tcgetattr 失败=inactive 守卫，攒行与回显逻辑零变化。
    pub async fn read_line(&self) -> std::io::Result<Option<String>> {
        let raw = crate::lineedit::RawMode::enable()?;
        // 自管回显仅在 TTY（RawMode active=ECHO 已关需应用补）时启用：
        // 管道/非 TTY 场景零回显（修正：无守卫版把管道输入也回显刷屏）
        let mut echo_out = if raw.active { Some(std::io::stdout()) } else { None };
        let mut line: Vec<u8> = Vec::new();
        loop {
            match self.read_byte().await? {
                Some(b'\n') | Some(b'\r') => {
                    if let Some(out) = echo_out.as_mut() {
                        let _ = out.flush();
                    }
                    return String::from_utf8(line)
                        .map(Some)
                        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err));
                }
                Some(0x7F) => {
                    // 自管退格（内核 ECHO 已关）：字符级删尾+按显示宽擦列；
                    // 空缓冲的退格静默（对齐 shell 空行退格习惯）。
                    let cols = pop_char(&mut line);
                    if cols > 0 {
                        if let Some(out) = echo_out.as_mut() {
                            for _ in 0..cols {
                                let _ = std::io::Write::write_all(out, b"\x08 \x08");
                            }
                            let _ = out.flush();
                        }
                    }
                }
                Some(b) => {
                    line.push(b);
                    // 自管回显：可打印字节原样写（UTF-8 多字节逐字节拼流，
                    // 终端按流解码）；控制字节不回显（\r\n 已在上方结行）。
                    if let Some(out) = echo_out.as_mut() {
                        if b >= 0x20 && b != 0x7F {
                            let _ = std::io::Write::write_all(out, &[b]);
                        }
                        let _ = out.flush();
                    }
                }
                None => {
                    return if line.is_empty() {
                        Ok(None)
                    } else {
                        String::from_utf8(line)
                            .map(Some)
                            .map_err(|err| {
                                std::io::Error::new(std::io::ErrorKind::InvalidData, err)
                            })
                    };
                }
            }
        }
    }

    /// 测试构造（注入预填通道；生产路径走 [`Self::global`]）。
    #[cfg(test)]
    pub(crate) fn from_receiver(rx: mpsc::Receiver<u8>) -> Self {
        Self {
            rx: tokio::sync::Mutex::new(rx),
            pending: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task-43 锁：通道攒行契约——LF/CR 结行、无尾行 EOF 提交、空流 EOF None、
    /// 无效 UTF-8 报 InvalidData（非交互路径既有 read_line 语义经通道保持）。
    #[tokio::test]
    async fn 通道攒行_lf_eof与utf8语义() {
        // 常规行：LF 结尾；随后读线程退出 = 流结束 None
        let (tx, rx) = mpsc::channel(16);
        for b in b"hi\n" {
            tx.send(*b).await.unwrap();
        }
        drop(tx);
        let src = InputSource::from_receiver(rx);
        assert_eq!(src.read_line().await.unwrap().as_deref(), Some("hi"));
        assert_eq!(src.read_line().await.unwrap(), None, "通道关闭=流结束");
        // 无尾行：EOF 时已攒内容提交
        let (tx, rx) = mpsc::channel(16);
        for b in b"tail" {
            tx.send(*b).await.unwrap();
        }
        drop(tx);
        let src = InputSource::from_receiver(rx);
        assert_eq!(src.read_line().await.unwrap().as_deref(), Some("tail"));
        // 空流：EOF → None（进程级流结束语义）
        let (tx, rx) = mpsc::channel(16);
        drop(tx);
        let src = InputSource::from_receiver(rx);
        assert_eq!(src.read_line().await.unwrap(), None);
        // 无效 UTF-8：InvalidData（不静默替换）
        let (tx, rx) = mpsc::channel(16);
        tx.send(0xFF).await.unwrap();
        drop(tx);
        let src = InputSource::from_receiver(rx);
        assert_eq!(
            src.read_line().await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}

    /// task-32 锁：>4KB 单行完整读回——旧 read_line_plain 的 per-call
    /// BufReader 8KB 预读在通道化（task-43）后根除，任意行长零截断。
    #[tokio::test]
    async fn 通道攒行_超4kb单行完整读回() {
        let (tx, rx) = mpsc::channel(20000);
        let long = "汉".repeat(4000); // 12000 字节 UTF-8，>4095 阈值
        for b in format!("{long}\n").into_bytes() {
            tx.send(b).await.unwrap();
        }
        drop(tx);
        let src = InputSource::from_receiver(rx);
        let line = src.read_line().await.unwrap().expect("行应完整");
        assert_eq!(line.len(), 12000, "12000 字节单行零截断（task-32 回归锁）");
        assert_eq!(line, long, "内容逐字节一致");
    }

#[cfg(test)]
mod pop_char_tests {
    use super::pop_char;

    /// task-17④：字符级删尾——ASCII 单字节、中文三字节、emoji 四字节按
    /// 字符整删并回报显示列数；非法序列兜底 1 列；空缓冲零操作。
    #[test]
    fn 字符级删尾与列数() {
        let mut buf = b"ab".to_vec();
        assert_eq!(pop_char(&mut buf), 1);
        assert_eq!(buf, b"a");
        let mut buf = "汉".as_bytes().to_vec();
        assert_eq!(pop_char(&mut buf), 2, "CJK 显示宽 2 列（回显擦两列）");
        assert!(buf.is_empty());
        let mut buf = "a汉".as_bytes().to_vec();
        assert_eq!(pop_char(&mut buf), 2);
        assert_eq!(buf, b"a", "删尾删整字不劈半");
        let mut buf = vec![b'a', 0xE4, 0xB8]; // 非法截断序列
        assert_eq!(pop_char(&mut buf), 1, "非法序列按 1 列兜底不 panic");
        assert_eq!(buf, b"a");
        let mut buf: Vec<u8> = Vec::new();
        assert_eq!(pop_char(&mut buf), 0, "空缓冲零操作");
    }
}
