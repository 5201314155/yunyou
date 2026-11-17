//! 两段式令牌（网关落地方案 §3.1）：配对令牌（一次性，换会话令牌后原子失效）
//! 与会话令牌（长期可吊销，30 天滑动过期随阶段 B 活跃度接线）。
//!
//! 形态纪律：`dsh_pair_<base64url(32B)>` / `dsh_sess_<base64url(32B)>`——前缀
//! 即类型（日志/审计一眼可辨），base64url 无填充（URL 安全：配对令牌进 QR
//! payload 与 HTTP Bearer 都不需转义）。存储侧一律 SHA-256 哈希（§3.1「库
//! 泄露不泄可用令牌」），明文只在签发时刻存在于调用方手里。

use sha2::{Digest, Sha256};

/// 配对令牌前缀。
pub const PAIRING_PREFIX: &str = "dsh_pair_";
/// 会话令牌前缀。
pub const SESSION_PREFIX: &str = "dsh_sess_";

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url 编码（无填充，方案 §3.1 原文——URL 安全场景填充符是噪音）。
pub fn b64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64URL[((n >> 18) & 63) as usize] as char);
        out.push(B64URL[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64URL[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64URL[(n & 63) as usize] as char);
        }
    }
    out
}

/// SHA-256 → 小写十六进制（存储哈希形态，§3.1）。
pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// OS CSPRNG 取 n 字节（方案 §3.1 随机源=getrandom；失败 panic=缺熵环境
/// 无法安全签发，fail-loud 优于静默降级）。
fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("CSPRNG 缺熵：无法签发凭证（fail-loud）");
    buf
}

/// 32B 随机 → `dsh_pair_` 前缀配对令牌明文。
pub fn new_pairing_plaintext() -> String {
    format!("{PAIRING_PREFIX}{}", b64url_encode(&random_bytes(32)))
}

/// 32B 随机 → `dsh_sess_` 前缀会话令牌明文。
pub fn new_session_plaintext() -> String {
    format!("{SESSION_PREFIX}{}", b64url_encode(&random_bytes(32)))
}

/// 令牌明文 → 存储哈希（SHA-256 十六进制；查表键与入库值同函数=单源）。
pub fn hash_plaintext(plaintext: &str) -> String {
    sha256_hex(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形态锁：前缀+长度（32B→43 字符无填充）+字符集 URL 安全（无 +/=）。
    #[test]
    fn 令牌形态与前缀() {
        let pairing = new_pairing_plaintext();
        let session = new_session_plaintext();
        assert!(pairing.starts_with(PAIRING_PREFIX), "{pairing}");
        assert!(session.starts_with(SESSION_PREFIX), "{session}");
        let body = pairing.trim_start_matches(PAIRING_PREFIX);
        assert_eq!(body.len(), 43, "32B base64url 无填充=43 字符: {body}");
        assert!(!body.contains('+') && !body.contains('/') && !body.contains('='));
        // 唯一性（CSPRNG 抽样；32B 碰撞概率天文小）
        assert_ne!(new_pairing_plaintext(), new_pairing_plaintext());
    }

    /// 哈希稳定与雪崩（存储哈希=查表键单源；不同输入必不同哈希）。
    #[test]
    fn 哈希稳定且区分输入() {
        assert_eq!(sha256_hex("abc"), sha256_hex("abc"));
        assert_ne!(hash_plaintext("dsh_pair_a"), hash_plaintext("dsh_pair_b"));
        assert_eq!(sha256_hex("abc").len(), 64, "SHA-256 十六进制 64 字符");
    }

    /// base64url 已知向量（RFC 4648 测试向量去填充版）。
    #[test]
    fn base64url_已知向量() {
        assert_eq!(b64url_encode(b""), "");
        assert_eq!(b64url_encode(b"f"), "Zg");
        assert_eq!(b64url_encode(b"fo"), "Zm8");
        assert_eq!(b64url_encode(b"foo"), "Zm9v");
        assert_eq!(b64url_encode(b"foob"), "Zm9vYg");
        assert_eq!(b64url_encode(b"foobar"), "Zm9vYmFy");
    }
}
