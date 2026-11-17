//! 存量明文 key 兼容迁移（S5 验收项：key 不再明文进 yunyou.toml）。
//!
//! 流程（幂等，可随每次启动安全重跑）：读到 `llm.api_key` 为**明文**（非空且非
//! `$` 引用）→ ①写入宿主托管存储（[`crate::reference::LLM_API_KEY_REF`] 槽）→
//! ②**文本级定点改写**配置文件为引用——逐行扫描只在 `[llm]` 段内替换 api_key 行
//! 的值内容，其余字节（注释、空行、等号格式、其他段）原样保留（用户手写文件
//! 严禁整文件重序列化：toml crate 会毁注释与键序）。
//!
//! 防御口径（fail-loud，不静默）：配置文本解析失败即 Err；转义明文经 toml crate
//! 编码比对天然支持（不自行 unescape，见 replace_quoted_value）；引号段与解析面
//! 对不上即 Err 请手动处理；已是引用/空值/无 llm 段 → 无操作报告（`config_text=None`）。

use crate::reference::{is_reference, LLM_API_KEY_REF};
use crate::store::HostCredentialStore;
use yunyou::{CordisError, ErrorKind, Result};

/// 迁移报告：迁移了哪些字段 + 脱敏后的配置文本（None = 无需改写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// 完成迁移的字段（一期至多 `["llm.api_key"]`）。
    pub migrated: Vec<String>,
    /// 脱敏后的配置全文（调用方负责写回文件并保持 600 权限；None=原文未动）。
    pub config_text: Option<String>,
}

/// 检测并迁移 `llm.api_key` 明文（同步函数：存储写入与文本改写均为纯同步路径，
/// 调用方在宿主装配序列里调用；与 [`crate::CredentialService::set`] 共享同一落盘口）。
pub fn migrate_plaintext_key(store: &HostCredentialStore, config_text: &str) -> Result<MigrationReport> {
    // 前置 fail-loud：整文件必须可解析（防对损坏配置做文本手术）
    let raw: toml::Value = toml::from_str(config_text).map_err(|err| {
        // F-2：脱敏摘要（message+行号），不透传含明文 key 的源码行
        CordisError::new(
            ErrorKind::Internal,
            format!("迁移前置校验失败（配置文本不可解析）：{}", crate::toml_error_summary(config_text, &err)),
        )
    })?;
    let plaintext = raw
        .get("llm")
        .and_then(|llm| llm.get("api_key"))
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty());
    let Some(plaintext) = plaintext else {
        // 无值/空值：无操作（空值=未配置，官方铁律；缺段走默认值面）
        return Ok(MigrationReport { migrated: Vec::new(), config_text: None });
    };
    if is_reference(plaintext) {
        // 已是引用：幂等重跑的无操作分支
        return Ok(MigrationReport { migrated: Vec::new(), config_text: None });
    }
    // ① 明文入托管存储
    store.set_sync(LLM_API_KEY_REF, plaintext)?;
    // ② 文本级定点改写（只在 [llm] 段内的 api_key 行换值，其余字节原样）
    let rewritten = rewrite_llm_api_key_line(config_text, plaintext)?;
    Ok(MigrationReport {
        migrated: vec!["llm.api_key".to_string()],
        config_text: Some(rewritten),
    })
}

/// 逐行扫描：跟踪 `[section]` 头，仅在 `[llm]` 段内把 api_key 行的**引号内值**
/// 替换为引用；未找到目标行（值面有键但文本行缺失的不一致）即 Err。
fn rewrite_llm_api_key_line(text: &str, plaintext: &str) -> Result<String> {
    let mut in_llm_section = false;
    let mut replaced = false;
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_llm_section = trimmed == "[llm]";
            lines.push(line.to_string());
            continue;
        }
        let is_target = in_llm_section
            && trimmed.starts_with("api_key")
            && trimmed["api_key".len()..].trim_start().starts_with('=');
        if !is_target {
            lines.push(line.to_string());
            continue;
        }
        lines.push(replace_quoted_value(line, plaintext)?);
        replaced = true;
    }
    if !replaced {
        return Err(CordisError::new(
            ErrorKind::Internal,
            "迁移失败：值面存在 llm.api_key 明文但文本中未找到可改写行（配置面不一致，请手动处理）",
        ));
    }
    let mut out = lines.join("\n");
    // 保留原文末尾换行（lines() 会吞掉结尾 \n）
    if text.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// 替换一行中「包住 plaintext 的引号段」为引用，其余字节（缩进/键名/等号空格/
/// 引号风格/行尾注释）原样保留。
///
/// 引号段识别：扫描等号右侧的引号包裹段（单引号 literal 到下一个 `'`；双引号
/// basic 的 `\"`/`\\` 转义跳过），段内容与 plaintext 比对——literal 无转义直接剥
/// 引号比，basic 经 serde_json 解析（TOML basic string 与 JSON 字符串转义同构，
/// 转义语义由 crate 保证，不自行 unescape）；命中即整段替换为引用。
fn replace_quoted_value(line: &str, plaintext: &str) -> Result<String> {
    let eq = line.find('=').ok_or_else(|| {
        CordisError::new(ErrorKind::Internal, "api_key 行缺等号（解析面与文本面不一致），请手动处理")
    })?;
    let rest = line[eq + 1..].trim_start();
    let value_start = line.len() - rest.len();
    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let quote = chars[i].1;
        if quote != '"' && quote != '\'' {
            i += 1;
            continue;
        }
        let Some(close) = find_close_quote(&chars, i + 1, quote) else { break };
        let face: String = rest[chars[i].0..=char_index_to_byte(rest, close)].to_string();
        let matches = if quote == '\'' {
            &face[1..face.len() - 1] == plaintext
        } else {
            serde_json::from_str::<String>(&face).map(|value| value == plaintext).unwrap_or(false)
        };
        if matches {
            let value_end = value_start + char_index_to_byte(rest, close) + 1;
            let reference_value = format!("\"{LLM_API_KEY_REF}\"");
            return Ok(format!("{}{reference_value}{}", &line[..value_start], &line[value_end..]));
        }
        i = close + 1;
    }
    Err(CordisError::new(
        ErrorKind::Internal,
        "api_key 行的值与解析面不一致（未找到包住明文的引号段），请手动处理",
    ))
}

/// 从 from 开始找闭引号的 char 下标（basic string 转义对 `\"`/`\\` 跳两字符）。
fn find_close_quote(chars: &[(usize, char)], from: usize, quote: char) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        match chars[i].1 {
            '\\' if quote == '"' => i += 2,
            c if c == quote => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// char 下标 → byte 下标（引号段切回 &str 用）。
fn char_index_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(byte, _)| byte).unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CredentialService;
    use std::path::PathBuf;

    /// 独立临时目录。
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunyou-migrate-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("临时目录应可创建");
        dir
    }

    /// 端到端：明文 → 存储有值 + 文本改写为引用 + 注释/空行/其他段原样保留。
    #[tokio::test]
    async fn 明文迁移端到端() {
        let store = HostCredentialStore::open(temp_home("e2e").join("credentials.toml")).expect("打开");
        let text = "# 用户手写配置\n\n[llm]\nbase_url = \"http://127.0.0.1:8317/v1\"\napi_key = \"sk-live-abc\"  # 测试 key（注释必须保留）\nmodel = \"m\"\n\n[tools]\nroot_dir = \".\"\n";
        let report = migrate_plaintext_key(&store, text).expect("迁移应成功");
        assert_eq!(report.migrated, vec!["llm.api_key".to_string()]);
        let new_text = report.config_text.expect("应有改写文本");
        assert!(new_text.contains("api_key = \"$YUNYOU_LLM_API_KEY\""), "值应改写为引用：{new_text}");
        assert!(new_text.contains("# 用户手写配置"), "头注释应保留");
        assert!(new_text.contains("# 测试 key（注释必须保留）"), "行尾注释应保留");
        assert!(new_text.contains("root_dir = \".\""), "其他段原样");
        // 存储侧：resolve 拿到原明文值
        let handle = store.resolve(LLM_API_KEY_REF).await.expect("resolve 应成功").expect("存储应有迁移值");
        handle.with_value(|value| assert_eq!(value, "sk-live-abc"));
        // 幂等：重跑无操作
        let again = migrate_plaintext_key(&store, &new_text).expect("重跑应成功");
        assert!(again.migrated.is_empty() && again.config_text.is_none(), "已是引用应无操作：{again:?}");
        std::fs::remove_dir_all(temp_home("e2e")).ok();
    }

    /// 单引号 literal string：值替换后引号风格统一为双引号（引用无转义歧义）。
    #[tokio::test]
    async fn 单引号明文迁移() {
        let store = HostCredentialStore::open(temp_home("sq").join("credentials.toml")).expect("打开");
        let text = "[llm]\napi_key = 'sk-single'\n";
        let report = migrate_plaintext_key(&store, text).expect("迁移应成功");
        let new_text = report.config_text.expect("应有改写文本");
        assert!(new_text.contains("api_key = \"$YUNYOU_LLM_API_KEY\""), "{new_text}");
        std::fs::remove_dir_all(temp_home("sq")).ok();
    }

    /// 无操作三分：已引用/空值/无 llm 段——报告 migrated 空且 config_text=None。
    #[tokio::test]
    async fn 无操作三分() {
        let store = HostCredentialStore::open(temp_home("noop").join("credentials.toml")).expect("打开");
        for text in [
            "[llm]\napi_key = \"$YUNYOU_LLM_API_KEY\"\n",   // 已引用
            "[llm]\napi_key = \"\"\n",                     // 空值
            "[tools]\nroot_dir = \".\"\n",                 // 无 llm 段
        ] {
            let report = migrate_plaintext_key(&store, text).expect("无操作应成功");
            assert!(report.migrated.is_empty() && report.config_text.is_none(), "文本={text} 应无操作");
        }
        std::fs::remove_dir_all(temp_home("noop")).ok();
    }

    /// 段状态机：非 llm 段的 api_key 行不动（只迁 llm 段；一期唯一凭据槽）。
    #[tokio::test]
    async fn 段外同名键不动() {
        let store = HostCredentialStore::open(temp_home("seg").join("credentials.toml")).expect("打开");
        let text = "[other]\napi_key = \"sk-not-llm\"\n\n[llm]\napi_key = \"sk-llm\"\n";
        let report = migrate_plaintext_key(&store, text).expect("迁移应成功");
        let new_text = report.config_text.expect("应有改写文本");
        assert!(new_text.contains("sk-not-llm"), "段外 api_key 必须不动");
        assert!(new_text.contains("$YUNYOU_LLM_API_KEY"), "llm 段应改写");
        assert!(store.resolve(LLM_API_KEY_REF).await.expect("resolve 应成功").is_some());
        std::fs::remove_dir_all(temp_home("seg")).ok();
    }

    /// 转义明文支持：basic string 的转义表示（`\\`/`\"`）经 toml crate 编码比对正确迁移，
    /// 存储侧拿到的是解析后的真实值（unescape 后）。
    #[tokio::test]
    async fn 转义明文正确迁移() {
        let store = HostCredentialStore::open(temp_home("esc").join("credentials.toml")).expect("打开");
        // TOML 文本 "sk\\-x" → 解析值 sk\-x（含反斜杠）
        let text = "[llm]\napi_key = \"sk\\\\-x\"\n";
        let report = migrate_plaintext_key(&store, text).expect("转义明文应可迁移");
        assert!(report.config_text.expect("应有文本").contains("$YUNYOU_LLM_API_KEY"));
        let handle = store.resolve(LLM_API_KEY_REF).await.expect("resolve 应成功").expect("存储应有值");
        handle.with_value(|value| assert_eq!(value, "sk\\-x", "存储侧应是 unescape 后的真实值"));
        // 值内含双引号的明文（TOML 转义 \"）：文本引号段原文比对命中
        let text = "[llm]\napi_key = \"sk-\\\"quoted\\\"\"\n";
        let report = migrate_plaintext_key(&store, text).expect("含引号明文应可迁移");
        assert!(report.config_text.expect("应有文本").contains("$YUNYOU_LLM_API_KEY"));
        let handle = store.resolve(LLM_API_KEY_REF).await.expect("resolve 应成功").expect("存储应有值");
        handle.with_value(|value| assert_eq!(value, "sk-\"quoted\""));
        std::fs::remove_dir_all(temp_home("esc")).ok();
    }

    /// 防御：损坏文本 Err（fail-loud，防对损坏配置做文本手术）。
    #[tokio::test]
    async fn 损坏文本fail_loud() {
        let store = HostCredentialStore::open(temp_home("def").join("credentials.toml")).expect("打开");
        assert!(migrate_plaintext_key(&store, "!!!! 不是 toml !!!=").is_err());
        std::fs::remove_dir_all(temp_home("def")).ok();
    }

    /// 格式兼容：紧凑等号（`api_key="v"`）与多空格等号都应正确改写。
    #[tokio::test]
    async fn 紧凑与宽松等号格式兼容() {
        let store = HostCredentialStore::open(temp_home("fmt").join("credentials.toml")).expect("打开");
        for text in ["[llm]\napi_key=\"sk-x\"\n", "[llm]\napi_key   =   \"sk-x\"\n"] {
            let report = migrate_plaintext_key(&store, text).expect("迁移应成功");
            let new_text = report.config_text.expect("应有改写文本");
            assert!(new_text.contains("$YUNYOU_LLM_API_KEY"), "文本={text} 应改写：{new_text}");
            assert!(!new_text.contains("sk-x"), "明文必须从文本消失");
        }
        std::fs::remove_dir_all(temp_home("fmt")).ok();
    }
}
