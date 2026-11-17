//! 访问令牌铸造与校验（网关 D7：强制 token 鉴权，无匿名通道）。
//!
//! 铸造用 uuid v4 simple（32 位十六进制，≈122bit 随机熵）——不引 rand 新依赖
//! （uuid 已在 workspace 依赖收敛清单）；纯十六进制意味着 URL 传输无需百分号
//! 编码，查询串原样匹配安全。校验用常量时比较，防逐字节时序侧信道猜测。

use axum::http::{header, HeaderMap};

/// 铸造本进程访问令牌（每次 `yunyou web` 启动各铸一枚）。
pub fn mint() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 常量时比较：长度差与逐字节差全部进累积值，不因前缀命中提前返回。
pub fn verify(presented: &str, expected: &str) -> bool {
    let a = presented.as_bytes();
    let b = expected.as_bytes();
    // 长度差也进累积（截断成 u8 足够：只看是否非零）
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

/// 会话 cookie 名（v2 形态：浏览器子资源请求不带查询串——R2 token 抹除的自补
/// 实现走 cookie 会话，与对端 browser-auth 兼容（同语义异实现）：对端 HMAC 签名 cookie，
/// 我们本进程单实例直接持原值，httpOnly+SameSite=Lax）。
pub const SESSION_COOKIE: &str = "yunyou-web-token";

/// 从请求取候选令牌：查询串 `?token=` 优先（启动打印的 URL 即此形态），
/// 其次 `Authorization: Bearer`，再次会话 cookie（页内子资源形态）。
/// 三处皆无 → None（中间件拒绝，无匿名通道）。
pub fn from_request(query: Option<&str>, headers: &HeaderMap) -> Option<String> {
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some(rest) = pair.strip_prefix("token=") {
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
    }
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            if let Some(t) = text.strip_prefix("Bearer ").filter(|t| !t.is_empty()) {
                return Some(t.to_string());
            }
        }
    }
    cookie_token(headers.get(header::COOKIE))
}

/// 从 Cookie 头取会话令牌（多 cookie 分号分隔，取 ours）。
fn cookie_token(header_value: Option<&axum::http::HeaderValue>) -> Option<String> {
    let text = header_value?.to_str().ok()?;
    for pair in text.split(';') {
        if let Some(rest) = pair.trim().strip_prefix(SESSION_COOKIE) {
            let rest = rest.strip_prefix('=')?;
            let rest = rest.split(';').next()?;
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    /// 铸造物形态：32 位十六进制；两次铸造不重复（随机熵存在性验证）。
    #[test]
    fn 铸造物为32位十六进制且不重复() {
        let a = mint();
        let b = mint();
        assert_eq!(a.len(), 32, "uuid simple 应为 32 字符：{a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "应纯十六进制：{a}");
        assert_ne!(a, b, "两次铸造不应相同");
    }

    /// 常量时比较语义：相等真、前缀假、长度差假、空串假。
    #[test]
    fn 校验相等前缀与长度差() {
        let t = "abcdef0123456789";
        assert!(verify(t, t));
        let same_value: String = t.chars().collect(); // 同值不同分配，验的不是同一指针
        assert!(verify(t, &same_value), "同值不同串对象应真");
        assert!(!verify("abcdef012345678", t), "短前缀应假");
        assert!(!verify(t, "abcdef0123456780"), "尾字符差应假");
        assert!(!verify("", t));
        assert!(!verify(t, ""));
    }

    /// 取令牌：查询串优先、空 query token 回退 Bearer、两处皆无 None。
    #[test]
    fn 取令牌查询串优先与bearer回退() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer h1".parse().unwrap());
        assert_eq!(from_request(Some("token=q1&x=2"), &headers).as_deref(), Some("q1"));
        assert_eq!(from_request(Some("token=&x=2"), &headers).as_deref(), Some("h1"));
        assert_eq!(from_request(None, &headers).as_deref(), Some("h1"));
        assert_eq!(from_request(Some("x=2"), &HeaderMap::new()), None);
        // 非 Bearer 形态不认
        let mut raw = HeaderMap::new();
        raw.insert(header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(from_request(None, &raw), None);
    }

    /// 会话 cookie 兜底：多 cookie 混排取 ours、优先级低于查询串与 Bearer、
    /// 缺失/空值不认——v2 页内子资源与 WS 重连的通行形态。
    #[test]
    fn 会话cookie兜底与优先级() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; yunyou-web-token=c1; x=2".parse().unwrap(),
        );
        assert_eq!(from_request(None, &headers).as_deref(), Some("c1"));
        // 查询串优先
        assert_eq!(from_request(Some("token=q1"), &headers).as_deref(), Some("q1"));
        // Bearer 优先于 cookie
        let mut bearer = HeaderMap::new();
        bearer.insert(header::COOKIE, "yunyou-web-token=c1".parse().unwrap());
        bearer.insert(header::AUTHORIZATION, "Bearer b1".parse().unwrap());
        assert_eq!(from_request(None, &bearer).as_deref(), Some("b1"));
        // 无 ours / 空值不认
        let mut none = HeaderMap::new();
        none.insert(header::COOKIE, "a=1; b=".parse().unwrap());
        assert_eq!(from_request(None, &none), None);
    }
}
