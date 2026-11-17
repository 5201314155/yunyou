//! bash_guard 的 heredoc 认知（P0 缺陷二）：heredoc 体内一切都是字面文本，
//! 体内的 `>`/`>>` 不是重定向、tee/cd/cp 词不是命令——不做体排除就会把写
//! HTML（`</div>`）等正常动作误判为越界写（业主任务实证：Agent 连被误杀 4 次
//! 后被迫用十六进制转义绕行成功）。
//!
//! # 识别规则（bash 词法的止损子集，与 mod.rs 盲区清单配套）
//! - 起始：`<<`（非 `<<<` herestring），可选 `-`，标记词（裸词 / '...' / "..."，
//!   词遇操作符字符或空白即止）；标记为空＝非 heredoc 起始（照常扫）；
//! - 体：起始行的**下一行**起，到「整行等于标记」的行（`<<-` 时行首 tab 剥除）；
//!   起始行自身不进体——同行的其他重定向是真重定向（`cat > /etc/x <<EOF` 必拦）；
//! - 引号语义：单引号/双引号内的 `<<` 是字面（不开体）；heredoc 体整段跳过
//!   （体内引号不参与配对，`it's` 类不成对引号不污染后续解析）；
//! - 未终止（无终止标记行）：bash 会告警并继续执行，但门禁无法判定体内/体外
//!   边界——fail-closed 拒绝整条命令；
//! - 已知盲区（止损非根治）：`$( )`/反引号命令替换内的 heredoc 起始不识别
//!   （其体内 `>` 被外层照扫＝误杀方向 fail-closed，可恢复代价）。

/// heredoc 体字节区间集（covers(pos)＝pos 落在任一体内）。
pub(crate) struct HeredocMask {
    spans: Vec<(usize, usize)>,
}

impl HeredocMask {
    /// 解析命令文本：返回 (体区间集, 是否存在未终止 heredoc)。
    pub fn parse(cmd: &str) -> (Self, bool) {
        let bytes = cmd.as_bytes();
        let len = bytes.len();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        let mut in_single = false;
        let mut in_double = false;
        let mut i = 0;
        while i < len {
            let b = bytes[i];
            if in_single {
                if b == b'\'' {
                    in_single = false;
                }
                i += 1;
                continue;
            }
            if in_double {
                if b == b'"' {
                    in_double = false;
                }
                i += 1;
                continue;
            }
            match b {
                b'\'' => {
                    in_single = true;
                    i += 1;
                }
                b'"' => {
                    in_double = true;
                    i += 1;
                }
                b'<' if bytes[i..].starts_with(b"<<") => {
                    if bytes[i..].starts_with(b"<<<") {
                        // herestring：其 word 不是体（bash 词法把裸 `>` 拆为操作
                        // 符，word 内 `>` 本就按重定向照扫），跳过标记即可
                        i += 3;
                        continue;
                    }
                    let dash = bytes.get(i + 2) == Some(&b'-');
                    let (tag, tag_end) = read_tag(cmd, i + 2);
                    if tag.is_empty() {
                        // `<<` 后无词＝非 heredoc 起始（bash 语法错误场景照常扫）
                        i += 2;
                        continue;
                    }
                    // 体从起始行的下一行开始；起始行剩余内容仍是命令行
                    let Some(nl) = cmd[tag_end..].find('\n') else {
                        return (Self { spans }, true); // 起始行即 EOF：无终止行
                    };
                    let body_start = tag_end + nl + 1;
                    match find_terminator(cmd, body_start, &tag, dash) {
                        Some(end) => {
                            spans.push((body_start, end));
                            i = end; // 终止行之后继续扫（一条命令可含多段 heredoc）
                        }
                        None => return (Self { spans }, true), // 未终止 fail-closed
                    }
                }
                _ => i += 1,
            }
        }
        (Self { spans }, false)
    }

    /// 字节位是否落在任一 heredoc 体内。
    pub fn covers(&self, pos: usize) -> bool {
        self.spans.iter().any(|&(s, e)| pos >= s && pos < e)
    }
}

/// 读 heredoc 标记：`from` 指向 `<<` 后首字节（可能是 `-`），支持裸词与引号
/// 包裹（'...'/"..."，内容即标记）；空串＝非 heredoc 起始。
fn read_tag(cmd: &str, from: usize) -> (String, usize) {
    let bytes = cmd.as_bytes();
    let mut j = from;
    if bytes.get(j) == Some(&b'-') {
        j += 1; // `<<-` 形态：dash 已由调用方判定，这里只推进
    }
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    match bytes.get(j) {
        Some(&q @ (b'\'' | b'"')) => {
            let content_start = j + 1;
            let Some(rel) = cmd[content_start..].find(q as char) else {
                return (String::new(), from); // 引号未闭合＝bash 语法错误，不当 heredoc
            };
            (cmd[content_start..content_start + rel].to_string(), content_start + rel + 1)
        }
        Some(b'\\') => read_bare_tag(cmd, j + 1), // \<tag> 转义形态：跳过反斜杠取词
        _ => read_bare_tag(cmd, j),
    }
}

/// 裸词标记：读到空白或 bash 操作符字符为止；`\` 转义字节跳过（标记内剥除）。
fn read_bare_tag(cmd: &str, mut j: usize) -> (String, usize) {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let start = j;
    while j < len {
        let b = bytes[j];
        if b.is_ascii_whitespace() || matches!(b, b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>') {
            break;
        }
        j = if b == b'\\' { (j + 2).min(len) } else { j + 1 };
    }
    (cmd[start..j].replace('\\', ""), j)
}

/// 从 body_start 起逐行找终止标记行（整行等于标记；dash 时行首 tab 剥除），
/// 返回终止行的行首字节位（体区间即 [body_start, 终止行首)）。
fn find_terminator(cmd: &str, body_start: usize, tag: &str, dash: bool) -> Option<usize> {
    let mut line_start = body_start;
    for line in cmd[body_start..].split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let candidate = if dash { content.trim_start_matches('\t') } else { content };
        if candidate == tag {
            return Some(line_start);
        }
        line_start += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 体区间边界：起始行与终止行不在体内，体内容都在；终止行之后照常。
    #[test]
    fn 体区间边界() {
        let cmd = "cat <<EOF\nfirst > line\nsecond\nEOF\nrest";
        let (mask, unterminated) = HeredocMask::parse(cmd);
        assert!(!unterminated);
        assert!(mask.covers(cmd.find("first").unwrap()));
        assert!(mask.covers(cmd.find("second").unwrap()));
        assert!(!mask.covers(cmd.find("<<EOF").unwrap()), "起始行不进体");
        assert!(!mask.covers(cmd.find("rest").unwrap()), "终止行与之后不进体");
    }

    /// `<<-` 形态：终止行行首 tab 剥除后匹配；体内的 `>` 不进重定向扫描范围。
    #[test]
    fn dash形态与tab终止() {
        let cmd = "cat <<-TAG\n\tbody > /tmp/x\n\tTAG\nafter";
        let (mask, unterminated) = HeredocMask::parse(cmd);
        assert!(!unterminated);
        assert!(mask.covers(cmd.find("body").unwrap()));
        assert!(!mask.covers(cmd.find("after").unwrap()));
    }

    /// 引号包裹标记、转义标记均识别；herestring 与引号内 `<<` 不开体。
    #[test]
    fn 标记形态与排除形态() {
        let (mask, u) = HeredocMask::parse("cat <<'EOF'\nbody > /tmp\nEOF\nx");
        assert!(!u);
        assert!(mask.covers("cat <<'EOF'\nbody".len() - 2));
        let (mask, u) = HeredocMask::parse("cat <<\\EOF\nbody\nEOF");
        assert!(!u);
        assert!(mask.covers("cat <<\\EOF\nbody".len() - 2));
        // herestring：不开体
        let cmd = "cat <<< text\nmore > /tmp\n";
        let (mask, u) = HeredocMask::parse(cmd);
        assert!(!u);
        assert!(!mask.covers(cmd.find("more").unwrap()));
        // 双引号内的 << 是字面：不开体
        let cmd = "echo \"<<EOF\"\nnext > /tmp\n";
        let (mask, u) = HeredocMask::parse(cmd);
        assert!(!u);
        assert!(!mask.covers(cmd.find("next").unwrap()));
    }

    /// 体内不成对引号不污染后续解析；一条命令可含多段 heredoc。
    #[test]
    fn 不成对引号与多段() {
        let cmd = "cat <<A\nit's fine > /tmp\nA\ncat <<B\nsecond > /tmp\nB\nend";
        let (mask, u) = HeredocMask::parse(cmd);
        assert!(!u, "体内不成对引号不得导致误判未终止");
        assert!(mask.covers(cmd.find("it's").unwrap()));
        assert!(mask.covers(cmd.find("second").unwrap()));
        assert!(!mask.covers(cmd.find("end").unwrap()));
    }

    /// 未终止 heredoc：fail-closed 标志置位。
    #[test]
    fn 未终止fail_closed() {
        let (_, u) = HeredocMask::parse("cat <<EOF\nbody > /tmp/x");
        assert!(u);
        let (_, u) = HeredocMask::parse("cat <<EOF");
        assert!(u);
        let (_, u) = HeredocMask::parse("cat <<EOF\nbody\nEOFX"); // 标记不完全匹配≠终止
        assert!(u);
    }
}
