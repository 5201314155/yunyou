//! 技能纯函数面（skills 下沉批，lead 2026-11-19 派单）：自 yunyou-cli/src/
//! skills.rs 迁出——web（web.v1 skills 域）与 cli 平行消费，零内核依赖
//! （模块守卫=禁运断言 tests/zero_kernel_dep.rs，SDK 先例同款）。
//!
//! # 槽位纪律（D4 裁决双扫描）
//! 项目根 `.yunyou/skills` 优先 + `.dsh/skills` 过渡兼容，两槽同名 `.yunyou` 胜。
//! 容错：单文件非法/读取失败/同名冲突只跳过该文件，绝不中断其余扫描。
//!
//! # 命令名守卫
//! `^[a-z][a-z0-9_-]*$`（设计 §七）；描述=剥 frontmatter 后首个一级标题
//! （缺标题的技能是坏目录条目，跳过不装哑命令）。

use std::path::Path;

/// 目录段（harness:skills）段名与档位：人设槽 0 与工具段 1000 之间的空隙
/// （守段位表「新段选空隙」纪律）；段位常量归本模块（技能目录是宿主资源面）。
pub const SKILLS_SECTION_NAME: &str = "harness:skills";
/// 目录段档位。
pub const SKILLS_SECTION_ORDER: f64 = 500.0;
/// 目录段 token 预算（字符口径：中文 1 字≈1 token，2000 字符≈2k token）。
const CATALOG_BUDGET_CHARS: usize = 2000;
/// 单条描述截断上限（字符；描述=first_heading 短句，60 字已够判相关性）。
const CATALOG_DESC_MAX_CHARS: usize = 60;
/// 目录段引导语（lead 批注 a）：模型不用自己猜「先看目录还是先读全文」。
const CATALOG_GUIDE: &str = "以下为可用技能目录（名+描述）。写插件/查能力前先 skill_read plugins-seams；需要某技能详情时用 skill_read 按名读全文。";

/// 技能目录条目：命令名 + 描述（元数据常驻面，正文不进）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: String,
    pub description: String,
}

/// 技能命令名守卫（设计 §七）：`^[a-z][a-z0-9_-]*$`。
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// 剥 frontmatter：文件以 `---` 行开头 → 到下一 `---` 行之间为元数据区。
/// 返回（alias 选项，正文）；无 frontmatter 原文返回。一期只认 `alias:` 键
/// （设计 §七：中文名别名，缺省无别名）。
pub fn split_frontmatter(text: &str) -> (Option<String>, &str) {
    let Some(rest) = text.strip_prefix("---") else { return (None, text) };
    if !rest.starts_with('\n') {
        return (None, text); // 非 frontmatter 开头（如 "---" 出现在正文首）按原文处理
    }
    let Some(end) = rest.find("\n---") else { return (None, text) };
    let meta = &rest[..end];
    let after = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);
    let alias = meta
        .lines()
        .find_map(|line| line.strip_prefix("alias:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    (alias, after)
}

/// 描述=剥 frontmatter 后首个一级标题（`# 标题`；缺标题=不可目录化）。
pub fn first_heading(text: &str) -> Option<&str> {
    text.lines().find_map(|line| line.strip_prefix("# ")).map(str::trim).filter(|h| !h.is_empty())
}

/// 双槽技能目录扫描（task-13①）：`.yunyou` 优先 + `.dsh` 过渡，同名 `.yunyou` 胜。
/// 输出按文件名序稳定排序（不随 read_dir 顺序漂移）。
pub fn scan_skill_catalog(cwd: &Path) -> Vec<CatalogEntry> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entries: Vec<CatalogEntry> = Vec::new();
    for slot_dir in [cwd.join(".yunyou").join("skills"), cwd.join(".dsh").join("skills")] {
        let Ok(dir_entries) = std::fs::read_dir(&slot_dir) else { continue };
        let mut files: Vec<std::path::PathBuf> = dir_entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        files.sort();
        for path in files {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if !valid_name(stem) || !seen.insert(stem.to_string()) {
                continue; // 非法名 / 同名（.yunyou 已收录）跳过
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let (_, body) = split_frontmatter(&text);
            let Some(description) = first_heading(body) else { continue };
            entries.push(CatalogEntry { name: stem.to_string(), description: description.to_string() });
        }
    }
    entries
}

/// 目录段文本（task-13①，lead 批预算）：引导语+每技能一行 `- name: 描述`；
/// 描述截断 60 字符；段总文本超 2000 字符预算即截断并加尾注计数。
pub fn catalog_section_text(entries: &[CatalogEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let truncate = |text: &str| -> String {
        if text.chars().count() <= CATALOG_DESC_MAX_CHARS {
            text.to_string()
        } else {
            let cut: String = text.chars().take(CATALOG_DESC_MAX_CHARS).collect();
            format!("{cut}…")
        }
    };
    let mut lines: Vec<String> = Vec::new();
    let mut omitted = 0usize;
    for entry in entries {
        let line = format!("- {}: {}", entry.name, truncate(&entry.description));
        let projected_len = lines.iter().map(String::len).sum::<usize>() + line.len() + CATALOG_GUIDE.len();
        if projected_len > CATALOG_BUDGET_CHARS {
            omitted += 1;
            continue;
        }
        lines.push(line);
    }
    if omitted > 0 {
        lines.push(format!("…另有 {omitted} 个技能未列入（/skills 查看全量）"));
    }
    format!("{CATALOG_GUIDE}\n{}", lines.join("\n"))
}

/// 按名读技能全文（web.v1 skills/get 消费）：双槽解析+frontmatter 剥离→正文。
/// 返回（描述, 正文）；非法名/文件缺席/无标题=None——调用方人话拒绝，不伪造。
pub fn read_skill_body(cwd: &Path, id: &str) -> Option<(String, String)> {
    if !valid_name(id) {
        return None;
    }
    for slot_dir in [cwd.join(".yunyou").join("skills"), cwd.join(".dsh").join("skills")] {
        let path = slot_dir.join(format!("{id}.md"));
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let (_, body) = split_frontmatter(&text);
        let Some(description) = first_heading(body) else { continue };
        return Some((description.to_string(), body.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造 temp 技能槽（content 对：(.yunyou, name, text)）。
    fn slot_with(files: &[(&str, &str, &str)]) -> std::path::PathBuf {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yunyou-skills-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (slot, name, text) in files {
            let path = dir.join(slot).join("skills");
            std::fs::create_dir_all(&path).expect("槽目录");
            std::fs::write(path.join(format!("{name}.md")), text).expect("写技能文件");
        }
        dir
    }

    /// 命令名守卫边界：合法形态/首字符大写/数字开头/中文/空串全拒。
    #[test]
    fn 命令名守卫边界() {
        assert!(valid_name("a") && valid_name("web-search") && valid_name("k2_design"));
        assert!(!valid_name("Aaa") && !valid_name("1abc") && !valid_name("中文") && !valid_name(""));
        assert!(!valid_name("-lead"), "首字符必须小写字母");
    }

    /// frontmatter 三态：无 frontmatter 原文/有 alias 剥离/缺尾（无 \n---）按原文。
    #[test]
    fn frontmatter三态() {
        assert_eq!(split_frontmatter("# 直接正文"), (None, "# 直接正文"));
        let (alias, body) = split_frontmatter("---\nalias: 中文名\n---\n# 标题\n正文");
        assert_eq!(alias.as_deref(), Some("中文名"));
        assert_eq!(body, "# 标题\n正文");
        assert_eq!(split_frontmatter("---\nalias: 无尾\n").0, None, "缺尾=原文");
    }

    /// 双槽扫描：同名 .yunyou 胜（.dsh 同名不重复出列）；非法名/无标题跳过。
    #[test]
    fn 双槽扫描同名优先与容错() {
        let dir = slot_with(&[
            (".dsh", "dup", "# dsh 版"),
            (".yunyou", "dup", "# yunyou 版"),
            (".yunyou", "Bad", "# 非法名"),
            (".yunyou", "noheading", "没有标题"),
            (".yunyou", "good", "# 好技能"),
        ]);
        let entries = scan_skill_catalog(&dir);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["dup", "good"], "非法/无标题跳过+稳定排序：{names:?}");
        let dup = entries.iter().find(|e| e.name == "dup").expect("dup 在");
        assert_eq!(dup.description, "yunyou 版", "同名 .yunyou 胜");
    }

    /// 目录段文本：空目录零输出；预算截断加尾注。
    #[test]
    fn 目录段文本与预算() {
        assert_eq!(catalog_section_text(&[]), "");
        let entries = vec![CatalogEntry { name: "a".into(), description: "描述".into() }];
        let text = catalog_section_text(&entries);
        assert!(text.contains("- a: 描述") && text.starts_with("以下为可用技能目录"), "{text}");
        // 预算：80 条长描述→截断尾注在
        let many: Vec<CatalogEntry> = (0..80)
            .map(|i| CatalogEntry { name: format!("s{i}"), description: "很".repeat(80) })
            .collect();
        let text = catalog_section_text(&many);
        assert!(text.contains("未列入"), "预算截断尾注：{}", &text[text.len() - 80..]);
    }

    /// 按名读全文：正文=frontmatter 后全量；缺席/非法名=None（不伪造）。
    #[test]
    fn 读全文与缺席() {
        let dir = slot_with(&[(".yunyou", "good", "---\nalias: 好技能\n---\n# 好技能\n正文第一行\n正文第二行")]);
        let (description, body) = read_skill_body(&dir, "good").expect("读全文在");
        assert_eq!(description, "好技能");
        assert_eq!(body, "# 好技能\n正文第一行\n正文第二行", "frontmatter 已剥：{body}");
        assert!(read_skill_body(&dir, "nope").is_none(), "缺席=None");
        assert!(read_skill_body(&dir, "Bad").is_none(), "非法名=None");
    }
}
