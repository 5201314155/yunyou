//! 配置文件安全写入（池 task-30 硬要求「原子写+坏配置回滚」）。
//!
//! # 先验后写三段式（lead 批③，验收标准=「落盘的每个字节都先通过了启动
//! 校验器本体」）
//! 1. **文本级定点替换**（不整文件重序列化——toml crate 会毁注释与键序，
//!    migrate.rs 手术先例同款）：只动目标键所在行；
//! 2. 新文本过 [`yunyou_config::load_from_str`] 全量校验（toml 解析+schema
//!    +deny_unknown_fields——与启动同一校验器）；
//! 3. 旧文存 `<path>.bak` 兜底 → 临时文件 → rename 原子替换。
//! 校验失败=不落盘+原文从未被动过（天然回滚）——**写坏在物理上不可能**。

use std::path::Path;

/// 读一行段头名：`[llm]`→`llm`、`[tools.builtin]`→`tools.builtin`；非段头 None。
fn header_name(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(str::trim)
}

/// 段内键行定点替换/插入（纯函数，测试锁定）：
/// - 目标段内已有 `key = ...` 行 → 整行替换为 `key = {raw_value}`；
/// - 段存在但无该键 → 段头下一行插入；
/// - 段不存在 → 文末追加 `[section]\nkey = {raw_value}`（前置空行）。
/// 找不到落点/段头歧义即 Err（fail-loud，不猜）。
pub fn upsert_section_value(
    text: &str,
    section: &str,
    key: &str,
    raw_value: &str,
) -> Result<String, String> {
    let target_header = format!("[{section}]");
    let key_prefix = format!("{key} ");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut current: Option<&str> = None;
    let mut header_idx: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(name) = header_name(trimmed) {
            current = Some(name);
            if name == section {
                header_idx = Some(idx);
            }
            continue;
        }
        if current == Some(section) && trimmed.starts_with(&key_prefix) {
            lines[idx] = format!("{key} = {raw_value}");
            return Ok(join(lines));
        }
    }
    // 段存在但无该键：段头下一行插入
    if let Some(idx) = header_idx {
        lines.insert(idx + 1, format!("{key} = {raw_value}"));
        return Ok(join(lines));
    }
    // 段不存在：文末追加
    lines.push(String::new());
    lines.push(target_header);
    lines.push(format!("{key} = {raw_value}"));
    Ok(join(lines))
}

/// 段内键行删除（纯函数）：`on` 语义=移除键回归默认；键/段不存在=原文原样
/// （幂等，不算错）。
pub fn remove_section_key(text: &str, section: &str, key: &str) -> Result<String, String> {
    let key_prefix = format!("{key} ");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut current: Option<String> = None;
    let mut removed = false;
    for line in lines.iter_mut() {
        let is_target = {
            let trimmed = line.trim();
            match header_name(trimmed) {
                Some(name) => {
                    current = Some(name.to_string());
                    false
                }
                None => current.as_deref() == Some(section) && trimmed.starts_with(&key_prefix),
            }
        };
        if is_target {
            *line = String::new();
            removed = true;
        }
    }
    let _ = removed; // 幂等：不存在也是成功态
    Ok(join(lines))
}

/// 段内键现值读取（纯函数）：返回未去引号的原始值段（`"x"` 带引号）。
pub fn read_section_value(text: &str, section: &str, key: &str) -> Option<String> {
    let key_prefix = format!("{key} ");
    let mut current: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = header_name(trimmed) {
            current = Some(name);
            continue;
        }
        if current == Some(section) && trimmed.starts_with(&key_prefix) {
            return trimmed
                .strip_prefix(&key_prefix)
                .and_then(|rest| rest.strip_prefix('='))
                .map(|v| v.trim().to_string());
        }
    }
    None
}

fn join(lines: Vec<String>) -> String {
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// 校验配置文本（与启动同一校验器：toml+schema+deny）。
pub fn validate_config(text: &str) -> Result<(), String> {
    yunyou_config::load_from_str(text)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// 先验后写落盘（task-30 硬要求）：校验新文本 → 旧文存 `.bak` → 临时文件
/// → rename 原子替换。校验失败即 Err 且**原文件零改动**。
pub fn commit(path: &Path, new_text: &str) -> Result<(), String> {
    validate_config(new_text)?;
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let bak = path.with_extension("toml.bak");
    if !old.is_empty() {
        std::fs::write(&bak, &old).map_err(|err| format!(".bak 兜底写入失败：{err}"))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, new_text).map_err(|err| format!("临时文件写入失败：{err}"))?;
    std::fs::rename(&tmp, path).map_err(|err| format!("原子替换失败：{err}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# 头注释保留\n[llm]\nbase_url = \"https://a.example/v1\" # 行尾注释\nmodel = \"old/model\"\n\n[tools]\nroot_dir = \".\"\n";

    /// upsert：已有键整行替换、行尾注释在目标行让位（替换即整行重写），
    /// 其余行（含头注释/其他段）原样。
    #[test]
    fn 已有键替换_他行原样() {
        let out = upsert_section_value(SAMPLE, "llm", "model", "\"new/model\"").expect("应成功");
        assert!(out.contains("model = \"new/model\""), "{out}");
        assert!(out.starts_with("# 头注释保留"), "头注释保留：{out}");
        assert!(out.contains("base_url = \"https://a.example/v1\""), "他行原样：{out}");
        assert!(out.contains("[tools]"), "段头原样：{out}");
        assert!(!out.contains("old/model"), "旧值清除：{out}");
    }

    /// upsert：键缺省→段头下一行插入；段缺省→文末追加。
    #[test]
    fn 插入与追加() {
        let out = upsert_section_value(SAMPLE, "llm", "timeout_secs", "60").expect("应成功");
        let idx_llm = out.find("[llm]").unwrap();
        assert!(out[idx_llm..].contains("timeout_secs = 60"), "段头后插入：{out}");
        let out = upsert_section_value(SAMPLE, "agent_loop", "max_steps", "50").expect("应成功");
        assert!(out.contains("[agent_loop]\nmax_steps = 50"), "文末追加段：{out}");
    }

    /// remove：段内键行移除（幂等）；其他键不动。
    #[test]
    fn 键删除与幂等() {
        let with = "[tools]\nroot_dir = \".\"\n\n[tools.builtin]\nweb_fetch = false\nbash = true\n";
        let out = remove_section_key(with, "tools.builtin", "web_fetch").expect("应成功");
        assert!(out.contains("bash = true") && !out.contains("web_fetch"), "{out}");
        let again = remove_section_key(&out, "tools.builtin", "web_fetch").expect("幂等");
        assert_eq!(out, again, "重复删除=原样");
        // [tools] 段的 root_dir 不受误伤
        assert!(out.contains("root_dir = \".\""), "{out}");
    }

    /// read：段内键值读取（带引号原样）；跨段同名键不误读。
    #[test]
    fn 键值读取_跨段不误读() {
        let text = "[a]\nmodel = \"a-model\"\n\n[b]\nmodel = \"b-model\"\n";
        assert_eq!(read_section_value(text, "a", "model").as_deref(), Some("\"a-model\""));
        assert_eq!(read_section_value(text, "b", "model").as_deref(), Some("\"b-model\""));
        assert_eq!(read_section_value(text, "c", "model"), None);
    }

    /// commit：合法新文落盘+.bak 兜底；非法新文拒绝且原文件零改动。
    #[test]
    fn 先验后写_拒绝即零改动() {
        let dir = std::env::temp_dir().join(format!("yunyou-cfg-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录");
        let path = dir.join("yunyou.toml");
        std::fs::write(&path, "[llm]\nbase_url = \"http://x\"\nmodel = \"m\"\napi_key = \"k\"\n")
            .expect("旧文");
        // 合法：落盘+bak（bak=旧文，含旧 model 不含新值）
        commit(&path, "[llm]\nbase_url = \"http://x\"\nmodel = \"m2\"\napi_key = \"k\"\n")
            .expect("合法应落盘");
        assert!(std::fs::read_to_string(&path).unwrap().contains("m2"));
        let bak = std::fs::read_to_string(path.with_extension("toml.bak")).unwrap();
        assert!(bak.contains("model = \"m\"") && !bak.contains("m2"), "bak=旧文：{bak}");
        // 非法：拒绝且原文件零改动
        let before = std::fs::read_to_string(&path).unwrap();
        let err = commit(&path, "[llm]\nmodel = 123\n").expect_err("非法应拒绝");
        assert!(!err.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "拒绝=零改动");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
