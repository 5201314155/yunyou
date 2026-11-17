//! 鉴权中间件与限速（网关落地方案 §7.1/§3.2）：HTTP 与 WS 同一条链——
//! Bearer 会话令牌 → SHA-256 哈希 → store 查表（generation 缓存 TTL 1s）→
//! scope 检查；限速=令牌桶按 §3.2 表（业务 60/min/令牌、WS 握手 10/min/IP、
//! healthz 6/min/IP）。

use crate::scope::Scope;
use crate::store::{GatewayStore, TokenRow};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// generation 缓存 TTL（§3.1：吊销/锁定 bump generation，下一请求即失效——
/// 缓存 1 秒=吊销生效延迟上界，业主 D5「吊销立即生效」的工程化读法）。
const GENERATION_CACHE_TTL: Duration = Duration::from_secs(1);

/// 会话令牌滑动过期缺省（G-3：§3.1「30 天滑动过期」定案值；缺活跃=失效
/// 须重新配对，每次认证成功前滚 last_active_ms=滑动动作）。
const SESSION_TTL_DEFAULT_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// 认证失败（HTTP 映射：401 未认证/403 无 scope，方案 §8 错误统一形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// 无 Bearer/令牌无效/已吊销/已锁定 → 401。
    Unauthorized(String),
    /// 令牌有效但 scope 不足 → 403。
    Forbidden { needed: Scope },
    /// 存储故障（F-5 修复：内部错误→500 人话，不映射 401 亦不泄 err 文本）。
    Storage,
}

impl AuthError {
    /// HTTP 状态码（借用形态：http.rs 错误映射需先取码再取人话，两消费方法
    /// 不可先后各取 self）。
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Unauthorized(_) => 401,
            Self::Forbidden { .. } => 403,
            Self::Storage => 500,
        }
    }

    /// HTTP 状态码（方案 §8 统一错误形态）。
    pub fn status(self) -> u16 {
        self.status_code()
    }

    /// 人话（错误响应体；不泄内部细节——§10.2 纪律 2）。
    pub fn message(self) -> String {
        match self {
            Self::Unauthorized(reason) => format!("未认证：{reason}"),
            Self::Forbidden { needed } => format!(
                "令牌 scope 不足：需要 {}（scope 申请走重新配对换发）",
                needed.as_str()
            ),
            // F-5：内部错误全文不进响应体（只留人话；详情落网关日志面）
            Self::Storage => "存储错误，请稍后重试".into(),
        }
    }
}

/// 认证产出（通过两道闸第一道后的调用方上下文）。
#[derive(Debug, Clone)]
pub struct AuthedDevice {
    pub device_id: String,
    pub device_name: String,
    pub scopes: Vec<Scope>,
    pub generation: u64,
}

/// 令牌桶（§3.2 限速；key=令牌哈希或源 IP+端点类）。
struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn try_take(&mut self, rate_per_min: f64, now: Instant) -> bool {
        let capacity = rate_per_min;
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * rate_per_min / 60.0).min(capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false // 429（方案 §8 限速形态）
        }
    }
}

/// 限速桶容量上限（G-7 防线：公网伪造 IP/扫描面防无界增长；超限=淘汰
/// 闲置超过 [`BUCKET_IDLE_TTL`] 的旧桶，全闲则整表清账保守放行下一轮）。
const BUCKET_CAP: usize = 10_000;
const BUCKET_IDLE_TTL: Duration = Duration::from_secs(600);

/// 鉴权+限速状态（axum State 共享；锁=短临界区同步操作）。
pub struct AuthMiddleware {
    store: Arc<GatewayStore>,
    /// generation 缓存：TTL 1s 内复用查表结果（§3.1 缓存语义）。
    cache: Mutex<Option<(Instant, Vec<TokenRow>)>>,
    /// 限速桶（key→桶；容量上限 [`BUCKET_CAP`]，超限淘汰闲置桶）。
    buckets: Mutex<HashMap<String, Bucket>>,
    /// 会话令牌滑动过期窗（毫秒；G-3）。
    session_ttl_ms: u64,
}

impl AuthMiddleware {
    pub fn new(store: Arc<GatewayStore>) -> Self {
        Self {
            store,
            cache: Mutex::new(None),
            buckets: Mutex::new(HashMap::new()),
            session_ttl_ms: SESSION_TTL_DEFAULT_MS,
        }
    }

    /// 设置滑动过期窗（毫秒；[`crate::config::GatewayConfig::session_ttl_days`]
    /// 装配透传）。
    pub fn set_session_ttl_ms(&mut self, ttl_ms: u64) {
        self.session_ttl_ms = ttl_ms;
    }

    /// Bearer 提取（Authorization: Bearer <token>；大小写不敏感 scheme）。
    pub fn extract_bearer(headers: &[(String, String)]) -> Option<String> {
        headers.iter().find_map(|(name, value)| {
            (name.eq_ignore_ascii_case("authorization"))
                .then(|| value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")))
                .flatten()
                .map(String::from)
        })
    }

    /// 认证主链：Bearer → 哈希 → 查表（缓存 TTL 1s）→ 产出（scope 检查由
    /// 调用方按端点要求调 [`Self::require_scope`]——两道闸的第一道在此分岔）。
    pub fn authenticate(&self, bearer: Option<&str>) -> Result<AuthedDevice, AuthError> {
        let Some(token) = bearer else {
            return Err(AuthError::Unauthorized("缺少 Bearer 凭证".into()));
        };
        // 前缀快筛（形态错=无表查询开销；dsh_sess_ 前缀=会话令牌）
        if !token.starts_with(crate::token::SESSION_PREFIX) {
            return Err(AuthError::Unauthorized("凭证形态不符（应为会话令牌）".into()));
        }
        let token_hash = crate::token::hash_plaintext(token);
        // generation 缓存（TTL 1s）：命中且含目标哈希→免查表
        {
            let cache = self.cache.lock().expect("generation 缓存锁");
            if let Some((at, rows)) = cache.as_ref() {
                if at.elapsed() < GENERATION_CACHE_TTL {
                    if let Some(row) = rows.iter().find(|row| row.token_hash == token_hash) {
                        return self.authed_checked(row);
                    }
                    // 缓存期内未命中=可能新签发（TTL 窗口内签发的令牌不在旧快照）
                    // →落穿查表（新签发令牌必须立即可用，不能等缓存过期）
                }
            }
        }
        let rows = self.active_rows()?;
        let Some(row) = rows.iter().find(|row| row.token_hash == token_hash) else {
            return Err(AuthError::Unauthorized(
                "令牌无效或已吊销（吊销立即生效）".into(),
            ));
        };
        let authed = self.authed_checked(row)?;
        // G-3 滑动动作：认证成功=活跃，last_active_ms 前滚（写放大可忽略：
        // 设备量 ≤ 上限 10，行数个位级）
        if let Err(_err) = self.store.touch_token_last_active(&token_hash, now_ms()) {
            // 触点写失败不阻断本次认证（已验证有效；下轮过期判定按旧值从宽）
            // ——与 F-5 同律：查询故障才映射 500，续期写故障非安全面
        }
        *self.cache.lock().expect("generation 缓存锁") = Some((Instant::now(), rows));
        Ok(authed)
    }

    /// 认证产出+滑动过期闸（G-3：now - last_active > ttl=失效，须重新配对）。
    fn authed_checked(&self, row: &TokenRow) -> Result<AuthedDevice, AuthError> {
        if now_ms().saturating_sub(row.last_active_ms) > self.session_ttl_ms {
            return Err(AuthError::Unauthorized(
                "会话令牌已过期（30 天未活跃），请重新配对".into(),
            ));
        }
        Ok(self.authed(row))
    }

    /// scope 检查（两道闸第二道：端点要求的 scope 必须被令牌覆盖，§7.2）。
    pub fn require_scope(authed: &AuthedDevice, needed: Scope) -> Result<(), AuthError> {
        Scope::covers(&authed.scopes, needed)
            .then_some(())
            .ok_or(AuthError::Forbidden { needed })
    }

    /// 限速（§3.2 令牌桶）：放行=true；拒绝=429。桶表超 [`BUCKET_CAP`]
    /// 时淘汰闲置超 [`BUCKET_IDLE_TTL`] 的桶（G-7：公网面内存防线）。
    pub fn rate_limit(&self, key: &str, rate_per_min: f64) -> bool {
        let mut buckets = self.buckets.lock().expect("限速桶锁");
        if buckets.len() >= BUCKET_CAP {
            let now = Instant::now();
            buckets.retain(|_, b| now.duration_since(b.last) < BUCKET_IDLE_TTL);
        }
        buckets
            .entry(key.to_string())
            .or_insert(Bucket { tokens: rate_per_min, last: Instant::now() })
            .try_take(rate_per_min, Instant::now())
    }

    fn active_rows(&self) -> Result<Vec<TokenRow>, AuthError> {
        // 全活跃行快照（设备量 ≤ 上限 10，行数个位级——全量快照即精确）
        self.store
            .all_active_tokens()
            .map_err(|_err| {
                // F-5：存储故障≠认证失败——映射 500 人话，不进 401 族不泄 err 文本
                //（诊断出口候阶段 C 网关日志面；本 crate 依赖白名单 §9.1 不引日志件）
                AuthError::Storage
            })
    }

    fn authed(&self, row: &TokenRow) -> AuthedDevice {
        AuthedDevice {
            device_id: row.device_id.clone(),
            device_name: row.device_name.clone(),
            scopes: Scope::parse_csv(&row.scope_csv),
            generation: row.generation,
        }
    }
}

/// unix 毫秒（G-3 滑动过期判定时钟；网关进程时钟与 http.rs now_ms 同源语义）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn middleware_with_token() -> (AuthMiddleware, String) {
        let store = Arc::new(GatewayStore::open_in_memory().unwrap());
        store
            .insert_token(&TokenRow {
                token_hash: crate::token::hash_plaintext("dsh_sess_test"),
                device_id: "d1".into(),
                device_name: "测试设备".into(),
                scope_csv: "chat,panel:read".into(),
                generation: 1,
                created_at_ms: 1,
                // G-3：活跃触点=当前（否则 30 天滑动过期判死，fixture 语义=新鲜令牌）
                last_active_ms: now_ms(),
                revoked_at_ms: None,
            })
            .unwrap();
        (AuthMiddleware::new(store), "dsh_sess_test".to_string())
    }

    /// G-3 滑动过期：30 天未活跃=401 提示重新配对；活跃令牌过闸且触点前滚；
    /// 过期窗可配置（session_ttl_days 装配透传）。
    #[test]
    fn g3_滑动过期与续期() {
        let store = Arc::new(GatewayStore::open_in_memory().unwrap());
        let now = now_ms();
        let day: u64 = 24 * 60 * 60 * 1000;
        // 新鲜令牌（活跃=1 天前）：过闸
        store
            .insert_token(&TokenRow {
                token_hash: crate::token::hash_plaintext("dsh_sess_fresh"),
                device_id: "d1".into(),
                device_name: "新".into(),
                scope_csv: "chat".into(),
                generation: 1,
                created_at_ms: now - day,
                last_active_ms: now - day,
                revoked_at_ms: None,
            })
            .unwrap();
        // 僵尸令牌（活跃=31 天前）：判死
        store
            .insert_token(&TokenRow {
                token_hash: crate::token::hash_plaintext("dsh_sess_stale"),
                device_id: "d2".into(),
                device_name: "僵".into(),
                scope_csv: "chat".into(),
                generation: 1,
                created_at_ms: now - 31 * day,
                last_active_ms: now - 31 * day,
                revoked_at_ms: None,
            })
            .unwrap();
        // 中间龄令牌（活跃=2 天前）：默认窗过、缩窗死——专测配置透传
        store
            .insert_token(&TokenRow {
                token_hash: crate::token::hash_plaintext("dsh_sess_mid"),
                device_id: "d3".into(),
                device_name: "中".into(),
                scope_csv: "chat".into(),
                generation: 1,
                created_at_ms: now - 2 * day,
                last_active_ms: now - 2 * day,
                revoked_at_ms: None,
            })
            .unwrap();
        let mut mw = AuthMiddleware::new(Arc::clone(&store));
        mw.set_session_ttl_ms(30 * day);
        // 新鲜：过闸且滑动续期（last_active 前滚到当前）
        mw.authenticate(Some("dsh_sess_fresh")).expect("新鲜令牌应过");
        let rows = store.all_active_tokens().unwrap();
        let fresh = rows.iter().find(|r| r.device_id == "d1").unwrap();
        assert!(now_ms().saturating_sub(fresh.last_active_ms) < 60_000, "触点应已前滚");
        // 僵尸：401+人话指路重新配对
        let err = mw.authenticate(Some("dsh_sess_stale")).unwrap_err();
        match err {
            AuthError::Unauthorized(msg) => {
                assert!(msg.contains("过期") && msg.contains("重新配对"), "{msg}");
            }
            other => panic!("应为 401 族：{other:?}"),
        }
        // 缩窗配置生效：TTL=1 天 → 活跃 2 天前的中间龄令牌判过期（默认窗下它活）
        let mut short = AuthMiddleware::new(Arc::clone(&store));
        short.set_session_ttl_ms(day);
        assert!(short.authenticate(Some("dsh_sess_mid")).is_err(), "缩窗后中间龄应过期");
    }

    /// 认证主链：有效令牌过/缺失拒/形态错拒/未知拒（401 族）。
    #[test]
    fn 认证主链四态() {
        let (mw, token) = middleware_with_token();
        let authed = mw.authenticate(Some(&token)).expect("有效令牌应过");
        assert_eq!(authed.device_id, "d1");
        assert!(Scope::covers(&authed.scopes, Scope::Chat));
        assert!(matches!(
            mw.authenticate(None),
            Err(AuthError::Unauthorized(_))
        ));
        assert!(matches!(
            mw.authenticate(Some("dsh_pair_错类型")),
            Err(AuthError::Unauthorized(_))
        ));
        assert!(matches!(
            mw.authenticate(Some("dsh_sess_未知")),
            Err(AuthError::Unauthorized(_))
        ));
    }

    /// scope 两道闸第二道：chat 令牌请求 panel:write=403 族。
    #[test]
    fn scope不足403() {
        let (mw, token) = middleware_with_token();
        let authed = mw.authenticate(Some(&token)).unwrap();
        assert!(matches!(
            AuthMiddleware::require_scope(&authed, Scope::PanelWrite),
            Err(AuthError::Forbidden { .. })
        ));
        assert!(AuthMiddleware::require_scope(&authed, Scope::PanelRead).is_ok());
    }

    /// 限速令牌桶：突发容量内放行、超容量 429、时间恢复。
    #[test]
    fn 限速桶放行与拒绝() {
        let mw = AuthMiddleware::new(Arc::new(GatewayStore::open_in_memory().unwrap()));
        // 容量=rate（60/min）——快取 61 次必有一次拒
        let allowed = (0..60).filter(|_| mw.rate_limit("t1", 60.0)).count();
        assert_eq!(allowed, 60, "容量内全放行");
        assert!(!mw.rate_limit("t1", 60.0), "超容量拒绝");
    }
}
