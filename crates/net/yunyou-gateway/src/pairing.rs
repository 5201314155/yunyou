//! 配对双式纯逻辑（网关落地方案 §4：扫码/验证码，两维正交——差异仅载体与
//! endpoint 形态，token 鉴权路径四格完全一致 §6）。
//!
//! 流程（§4.1/§4.2）：issue（签发配对凭证+登记配对会话）→ consume（原子一次
//! 性消费→设备数上限检查→签发会话令牌+登记设备+惰性会话映射）。失败锁定双维：
//! 配对会话级（fail_count 5 次→锁 15 分钟，本文件判定落库）+源 IP 级（locks
//! 表计数，阶段 B 中间件接线）。

use crate::store::{GatewayStore, PairingRow, TokenRow};
use crate::token::{hash_plaintext, new_pairing_plaintext, new_session_plaintext};
use getrandom::getrandom;
use std::fmt;

/// 配对方式（§4 两维之一）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingMethod {
    /// 扫码（§4.1：QR 载荷 ws(s)://endpoint+配对令牌，时效 5 分钟）。
    Qr,
    /// 验证码（§4.2：8 位短码设备码流程，时效 10 分钟，对标 Device Flow 简化版）。
    DeviceCode,
}

impl PairingMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Qr => "qr",
            Self::DeviceCode => "device_code",
        }
    }

    /// 配对凭证时效（§4.1 扫码 5 分钟/§4.2 验证码 10 分钟）。
    fn ttl_ms(self) -> u64 {
        match self {
            Self::Qr => 5 * 60 * 1000,
            Self::DeviceCode => 10 * 60 * 1000,
        }
    }
}

/// 网络维度（§5 两维之二）：差异仅 endpoint 形态与 TLS（阶段 B），纯逻辑层
/// 只作登记（审计与矩阵测试对账用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkDimension {
    /// 内网（§5.1：局域网明文 ws，依托内网边界）。
    Lan,
    /// 公网（§5.2：TLS 强制 fail-loud——阶段 B 网络层执行）。
    Public,
}

impl NetworkDimension {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lan => "lan",
            Self::Public => "public",
        }
    }
}

/// 配对凭证 TTL（毫秒；按方式取，方案 §4.1/§4.2 定案值）。
pub const QR_TTL_MS: u64 = 5 * 60 * 1000;
pub const DEVICE_CODE_TTL_MS: u64 = 10 * 60 * 1000;
/// 连续失败锁定阈值与时长（§3.2 定案：5 次→15 分钟）。
pub const FAIL_LIMIT: u32 = 5;
pub const LOCK_MS: u64 = 15 * 60 * 1000;
/// 活跃设备数上限（§3.1 评审建议采纳，默认 10；可配=TeamConfig 式留位）。
pub const DEVICE_LIMIT: u32 = 10;
/// 验证码字符集（§4.2：去掉 0O1lI 的 32 字符集，8 位≈1.1 万亿空间防爆破）。
const SHORT_CODE_ALPHABET: &[u8] = b"23456789abcdefghjkmnpqrstuvwxyzABCDEFGHJKMNPQRSTUVWXYZ";
/// 验证码长度。
const SHORT_CODE_LEN: usize = 8;

/// 消费失败（fail-loud 人话；审计由调用方按变体留痕）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumeError {
    /// 凭证未登记（typo/伪造——auth_failure 留痕）。
    UnknownToken,
    /// 已被消费（重放拒绝——原子一次性语义）。
    AlreadyConsumed,
    /// 已过期。
    Expired,
    /// 失败锁定中（§3.2：5 次失败→15 分钟）。
    Locked { until_ms: u64 },
    /// 设备数上限（§3.1：活跃设备 10 台上限，409 对应物）。
    DeviceLimit { limit: u32 },
    /// 存储层错误（rusqlite 透传——不吞错）。
    Storage(String),
}

impl fmt::Display for ConsumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownToken => write!(f, "配对凭证无效（未登记或已失效）"),
            Self::AlreadyConsumed => write!(f, "配对凭证已被使用（一次性语义，防重放）"),
            Self::Expired => write!(f, "配对凭证已过期，请重新配对"),
            Self::Locked { until_ms } => {
                write!(f, "连续失败已锁定至 {until_ms}（15 分钟后可重试）")
            }
            Self::DeviceLimit { limit } => {
                write!(f, "活跃设备数已达上限 {limit}，请先移除旧设备再配对")
            }
            Self::Storage(err) => write!(f, "网关存储错误：{err}"),
        }
    }
}

/// 签发产出（issue 返回；明文只在签发时刻存在于调用方——存储侧一律哈希）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingIssue {
    /// 配对凭证明文：扫码=dsh_pair_ 令牌；验证码=8 位短码。
    pub credential: String,
    /// 过期时刻（unix 毫秒）。
    pub expires_at_ms: u64,
    /// 扫码载荷（§4.1 schema：{v,kind,endpoint,pairing_token,expires_at}）；
    /// 验证码=None（短码由界面直显，无 QR 载荷）。
    pub qr_payload: Option<String>,
    /// 配对方式（回显）。
    pub method: PairingMethod,
}

/// 8 位短码铸造（§4.2 字符集去 0O1lI；CSPRNG 取模拒绝偏差——逐字符重采样，
/// 无 256%32 偏差面）。
fn short_code() -> String {
    let alphabet_len = SHORT_CODE_ALPHABET.len() as u8; // 32 → 256%32=0 无偏差
    (0..SHORT_CODE_LEN)
        .map(|_| {
            let mut buf = [0u8; 1];
            getrandom(&mut buf).expect("CSPRNG 缺熵：无法铸造验证码（fail-loud）");
            SHORT_CODE_ALPHABET[(buf[0] % alphabet_len) as usize] as char
        })
        .collect()
}

/// 签发配对（issue）：铸造凭证→哈希入库（未消费）→产出（扫码附 QR 载荷）。
/// 纯逻辑+存储，无网络（endpoint 形态由调用方传——阶段 B 接真实监听地址）。
pub fn issue(
    store: &GatewayStore,
    method: PairingMethod,
    network: NetworkDimension,
    endpoint: &str,
    now_ms: u64,
) -> Result<PairingIssue, ConsumeError> {
    let credential = match method {
        PairingMethod::Qr => new_pairing_plaintext(),
        PairingMethod::DeviceCode => short_code(),
    };
    let token_hash = hash_plaintext(&credential);
    // 验证码流程：短码哈希另存一列（App 提交的是短码原文，按码哈希检索）
    let device_code_hash = if method == PairingMethod::DeviceCode {
        Some(hash_plaintext(&credential))
    } else {
        None
    };
    let expires_at_ms = now_ms + method.ttl_ms();
    let row = PairingRow {
        token_hash: token_hash.clone(),
        method: method.as_str().into(),
        network: network.as_str().into(),
        device_code_hash,
        expires_at_ms,
        consumed_at_ms: None,
        fail_count: 0,
        locked_until_ms: None,
    };
    store.create_pairing(&row).map_err(|err| ConsumeError::Storage(err.to_string()))?;
    let qr_payload = match method {
        PairingMethod::Qr => Some(format!(
            "{{\"v\":1,\"kind\":\"yunyou-gateway\",\"endpoint\":\"{endpoint}\",\"pairing_token\":\"{credential}\",\"expires_at\":{expires_at_ms}}}"
        )),
        PairingMethod::DeviceCode => None,
    };
    Ok(PairingIssue { credential, expires_at_ms, qr_payload, method })
}

/// 消费配对（consume）：凭证哈希检索→锁定/时效/一次性三闸→设备数上限→
/// 签发会话令牌+登记设备+惰性会话映射。审查各失败分支均落审计由调用方执行
/// （本函数返回结构化错误，人话见 Display）。
#[allow(clippy::too_many_lines)]
pub fn consume(
    store: &GatewayStore,
    credential: &str,
    device_name: &str,
    now_ms: u64,
) -> Result<SessionIssued, ConsumeError> {
    let token_hash = hash_plaintext(credential);
    // 双式检索：先按令牌哈希（扫码），再按短码哈希（验证码）
    let by_token = store
        .find_pairing_by_token(&token_hash)
        .map_err(|err| ConsumeError::Storage(err.to_string()))?;
    let row = match by_token {
        Some(row) => Some(row),
        None => store
            .find_pairing_by_code(&token_hash)
            .map_err(|err| ConsumeError::Storage(err.to_string()))?,
    };
    let Some(row) = row else {
        return Err(ConsumeError::UnknownToken);
    };
    // 闸一：失败锁定（§3.2 配对会话级，先于一切）
    if let Some(until) = row.locked_until_ms {
        if now_ms < until {
            return Err(ConsumeError::Locked { until_ms: until });
        }
    }
    // 闸二：过期（先记失败计数——过期凭证上的反复尝试=爆破行为面）
    if row.expires_at_ms <= now_ms {
        let count = store.bump_pairing_fail(&row.token_hash).map_err(|e| ConsumeError::Storage(e.to_string()))?;
        if count >= FAIL_LIMIT {
            store.lock_pairing(&row.token_hash, now_ms + LOCK_MS).map_err(|e| ConsumeError::Storage(e.to_string()))?;
        }
        return Err(ConsumeError::Expired);
    }
    // 闸三：原子一次性（重放拒绝；SQL 行数判定，§3.1）
    if !store.consume_pairing(&row.token_hash, now_ms).map_err(|e| ConsumeError::Storage(e.to_string()))? {
        return Err(ConsumeError::AlreadyConsumed);
    }
    // 闸四：设备数上限（§3.1 默认 10；拒绝不返还配对凭证——一次性语义优先）
    if store.active_device_count().map_err(|e| ConsumeError::Storage(e.to_string()))? >= DEVICE_LIMIT {
        return Err(ConsumeError::DeviceLimit { limit: DEVICE_LIMIT });
    }
    // 签发会话令牌+登记设备+惰性会话映射（§2.5/§3.1）
    let session_plaintext = new_session_plaintext();
    let device_id = format!("dev-{}", &hash_plaintext(&session_plaintext)[..12]);
    let scope_csv = "chat,panel:read,panel:write".to_string(); // v1 全 scope 签发（§7.2 冻结集全集；scope 收敛随面板票）
    store
        .insert_token(&TokenRow {
            token_hash: hash_plaintext(&session_plaintext),
            device_id: device_id.clone(),
            device_name: device_name.to_string(),
            scope_csv: scope_csv.clone(),
            generation: 1,
            created_at_ms: now_ms,
            last_active_ms: now_ms,
            revoked_at_ms: None,
        })
        .map_err(|err| ConsumeError::Storage(err.to_string()))?;
    let new_session_id = format!("sess-{}", &hash_plaintext(&device_id)[..16]);
    let session_id = store
        .upsert_session_map(&device_id, &new_session_id)
        .map_err(|err| ConsumeError::Storage(err.to_string()))?;
    Ok(SessionIssued {
        device_id,
        device_name: device_name.to_string(),
        session_token: session_plaintext,
        session_id,
        scope_csv,
    })
}

/// 消费产出（App 侧保存会话令牌；明文只此一次）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIssued {
    /// 设备 id（配对时铸造）。
    pub device_id: String,
    /// 设备名（回显）。
    pub device_name: String,
    /// 会话令牌明文（dsh_sess_ 前缀；仅此一次可见）。
    pub session_token: String,
    /// 会话 id（§2.5 device↔session 映射）。
    pub session_id: String,
    /// scope 逗号分隔（§7.2）。
    pub scope_csv: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> GatewayStore {
        GatewayStore::open_in_memory().unwrap()
    }

    /// 扫码全链：issue→QR 载荷带凭证与时效→consume→会话令牌签发+映射落库。
    #[test]
    fn 扫码配对全链() {
        let store = store();
        let issued = issue(&store, PairingMethod::Qr, NetworkDimension::Lan, "ws://192.168.1.5:7900", 1_000)
            .unwrap();
        assert!(issued.credential.starts_with("dsh_pair_"));
        assert_eq!(issued.expires_at_ms, 1_000 + QR_TTL_MS);
        let payload = issued.qr_payload.clone().unwrap();
        assert!(payload.contains("\"kind\":\"yunyou-gateway\""));
        assert!(payload.contains("ws://192.168.1.5:7900"));
        assert!(payload.contains(&issued.credential), "QR 载荷应携带配对令牌");
        let consumed = consume(&store, &issued.credential, "业主手机", 2_000).unwrap();
        assert!(consumed.session_token.starts_with("dsh_sess_"));
        assert!(!consumed.session_id.is_empty());
        // 重放：原子一次性（§3.1）
        assert_eq!(consume(&store, &issued.credential, "重放者", 3_000), Err(ConsumeError::AlreadyConsumed));
    }

    /// 验证码全链：短码形态（8 位、字符集无 0O1lI）→按码消费→签发。
    #[test]
    fn 验证码配对全链() {
        let store = store();
        let issued = issue(&store, PairingMethod::DeviceCode, NetworkDimension::Lan, "wss://gw.example.com", 1_000)
            .unwrap();
        assert!(issued.qr_payload.is_none(), "验证码无 QR 载荷");
        assert_eq!(issued.credential.len(), SHORT_CODE_LEN, "8 位短码: {}", issued.credential);
        assert!(!issued.credential.contains('0') && !issued.credential.contains('O'));
        assert!(!issued.credential.contains('1') && !issued.credential.contains('l') && !issued.credential.contains('I'));
        assert_eq!(issued.expires_at_ms, 1_000 + DEVICE_CODE_TTL_MS);
        let consumed = consume(&store, &issued.credential, "平板", 2_000).unwrap();
        assert_eq!(consumed.device_name, "平板");
    }

    /// 过期与失败锁定（§3.2：5 次失败→锁 15 分钟；锁定期内拒绝优先）。
    /// 时间线：issue@1s（QR TTL 5min→expires@301s）→ 400s 起逐次过期消费
    /// （每次 fail+1），第 5 次失败触发锁定（截止=第 5 次时刻+15min）。
    #[test]
    fn 过期失败与锁定() {
        let store = store();
        let issued =
            issue(&store, PairingMethod::Qr, NetworkDimension::Lan, "ws://x", 1_000).unwrap();
        // 400s > 301s=过期点：Expired 且失败计数累计
        for i in 0..FAIL_LIMIT {
            let at = 400_000 + u64::from(i) * 100_000;
            assert_eq!(consume(&store, &issued.credential, "d", at), Err(ConsumeError::Expired));
        }
        // 第 5 次失败（at=800s）触发锁定：截止=800s+15min=1_700s
        let row = store
            .find_pairing_by_token(&hash_plaintext(&issued.credential))
            .unwrap()
            .unwrap();
        assert_eq!(row.fail_count, FAIL_LIMIT);
        assert_eq!(row.locked_until_ms, Some(800_000 + LOCK_MS));
        // 锁定期内：consume 让位于 Locked（爆破面收口——即便此刻已真实过期）
        assert!(matches!(
            consume(&store, &issued.credential, "d", 900_000),
            Err(ConsumeError::Locked { .. })
        ));
    }

    /// 未知凭证=UnknownToken（typo/伪造——auth_failure 留痕由调用方执行）。
    #[test]
    fn 未知凭证拒绝() {
        let store = store();
        assert_eq!(
            consume(&store, "dsh_pair_不存在的凭证", "d", 1_000),
            Err(ConsumeError::UnknownToken)
        );
    }

    /// 设备数上限（§3.1 默认 10；拒绝变体 device_limit_rejected）。
    #[test]
    fn 设备数上限拒绝() {
        let store = store();
        // 直灌 10 台活跃设备（绕过配对，直测上限闸）
        for i in 0..DEVICE_LIMIT {
            store
                .insert_token(&TokenRow {
                    token_hash: format!("sess{i}"),
                    device_id: format!("dev{i}"),
                    device_name: format!("设备{i}"),
                    scope_csv: "chat".into(),
                    generation: 1,
                    created_at_ms: 1,
                    last_active_ms: 1,
                    revoked_at_ms: None,
                })
                .unwrap();
        }
        let issued = issue(&store, PairingMethod::Qr, NetworkDimension::Lan, "ws://x", 1_000).unwrap();
        assert_eq!(
            consume(&store, &issued.credential, "第 11 台", 2_000),
            Err(ConsumeError::DeviceLimit { limit: DEVICE_LIMIT })
        );
    }
}
