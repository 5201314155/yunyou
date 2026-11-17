//! bash_guard 的词法基础件：单引号区段表 / 词边界查找 / 参数 token 切分。
//! 按职责从门禁主体拆出（扫描器逻辑在 mod.rs，heredoc 解析在 heredoc.rs）。

/// 单引号区段表：`cmd` 中每个 `'...'` 配对区间的字节范围 [start, end)（含两端引号）。
/// bash 词法中单引号内默认为字面文本（不展开、不解释操作符），各扫描器据此整段
/// 跳过——以避免把教程/输出文本中的 `>`、tee 等字样误判为真写。**边界（P1-1）**：
/// 引号内容作为 `sh -c` 等启动器的脚本参数时被子 shell 二次解释，「字面」前提
/// 不成立——该形态由 mod.rs `scan_shell_c` 递归扫描覆盖，不经本表跳过。
/// 双引号不含在内（`"$(cmd > /abs)"` 命令替换里命令是活的，fail-closed 照扫）。
pub(super) fn single_quote_spans(cmd: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut open: Option<usize> = None;
    for (i, c) in cmd.char_indices() {
        if c == '\'' {
            match open {
                None => open = Some(i),
                Some(s) => {
                    spans.push((s, i + '\''.len_utf8()));
                    open = None;
                }
            }
        }
    }
    spans // 未闭合的单引号不记（bash 语法错误不执行，扫描结果无意义）
}

/// 字节位是否落在任一单引号区段内。
pub(super) fn in_single_quote(spans: &[(usize, usize)], pos: usize) -> bool {
    spans.iter().any(|&(s, e)| pos >= s && pos < e)
}

/// 从 `from` 起取下一个参数 token（跳空白；遇 `|;&` 视为命令段结束）；剥首尾
/// 引号后返回（`tee '/etc/x'` 的真目标），并给出 token 的字节起点（heredoc
/// 体排除据此判断 token 是否落在体内）。
pub(super) fn next_token(cmd: &str, from: usize) -> Option<(&str, usize, usize)> {
    let rest = &cmd[from..];
    let start = rest.len() - rest.trim_start().len();
    let rest = &rest[start..];
    if rest.is_empty() || rest.starts_with(['|', ';', '&']) {
        return None;
    }
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '|' | ';' | '&'))
        .unwrap_or(rest.len());
    let raw = &rest[..end];
    let token = raw.trim_matches(|c| c == '\'' || c == '"');
    let lead = raw.len() - raw.trim_start_matches(['\'', '"']).len();
    let token_start = from + start + lead;
    Some((token, token_start, from + start + end))
}

/// 重定向 target 的引号感知词采样（P2-A3）：从 `from`（操作符之后）起跳过
/// 空白收集**一个完整 bash 词**：
/// - `"`/`'` 起始＝引号包裹段整体取值（引号内空白与操作符不截断），
///   `> "path with space"` 取整词 `"path with space"`；闭合引号后继续吸收
///   拼接尾巴（`"a b"/c`＝一个词，bash 词法）；
/// - 双引号段内 `\"` 转义跳过；单引号段内无转义（bash 语义）；
/// - 引号未闭合＝bash 语法错误（命令不执行）或边界不可判定——返回 Err，
///   调用方 fail-closed（与 heredoc 未终止同取舍：解析不确定即保守拦截）。
///
/// `&` 不在词边界集：`2>&1` 的 target 采样为 `&1`（fd 复制由调用方按
/// 「`&`+数字」放行）；紧贴形态 `>file&` 多吸收 `&` 只会让越界判定更保守
/// （相对名两侧都放行，绝对名两侧都拦）。
///
/// 返回：`Ok(None)`＝无词（空白/词边界打头，如 `echo x >` 收尾与 `> ;`）；
/// `Ok(Some((word, end)))`＝词（含引号原文）与词尾字节位；`Err(引号位置)`
/// ＝引号未闭合。
pub(super) fn quoted_word(cmd: &str, from: usize) -> Result<Option<(String, usize)>, usize> {
    let bytes = cmd.as_bytes();
    let mut i = from.min(bytes.len());
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || is_word_break(bytes[i]) {
        return Ok(None);
    }
    let mut word = String::new();
    loop {
        match bytes.get(i) {
            None => return Ok(Some((word, i))),
            Some(b'\'') => {
                let content = i + 1;
                let Some(rel) = cmd[content..].find('\'') else {
                    return Err(i);
                };
                word.push_str(&cmd[i..content + rel + 1]);
                i = content + rel + 1;
            }
            Some(b'"') => {
                let mut k = i + 1;
                let closed = loop {
                    match bytes.get(k) {
                        None => break false,
                        Some(b'\\') => k += 2,
                        Some(b'"') => break true,
                        Some(_) => k += 1,
                    }
                };
                if !closed {
                    return Err(i);
                }
                word.push_str(&cmd[i..=k]);
                i = k + 1;
            }
            Some(&b) if is_word_break(b) => return Ok(Some((word, i))),
            Some(_) => {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && !is_word_break(bytes[i])
                    && !matches!(bytes[i], b'\'' | b'"')
                {
                    i += 1;
                }
                word.push_str(&cmd[start..i]);
            }
        }
    }
}

/// 词边界字符：ASCII 空白与 bash 操作符 `;|<>()`（`&` 不在内，见
/// [`quoted_word]` 注释）。
fn is_word_break(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b';' | b'|' | b'<' | b'>' | b'(' | b')')
}

/// 词边界查找 `word`（前后均非标识符/路径字符，避免命中路径段与子词）。
///
/// 引号语义（与 [`single_quote_spans`] 同源表驱动）：
/// - 单引号区段内的子词一律跳过——`echo 'run tee /etc/x'` 是纯字面输出不产生写；
/// - `'word'` 紧贴包裹形态视为 **quoting 命令名命中**（bash 引号拼接命令名合法：
///   `'tee' /etc/x` 是真写，fail-closed）；
/// - 词身拼接引号（`x'tee'`＝词 xtee）维持不命中（bounded 对 `'` 的既有排除）。
///
/// heredoc 体内的词不在本函数判断——调用方拿命中位置对照 heredoc 体区间集
/// 自行排除（词法层不依赖解析层）。
///
/// 已知盲区：引号拆词拼接（`te'e' /etc/x`＝词 tee）不识别，见 mod.rs 盲区清单。
pub(super) fn find_word(cmd: &str, from: usize, word: &str) -> Option<(usize, usize)> {
    let spans = single_quote_spans(cmd);
    let mut search = from;
    while let Some(rel) = cmd[search..].find(word) {
        let start = search + rel;
        let end = start + word.len();
        let before = cmd[..start].chars().next_back();
        let after = cmd[end..].chars().next();
        // `'word'` 紧贴包裹 = quoting 命令名（bash 合法形态：'tee' /etc/x 是真写），
        // fail-closed 命中；但左引号自身须处于词边界——`x'tee'` 的词是 xtee（引号
        // 属于词身拼接），左引号前是词字符时不得命中。
        if before == Some('\'') && after == Some('\'') {
            let before_quote = cmd[..start - 1].chars().next_back();
            let quote_bounded = !before_quote
                .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'));
            if quote_bounded {
                return Some((start, end));
            }
        }
        // 单引号区段内的子词 = 纯字面，零写不拦
        if in_single_quote(&spans, start) {
            search = start + 1;
            continue;
        }
        let bounded = |c: Option<char>| {
            !c.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '\''))
        };
        if bounded(before) && bounded(after) {
            return Some((start, end));
        }
        search = start + 1;
    }
    None
}
