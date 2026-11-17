//! bash 命令文本越界写门禁（D-17-1 止损层；**非根治**）。
//!
//! # 定位
//! path_guard 只守文件类工具（read/write/edit/glob/grep）的路径解析，bash 起独立
//! 子进程，其 shell 重定向天然绕过该层（云枢任务17 运行态实证）。本模块在**执行
//! 前**静态扫描命令文本，拒绝四类直白越界写：
//! ① 重定向到绝对路径（`>`/`>>`/`1>`/`2>`/`&>`；`2>&1` fd 复制放行）；
//! ② `tee`/`tee -a` 的绝对路径目标；
//! ③ `cd` 出根（绝对路径/`..`/`~`，含「cd 到绝对路径后的相对写」）；
//! ④ `cp`/`mv`/`install` 到绝对路径（只查写目标，源即使绝对也只是读）。
//!
//! # 止损非根治（明写，防误读为安全边界）
//! bash 图灵完备，本门禁是**启发式止损**：挡不住 `eval`/变量间接/`$( )` 变体/
//! `dd of=`/`rsync`/编码绕过等，漏杀即安全事件；取向 fail-closed——`echo 'cd
//! /etc'` 类纯字面误杀是已知代价（误杀可恢复，漏杀不可逆）。根治＝契约⑨
//! sandboxPolicy 策略对象（P1-11a，见《超时重试插件设计》§6 排期终裁：一期
//! 受约束批落地「bash 子进程经沙箱执行」）。
//!
//! # 引号语义（bash 解析规则决定的不对称处理）
//! - 单引号内默认**跳过重定向扫描**：bash 词法中单引号内不展开、不解释操作符，
//!   `echo 'text > /tmp/x'` 是纯字面输出不产生写——跳过是为不把教程/输出文本
//!   中的操作符误判为真重定向；tee/cd/cp/mv/install 的**词与参数**同理经单引号
//!   区段表跳过。但 `'word'` 紧贴包裹视为 quoting 命令名命中（`'tee' /etc/x`
//!   是 bash 合法的真写，fail-closed），参数在引号内仍是真目标（`tee '/etc/x'`
//!   真写）照查；
//! - **单引号跳过有一个已证伪的边界（P1-1 修复）**：引号内容作为 `sh -c`/
//!   `bash -c` 等启动器的脚本参数时会被**子 shell 二次解释**，`sh -c 'cat x >
//!   /tmp/y'` 内的 `>` 是真重定向——「单引号内都是字面」在该形态不成立。该形态
//!   由 [`scan_shell_c`] 识别（`-c` 选项词+shell 家族名单+命令位置三层判定），
//!   对脚本内容递归跑完整门禁管线（含 heredoc 状态机）。已覆盖形态：双引号
//!   外壳包内层脚本（`bash -c "sh -c '…'"`——shell 名剥引号壳判定，复核打回
//!   项 1）与复合命令组（`{ bash -c '…'; }`——`{` 与 `(` 同入命令位置分隔符，
//!   复核打回项 2）。管道喂 shell 形态（A-1 修复：`echo '…' | sh` 从 stdin 读
//!   脚本）由 [`scan_pipe_shell`] fail-closed 拒绝（脚本内容静态不可判）。
//!   残余盲区（止损层如实声明，逐条可复核）：`su <user> -c
//!   '…'`（用户名夹层）、`find . -exec sh -c '…' ;`（-exec 不在透明启动器
//!   名单）、带参启动器（`xargs -I{} sh -c`）、非名单 shell、`\''` 拼接转义的
//!   脚本内容——根治＝契约⑨ sandboxPolicy（P1-11a）；
//! - 双引号内**照扫**：`"$(cmd > /abs)"` 的命令替换里重定向是活的，fail-closed
//!   （纯字面 echo 文本含 `>` 会被一并拦＝已知误杀代价，见测试对照）；
//! - 重定向 target 采样**引号感知整词**（P2-A3，[`lex::quoted_word`]）：引号
//!   内空格不截断（`> "path with space"` 整体判越界），引号未闭合 fail-closed
//!   （解析不确定即保守拦截；误杀可恢复、漏杀不可逆，与 heredoc 未终止同取舍）。
//!
//! # heredoc 语义（P0 缺陷二：词法拆分 heredoc.rs / lex.rs 两个子模块）
//! heredoc 体内一切都是字面文本：体内 `>`/`>>` 不是重定向、tee/cd/cp 词不是
//! 命令、体内 token 不是参数——不做体排除会把写 HTML（`</div>`）等正常动作
//! 误判为越界写。四类扫描器统一按 heredoc 体区间集（[`heredoc::HeredocMask`]）
//! 排除；起始行不进体（`cat > /etc/x <<EOF` 的真重定向照拦）；未终止 heredoc
//! 无法判定体边界＝fail-closed 拒绝整条命令。词法基础件在 [`lex`]。
//!
//! # 已知盲区（止损非根治的边界，评审 P2-3 补记）
//! 引号拆词拼接不识别：`te'e' /etc/x` 在 bash 里词=tee（真命令），本门禁
//! find_word 找不到连续字面 `tee` 而漏杀；同族 `te"e"e`/反斜杠续行等同理。
//! tee/cp 族参数走 next_token 未引号感知：`tee "a b/../../x"` 词法取值在
//! 引号内截断，越界分量逃过越界检查而漏杀（云镜 A3 复核 nit②；重定向族已由
//! quoted_word 修复，此族修法候选=同构采样，待云枢线裁决后随 P1-11a 收敛，
//! 先入清单留痕防遗忘）。
//! 裸 `cd`（无参数＝cd $HOME）不覆盖：home 在根外属越界，但静态扫描无参数
//! 可查（审三 nit，根治在契约⑨ sandboxPolicy P1-11a）。
//! 与 eval/变量间接/`$( )` 变体/`dd of=`/`rsync`/编码绕过同属启发式止损盲区，
//! 根治＝契约⑨ sandboxPolicy（P1-11a）。

/// 门禁拒绝的统一话术（每条拒绝消息都以它开头）：明写止损定位与根治指向，
/// 让模型/用户一眼看出「这是止损层，别当成沙箱」。pub 供集成测试单源引用
///（评审 nit：tests/bash_guard.rs 原 GB 字面量双份维护已消除）。
pub const GUARD_REJECT_NOTE: &str =
    "越界写被 bash 门禁拒绝（止损层；根治靠契约⑨ sandboxPolicy P1-11a）";

/// 把扫描出的原因组装成统一拒绝话术（cmd 扫描与 cwd 校验共用）。
pub(crate) fn reject_message(reason: &str) -> String {
    format!("{GUARD_REJECT_NOTE}：{reason}")
}

mod heredoc;
mod lex;

use heredoc::HeredocMask;
use lex::{find_word, in_single_quote, next_token, quoted_word, single_quote_spans};

/// 命令文本越界写静态门禁（D-17-1 止损版；**非根治**，根治＝契约⑨
/// sandboxPolicy P1-11a）。
///
/// 返回 `Some(原因)` ＝ 拒绝；`None` ＝ 通过（不代表安全，仅未命中已知模式）。
pub fn detect_escaping_write(cmd: &str) -> Option<String> {
    // heredoc 体区间先解析（P0 缺陷二）：体内一切字面——四类扫描器按 mask
    // 排除；未终止＝体边界不可判定，fail-closed 拒绝整条命令
    let (mask, unterminated) = HeredocMask::parse(cmd);
    if unterminated {
        return Some("存在未终止的 heredoc，无法静态判定命令边界（fail-closed）".into());
    }
    // sh -c 脚本参数扫描（P1-1 修复）先行：单引号跳过机制对「shell -c 脚本」
    // 形态不适用，脚本内容递归跑完整管线（含 heredoc 状态机独立重建）
    scan_shell_c(cmd)
        .or_else(|| scan_pipe_shell(cmd, &mask))
        .or_else(|| scan_redirects(cmd, &mask))
        .or_else(|| scan_tee(cmd, &mask))
        .or_else(|| scan_cd(cmd, &mask))
        .or_else(|| scan_copy_like(cmd, &mask))
}

/// 扫描 `sh -c '…'` 类单引号脚本参数（P1-1 修复）：引号内容被子 shell 二次
/// 解释，内层 `>`/tee/cd/cp 是真操作——对脚本内容递归跑完整门禁管线（heredoc
/// 状态机对脚本内容独立重建，脚本内 heredoc 语义正确）。判定三层（见
/// [`is_shell_c_script_arg`]），命中即剥引号递归；未命中回落单引号字面跳过
/// （`echo 'text > /tmp/x'` 类纯输出不误杀）。
fn scan_shell_c(cmd: &str) -> Option<String> {
    for (start, end) in single_quote_spans(cmd) {
        if !is_shell_c_script_arg(cmd, start) {
            continue;
        }
        // 剥引号取脚本内容，递归扫描（脚本有自己的词法语境；每层剥一层引号
        // 长度单调递减，无递归失控面）。已知盲区：`'\''` 拼接转义的脚本内容
        // 被外层区段表拆断，仅扫到首段（头注盲区清单声明）。
        let script = &cmd[start + '\''.len_utf8()..end - '\''.len_utf8()];
        if let Some(reason) = detect_escaping_write(script) {
            return Some(format!("shell -c 脚本内：{reason}"));
        }
    }
    None
}

/// 扫描「管道喂 shell」形态（A-1）：管道末端是裸 shell（无 `-c` 脚本参数）
/// 时，shell 从 stdin 读脚本——脚本内容来自前段命令输出（echo 字面/curl 远程
/// 内容/文件内容皆可行），静态扫描无法判定其是否含越界写，fail-closed 拒绝
/// 整条命令。`| sh -c '…'` 不在本扫描器范围（脚本文本静态可见，[`scan_shell_c`]
/// 先行递归处理）；引号包裹命令名（`| 'sh'`）与 heredoc 体内字面（`<<EOF … sh`
/// 是文档不是命令）按既有词法排除。
fn scan_pipe_shell(cmd: &str, mask: &HeredocMask) -> Option<String> {
    for shell in ["sh", "bash", "dash", "zsh", "ksh", "ash", "busybox"] {
        let mut from = 0usize;
        while let Some((start, end)) = find_word(cmd, from, shell) {
            from = end;
            if mask.covers(start) {
                continue; // heredoc 体内是字面文档
            }
            // 前一非空白字符是管道符＝该 shell 位于管道末端读 stdin（`|&` 同判）
            let prev = cmd[..start].trim_end().chars().next_back();
            if !matches!(prev, Some('|')) {
                continue;
            }
            // 后随 `-c` 选项＝脚本参数静态可见，交 scan_shell_c，不在此拦
            if let Some((token, _, _)) = next_token(cmd, end) {
                if is_dash_c_option(token) {
                    continue;
                }
            }
            return Some(format!(
                "管道输出直接喂 {shell}（从 stdin 读脚本，脚本内容静态不可判，fail-closed）"
            ));
        }
    }
    None
}

/// 判定左引号位于 `quote_start` 的单引号区段是否为 shell 启动器 `-c` 的脚本
/// 参数。三层判定（任一不满足＝按字面处理）：
/// ①引号前紧邻词匹配合并短选项 `-[a-z]*c`（`-c`/`-lc`/`-rc`…，引号紧贴
/// `-c'…'` 时引号前恰为完整选项词，自然覆盖）；
/// ②该词之前的词剥路径 basename 后属 shell 家族名单（su 直达形态一并认）；
/// ③shell 词处于**命令位置**（行首/命令分隔符之后/透明启动器或赋值前缀之后）
/// ——防 `echo sh -c '…'` 类「字面脚本文本」被误判（bash 里那是 echo 输出）。
fn is_shell_c_script_arg(cmd: &str, quote_start: usize) -> bool {
    let before = &cmd[..quote_start];
    let trimmed = before.trim_end();
    let Some((opt_word, opt_start)) = last_word(trimmed) else {
        return false;
    };
    if !is_dash_c_option(opt_word) {
        return false;
    }
    let head = trimmed[..opt_start].trim_end();
    let Some((shell_word, shell_start)) = last_word(head) else {
        return false;
    };
    if !is_shell_family(shell_word) {
        return false;
    }
    is_command_position(&head[..shell_start])
}

/// 取 `text` 末尾的词（跳尾部空白；词=空白与 `;|&(` 之前的连续段）。
/// 返回 (词, 词起点字节位)。
fn last_word(text: &str) -> Option<(&str, usize)> {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    let start = trimmed
        .rfind(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '('))
        .map(|p| p + 1)
        .unwrap_or(0);
    Some((&trimmed[start..], start))
}

/// `-[字母]*c` 合并短选项形态：`-c` 主形态＋`-lc`/`-rc`/`-ic`（bash login/
/// restricted/interactive 与 -c 合并写法）。`-x`/`-c2`/`-ab` 均不匹配。
fn is_dash_c_option(word: &str) -> bool {
    let Some(body) = word.strip_prefix('-') else {
        return false;
    };
    let Some(stem) = body.strip_suffix('c') else {
        return false;
    };
    stem.chars().all(|c| c.is_ascii_lowercase())
}

/// shell 家族名单（按 basename 判定，容忍 `/bin/sh` 绝对路径形态）。
/// 引号壳剥除后判定（云垒复核打回项 1）：`bash -c "sh -c '…'"` 的内层单引号
/// span 判定中 shell 词被外层双引号粘连成 `"sh`——引号是 quoting 壳不是词身，
/// 剥壳后按真命令处理（与 `'tee' /etc/x` quoting 命令名 fail-closed 先例同向，
/// 多拦不漏）。`su -c '…'`（用户名省略）直达形态一并认；名单是启发式止损的
/// 边界，非名单 shell 与 `su <user> -c` 夹层形态属已知盲区（头注声明）。
fn is_shell_family(word: &str) -> bool {
    let stripped = word.trim_matches(|c| c == '\'' || c == '"');
    let base = stripped.rsplit('/').next().unwrap_or(stripped);
    matches!(
        base,
        "sh" | "bash" | "dash" | "zsh" | "ksh" | "ash" | "busybox" | "fish" | "csh" | "tcsh" | "su"
    )
}

/// 命令位置判定：`text`（shell 词之前的前缀）为空、以命令分隔符或子 shell
/// 开括号结尾、或尾部是透明前缀链（启动器/纯数字参数/环境赋值——循环向前
/// 剥，覆盖 `nohup sh -c`、`sudo sh -c`、`timeout 5 sh -c`、`FOO=1 bash -c`）。
/// 尾部为普通词（如 `echo sh` 的 echo）＝非命令位置。
fn is_command_position(text: &str) -> bool {
    let mut rest = text.trim_end();
    loop {
        if rest.is_empty() {
            return true; // 行首
        }
        let Some(last) = rest.chars().last() else {
            return true; // trim_end 后非空则必有首字符（防御分支，理论不可达）
        };
        if matches!(last, ';' | '|' | '&' | '(' | '{') {
            return true; // 命令分隔符 / 子 shell 开括号 / 复合命令组开括号之后
        }
        let Some((word, wstart)) = last_word(rest) else {
            return true;
        };
        if is_launcher(word)
            || is_pure_number(word)
            || is_env_assignment(word)
            // 选项词与 shell 名同为透明（复核打回项 1 深层）：双引号外壳形态
            // `bash -c "sh -c '…'"` 的内层 shell 词（"sh）之前是外层启动器的
            // `-c` 选项与 shell 名本身——它们属于外层命令的组成部分，向前剥到
            // 外层命令的命令位置为止。防误杀自证：`echo bash -c '…'` 的 echo
            // 是普通词（非透明、非分隔符）在此卡住＝不判脚本，echo 纯输出放行。
            || is_dash_c_option(word)
            || is_shell_family(word)
        {
            rest = rest[..wstart].trim_end(); // 透明前缀，继续向前剥
            continue;
        }
        return false;
    }
}

/// 透明启动器名单：其后可跟任意命令（is_command_position 循环可剥多层）。
fn is_launcher(word: &str) -> bool {
    matches!(
        word,
        "nohup" | "env" | "exec" | "nice" | "timeout" | "xargs" | "setsid" | "time" | "sudo"
            | "doas" | "stdbuf" | "ionice" | "taskset"
    )
}

/// 纯数字词透明（`timeout 5 sh -c` 的 `5`；`xargs -I{}` 类带参启动器留盲区）。
fn is_pure_number(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_digit())
}

/// 环境变量赋值前缀（`FOO=bar` 形态，透明）。
fn is_env_assignment(word: &str) -> bool {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    let mut saw_eq = false;
    for c in chars {
        if c == '=' {
            saw_eq = true;
            break;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    saw_eq
}

/// 门禁拒绝 → (统一话术, 指路)。按原因类别给可执行指引（P0 三缺陷之三）：
/// 重定向类指路 write 工具（`>` 特殊字符场景）；tee/cp 族同族意图同去处；
/// cd/cwd 类指路项目根内相对路径；未终止 heredoc 指路补标记或 write 工具。
/// 指引全中文、不含内部路径；四类原因均有指引（返回值恒带 hint）。
/// 分类用 contains 而非 starts_with：sh -c 脚本递归命中的 reason 带
/// 「shell -c 脚本内：」外层前缀（P1-1 修复），内层类别词仍须命中分派。
pub(crate) fn reject_with_hint(reason: &str) -> (String, String) {
    use crate::builtin::{HINT_PATH_OUTSIDE_ROOT, HINT_REDIRECT_BLOCKED, HINT_WRITE_BLOCKED};
    let hint = if reason.contains("重定向目标") {
        HINT_REDIRECT_BLOCKED
    } else if reason.contains("cd 目标") || reason.contains("cwd ") {
        HINT_PATH_OUTSIDE_ROOT
    } else if reason.contains("存在未终止的 heredoc") {
        "heredoc 未终止；请补齐终止标记行，或改用 write 工具写多行文本"
    } else {
        // tee/cp/mv/install 目标＝写文件意图被拦
        HINT_WRITE_BLOCKED
    };
    (reject_message(reason), hint.to_string())
}

/// 越界路径判定（绝对路径/`~`/`..` 出界；相对子目录合法）。例外：`/dev` 流设备
/// （null/stdout/stderr/tty/zero）作重定向目标＝「丢弃/透传输出」语义，非文件
/// 越界（`2>/dev/null` 高频惯用法），放行。
///
/// `..` 判定（评审 nit 收紧 + 审三 P1 回炉）：**任一 `..` 分量即拒**——
/// `./../evil`、`sub/../../x` 的 `..` 藏在分量中间，前缀匹配拦不住（实证漏杀）。
/// fail-closed：合法根内写不需要 `..`，`sub/../f` 回退再进写法一并拒（误杀可
/// 恢复，漏杀不可逆）；不模拟净路径（cd `sub/..` 净效果在根内也被拒＝已知代价，
/// 与「`echo 'cd /etc'` 纯字面误杀」同一取舍，根治＝契约⑨ sandboxPolicy）。
/// `..foo`/`...` 是 bash 合法文件名（分量 ≠ `..`），不误杀。
pub(crate) fn is_outside_root_path(path: &str) -> bool {
    let stripped = path.trim_matches(|c| c == '\'' || c == '"');
    if matches!(
        stripped,
        "/dev/null" | "/dev/stdout" | "/dev/stderr" | "/dev/tty" | "/dev/zero"
    ) {
        return false;
    }
    stripped.starts_with('/')
        || stripped.starts_with('~')
        || stripped == ".."
        || stripped.starts_with("../")
        || has_dotdot_component(stripped)
}

/// 路径是否含字面 `..` 分量（任一分量等于 `..` 即真）：`./../evil`、
/// `sub/../../x` 的越界靠前缀匹配拦不住（审三 P1 回炉实证），按分量扫。
/// `..foo`/`...` 分量不等于 `..`，不误杀。
fn has_dotdot_component(path: &str) -> bool {
    path.split('/').any(|c| c == "..")
}

/// 扫描重定向目标（`>` 族）：单引号内跳过、双引号内照扫、heredoc 体内一律
/// 不算重定向（引号与 heredoc 语义见模块文档）。target 采样引号感知整词
/// （P2-A3，[`lex::quoted_word`]）。
fn scan_redirects(cmd: &str, mask: &HeredocMask) -> Option<String> {
    // 单引号区段表驱动（评审 P2 回炉）：与 find_word 共用 [`lex::single_quote_spans`]。
    // 原 toggle 式跟踪会被双引号内字面 `'` 翻转（`echo "it's fine" > /etc/x`
    // 翻转后 `>` 全盲＝漏杀）；表驱动只认 bash 真实配对区段，且未闭合 `'` 不记
    // 区段（bash 语法错误不执行，扫描结果无意义），语义分歧一并消除。
    let spans = single_quote_spans(cmd);
    let chars: Vec<(usize, char)> = cmd.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c != '>' || in_single_quote(&spans, pos) || mask.covers(pos) {
            i += 1;
            continue;
        }
        // 连续 `>`（`>>` 追加）；`>|` 强制覆盖 / `<>` 读写重定向的操作符也要跳过，
        // 否则 target 被截成空串＝漏杀；`2>&1`/`1>&2` fd 复制放行
        let mut j = i + 1;
        while j < chars.len() && chars[j].1 == '>' {
            j += 1;
        }
        if chars.get(j).is_some_and(|(_, c)| matches!(c, '|' | '<')) {
            j += 1;
        }
        // target 采样：引号感知整词（P2-A3）——旧 `take_while` 遇空白即断，
        // `> "path with space"` 被截成 `"path`：前段不越界而完整路径越界
        // （`> "a b/../../x"` 形态）＝漏杀。引号未闭合 fail-closed，取舍：
        // 误杀可恢复（拒绝话术带指路 hint），漏杀是不可逆越界写；解析不确定
        // 即保守拦截，与 heredoc 未终止同取舍。
        let Some(&(word_start, _)) = chars.get(j) else {
            return None; // `>` 收尾无 target：bash 语法错误不执行，放行
        };
        match quoted_word(cmd, word_start) {
            Err(qpos) => {
                return Some(format!(
                    "重定向目标引号未闭合（位置 {qpos}），无法静态判定（fail-closed）"
                ));
            }
            Ok(None) => {
                i = j; // 无词（分隔符打头）：bash 语法错误不执行，放行
            }
            Ok(Some((target, word_end))) => {
                let body = target.strip_prefix('&').unwrap_or(&target);
                if body.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    i = j; // fd 复制（2>&1）放行
                    continue;
                }
                if is_outside_root_path(&target) {
                    return Some(format!("重定向目标 \"{target}\" 越出根目录（位置 {pos}）"));
                }
                i = chars.partition_point(|&(p, _)| p < word_end);
            }
        }
    }
    None
}

/// 扫描 `tee` 的输出目标（选项后逐参数检查，遇管道/分号/`&&`/heredoc 体停止）。
fn scan_tee(cmd: &str, mask: &HeredocMask) -> Option<String> {
    let mut from = 0;
    while let Some((start, end)) = find_word(cmd, from, "tee") {
        if mask.covers(start) {
            from = end; // 词在 heredoc 体内＝字面文本，非命令
            continue;
        }
        let mut cursor = end;
        while let Some((token, token_start, token_end)) = next_token(cmd, cursor) {
            if mask.covers(token_start) {
                break; // token 起点进体＝字面文本，非参数
            }
            if token.starts_with('-') {
                cursor = token_end; // -a / --append 等选项
                continue;
            }
            if is_outside_root_path(token) {
                return Some(format!("tee 目标 \"{token}\" 越出根目录"));
            }
            cursor = token_end;
        }
        from = end;
    }
    None
}

/// 扫描 `cd` 出根（目标为绝对路径、`..` 或 `~`；相对子目录合法）。命中即拒，
/// 一并挡掉「cd 到绝对路径后的相对写」——cd 之后 cwd 已出根，后续相对写全越界。
/// `cd -- DIR` 的 end-of-options 分隔符跳过后取真目标（评审 P2-1：`cd -- /etc`
/// 是直白越界写，不跳过即漏杀）。
fn scan_cd(cmd: &str, mask: &HeredocMask) -> Option<String> {
    let mut from = 0;
    while let Some((start, end)) = find_word(cmd, from, "cd") {
        if mask.covers(start) {
            from = end; // 词在 heredoc 体内＝字面文本
            continue;
        }
        let mut cursor = end;
        let mut target: Option<&str> = None;
        while let Some((token, token_start, token_end)) = next_token(cmd, cursor) {
            if mask.covers(token_start) {
                break; // token 起点进体＝字面文本，非参数
            }
            if token == "--" {
                cursor = token_end; // end-of-options：真目标在其后
                continue;
            }
            target = Some(token);
            break;
        }
        if let Some(token) = target {
            if is_outside_root_path(token) {
                return Some(format!("cd 目标 \"{token}\" 越出根目录"));
            }
        }
        from = end;
    }
    None
}

/// 扫描 `cp`/`mv`/`install` 的越界目标（**启发式止损**，只覆盖主模式）。
///
/// 只查「写目标」＝最后一个非选项参数（`cp src... dst` / `install src dstdir`）；
/// 源即使绝对路径也只是读（门禁只管写，与 [`scan_redirects`] 同口径），`-t DIR`/
/// `--target=DIR` 的显式目标一并查。绕法（`dd of=`/`rsync`/`ln -s`/变量间接）
/// ＝已知盲区，止损非根治。
fn scan_copy_like(cmd: &str, mask: &HeredocMask) -> Option<String> {
    for tool in ["cp", "mv", "install"] {
        let mut from = 0;
        while let Some((start, end)) = find_word(cmd, from, tool) {
            if mask.covers(start) {
                from = end; // 词在 heredoc 体内＝字面文本，非命令
                continue;
            }
            if let Some(dest) = find_dest(cmd, end, mask) {
                if is_outside_root_path(dest) {
                    return Some(format!("{tool} 目标 \"{dest}\" 越出根目录"));
                }
            }
            from = end;
        }
    }
    None
}

/// 取命令段的写目标：最后一个非选项 token；`-t DIR`/`--target-directory=DIR`
/// 是显式目标，直接返回（它们后面的 token 才是目标，不当普通参数）。
/// `--target-directory=` 是 GNU coreutils cp/install 的真实长选项（评审 P2-2：
/// 原实现只认不存在的 `--target=`，真形态漏杀）；`--target=` 保留容错。
fn find_dest<'a>(cmd: &'a str, from: usize, mask: &HeredocMask) -> Option<&'a str> {
    let mut cursor = from;
    let mut last: Option<&str> = None;
    while let Some((token, token_start, token_end)) = next_token(cmd, cursor) {
        if mask.covers(token_start) {
            break; // token 起点进体＝字面文本，非参数
        }
        if let Some(target) = token
            .strip_prefix("--target-directory=")
            .or_else(|| token.strip_prefix("--target="))
        {
            return Some(target);
        }
        if token == "-t" || token == "--target" {
            return next_token(cmd, token_end).map(|(t, _, _)| t);
        }
        if !token.starts_with('-') {
            last = Some(token);
        }
        cursor = token_end;
    }
    last
}