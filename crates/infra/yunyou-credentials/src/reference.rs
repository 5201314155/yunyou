//! 凭据引用语法（契约⑤配置面）：`$NAME`，NAME 匹配 POSIX 环境变量名。
//!
//! 官方对照：`packages/credentials/credentials/src/index.ts` 的
//! `REF_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/`——官方 CredentialRef 是**裸**环境变量名
//! （官方配置面从不存明文，无歧义）。yunyou 存量 yunyou.toml 的 api_key 是明文
//! （迁移期兼容），故引用必须显式消歧：一律 `$` 前缀（明文 key 不以 `$` 开头），
//! 迁移完成后配置面永无明文（S5 验收「key 不再明文进 yunyou.toml」）。

/// 引用前缀：`$NAME` 形式即凭据引用。
pub const REF_PREFIX: char = '$';

/// 迁移引用名约定：`llm.api_key` 的宿主托管引用（一期唯一凭据槽；一期只迁移
/// llm 段 api_key 一个字段，后续凭据槽随插件需求按 `<用途>` 命名扩展）。
pub const LLM_API_KEY_REF: &str = "$YUNYOU_LLM_API_KEY";

/// 引用名段语法：POSIX 环境变量名（官方 REF_PATTERN 同构）。
fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 是否凭据引用（`$` 前缀 + 合法 POSIX 名；`$` 后非法即**不是**引用——
/// 按「值恰好以 $ 开头的明文」对待，交给迁移器升级，不在此抛错）。
pub fn is_reference(value: &str) -> bool {
    match value.strip_prefix(REF_PREFIX) {
        Some(name) => is_valid_name(name),
        None => false,
    }
}

/// 剥 `$` 前缀取引用名（已验合法性；供 resolve/set 定位存储槽与环境变量名）。
pub fn reference_name(value: &str) -> Option<&str> {
    if is_reference(value) { Some(&value[1..]) } else { None }
}

/// 引用合法性校验（resolve/set 的前置防线）：非 `$` 引用即 Err，`$` 后名字非法也 Err
/// ——`$` 前缀是调用方的显式声明，名字非法属于配置错误，fail-loud（对齐官方
/// `isCredentialRefName` 的问询语义：官方无前缀故需问询，yunyou 前缀已显式化）。
pub fn validate_reference(value: &str) -> Result<(), String> {
    match value.strip_prefix(REF_PREFIX) {
        Some(name) if is_valid_name(name) => Ok(()),
        Some(_) => Err(format!(
            "凭据引用 {value:?} 的名字段非法：应为 POSIX 环境变量名（字母/数字/下划线，首字符非数字）"
        )),
        None => Err(format!(
            "凭据引用 {value:?} 缺 {} 前缀：配置面一律以引用形式持有密钥（明文应走迁移器）",
            REF_PREFIX
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 引用识别：合法名/非法名/裸明文三分。
    #[test]
    fn 引用识别三分() {
        assert!(is_reference("$YUNYOU_LLM_API_KEY"));
        assert!(is_reference("$_LOWER_2"));
        // $ 后非法名：不是引用（按「恰好以 $ 开头的明文」对待）
        assert!(!is_reference("$9BAD"));
        assert!(!is_reference("$with-dash"));
        assert!(!is_reference("$"));
        // 无前缀=明文（迁移期兼容面）
        assert!(!is_reference("sk-abc123"));
        assert!(!is_reference(""));
    }

    /// 名字剥离与校验：合法剥名、明文与非法名 Err（中文报错点名前缀）。
    #[test]
    fn 剥名与校验() {
        assert_eq!(reference_name("$A_B"), Some("A_B"));
        assert_eq!(reference_name("plain"), None);
        assert!(validate_reference("$A_B").is_ok());
        let err = validate_reference("sk-plain").unwrap_err();
        assert!(err.contains("前缀"), "明文报错应点名 $ 前缀：{err}");
        let err = validate_reference("$bad-name").unwrap_err();
        assert!(err.contains("名字段非法"), "非法名报错应点名名字段：{err}");
    }
}
