//! lineedit 单元测试（task-42 外迁：主体贴 400 硬顶，tests mod 外挂 `#[path]`
//! 保持同 crate 私有可达（flush_stdin_fd/refresh/appended_suffix 等私有项直接
//! 测）；拆分不改逻辑、导出符号不动（家规四·拆分规矩）。

use super::*;

    /// 缺陷票①安全契约：对非终端 fd 调 flush——失败可辨（不伪装成功）且不破坏
    /// 缓冲数据（调用方 `let _ =` 忽略，不 panic）。真 TTY 的「回答期间打字→
    /// 进编辑态前清空」语义需 pty 交互面，一期不造：单元锁此安全契约，
    /// 端到端清场效果由真机手测兜底（业主现场可复测），取舍已在台账注明。
    #[test]
    fn flush_stdin_非终端fd安全失败不破坏数据() {
        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "测试前置：pipe 应建成");
        let (read_fd, write_fd) = (fds[0], fds[1]);
        // 预置数据进管道（模拟「对话期间用户打的字已进缓冲」）
        const LEAKED: &[u8; 16] = b"cooked-typeahead";
        assert_eq!(
            unsafe { libc::write(write_fd, LEAKED.as_ptr().cast(), LEAKED.len()) },
            LEAKED.len() as isize,
        );
        // 管道读端永非终端：tcflush 必须显式失败，不得伪装成功
        assert!(flush_stdin_fd(read_fd).is_err(), "非终端 fd tcflush 应失败而非伪装成功");
        // 失败路径数据无损（flush 对非终端无副作用）
        let mut buf = [0u8; LEAKED.len()];
        assert_eq!(
            unsafe { libc::read(read_fd, buf.as_mut_ptr().cast(), buf.len()) },
            LEAKED.len() as isize,
        );
        assert_eq!(&buf, LEAKED, "flush 失败路径不得动缓冲数据");
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }

    /// 票④根因锁：CSI 字节分类——根因事实是 '[' (0x5B) 按区间确属 final，
    /// 旧实现把 ESC 后首字节直接送 final 判定即在此翻车；修复=read_csi 结构上
    /// **先消费引导符再进参数循环**（顺序锁在 read_csi 的 '[' 分支，非区间定义）。
    #[test]
    fn csi字节分类根因锁() {
        assert!(csi_is_final(b'A'), "方向键 final 'A' 应判终止");
        assert!(csi_is_final(b'~'), "'~' 结尾（Home/End 等）应判终止");
        assert!(
            csi_is_final(b'['),
            "'[' (0x5B) 确属 final 区间——正因如此 read_csi 必须先消费引导符，误改区间定义反把合法 final 吞掉"
        );
        assert!(!csi_is_final(b'1'), "参数字节非 final");
        assert!(csi_continues(b'1'));
        assert!(csi_continues(b';'), "参数分隔符继续读");
        assert!(!csi_continues(b'A'), "final 字节不得再继续（终止即分发）");
    }

    /// task-42 锁：追加差分判定——纯追加（打字/粘贴）只写新增尾段（O(n)），
    /// 非前缀（退格/历史/面板替换）返回 None 走全行重绘。多字节安全护栏：
    /// 字节前缀之外再验 is_char_boundary（防御性，prev 恒为完整快照实际不触发）。
    #[test]
    fn 追加差分_纯追加出尾段_退格与替换走全行() {
        assert_eq!(appended_suffix("", "你"), Some("你"), "首字符=全量新增");
        assert_eq!(appended_suffix("你好", "你好世界"), Some("世界"), "CJK 多字节追加");
        assert_eq!(appended_suffix("abc", "abcdef"), Some("def"));
        assert_eq!(appended_suffix("你好", "你"), None, "退格=非前缀，走全行重绘");
        assert_eq!(appended_suffix("你好", "历史"), None, "历史替换=非前缀");
        assert_eq!(appended_suffix("同长", "同长"), None, "等长无新增（防御）");
    }

/// task-39 批 A：预输入提示行字节口径锁——「字节」用词（FIONREAD 计数为
/// 字节，按「字符」报数=伪造）+非零才出提示的语义（N=0 静默在调用方守卫）。
#[test]
fn 预输入提示字节口径() {
    let hint = pretype_hint(7);
    assert!(hint.contains("7 字节预输入"), "{hint}");
    assert!(!hint.contains("字符"), "不得按字符报数（UTF-8 多字节=伪造计数）: {hint}");
    assert!(pretype_hint(1).contains("1 字节预输入"));
}
