//! persona.md 文本切段：`#` 顶层标题把人设 markdown 切成多个提示段（池 task-6）。
//!
//! 设计口径：人设文件改完即生效、零代码配置——文件内容经既有同名 shadow 人设槽
//! （[`crate::section::SECTION_PERSONA_PREFIX`]）进装配；`#` 标题切段让人设文件
//! 里按主题组织的多个小节各自成段（段序同人设槽档位，按段名后缀升序保持文件顺序）。
//!
//! 切段规则（有意从简，测试锁定）：
//! - 只认**行首单 `#` 加空格**（顶层标题）；`##`/`###` 等子级标题不切段（它们是
//!   小节内部结构，属人设文本本体）；
//! - 首个标题前的散文本自成一段（无标题的纯文本文件＝整文件一段，向后兼容
//!   toml 字符串人设形态）；
//! - 空段（纯空白）丢弃；输出段序与文件出现顺序一致。

/// 把人设 markdown 文本按 `#` 顶层标题切段，返回按文件顺序排列的非空段文本。
pub fn split_md_sections(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let flush = |current: &mut Vec<&str>, chunks: &mut Vec<String>| {
        let chunk = current.join("\n").trim().to_string();
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        current.clear();
    };
    for line in text.lines() {
        if line.starts_with("# ") {
            flush(&mut current, &mut chunks);
        }
        current.push(line);
    }
    flush(&mut current, &mut chunks);
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 段落切分：# 标题切段、段序保文件顺序、## 子级不切、前置散文本成段、空段丢弃。
    #[test]
    fn 标题切段与顺序保持() {
        let md = "开场白一句\n# 身份\n你是运维助手。\n# 红线\n不许改库。\n## 子节\n子节属红线段。\n\n# 收尾\n   \n";
        let chunks = split_md_sections(md);
        assert_eq!(
            chunks,
            vec![
                "开场白一句",
                "# 身份\n你是运维助手。",
                "# 红线\n不许改库。\n## 子节\n子节属红线段。",
                "# 收尾",
            ]
        );
    }

    /// 无标题纯文本＝整文件一段（toml 字符串人设同形态，向后兼容）。
    #[test]
    fn 无标题整文件一段() {
        assert_eq!(split_md_sections("你是测试助手。"), vec!["你是测试助手。"]);
        assert!(split_md_sections("   \n\n").is_empty(), "空白文件零段");
    }
}
