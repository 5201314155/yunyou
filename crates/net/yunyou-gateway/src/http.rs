//! HTTP 端点（网关落地方案 §8 九端点骨架；阶段 B 范围——axum 0.7）。
//!
//! 鉴权：Bearer 会话令牌经 [`middleware::AuthMiddleware`]（§7.1 HTTP/WS 同一
//! 链）；scope 两道闸第二道按端点要求（§2.4 暴露面 ⊆ App scope）。错误统一
//! 形态（§8）：401/403/409/429+锁定码，人话 body 不泄内部细节。

use crate::audit::{AuditEvent, AuditLog};
use crate::config::{GatewayConfig, IpLockConfig, RateLimitConfig};
use crate::middleware::{AuthError, AuthMiddleware, AuthedDevice};
use crate::pairing::{self, PairingMethod};
use crate::scope::Scope;
use crate::store::GatewayStore;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// 网关应用状态（axum State；组件句柄集中）。
#[derive(Clone)]
pub struct GatewayState {
    pub store: Arc<GatewayStore>,
    pub auth: Arc<AuthMiddleware>,
    /// 配对凭证 TTL 由 issue 侧按方式定（阶段 A），端点只透传。
    pub pairing_modes: Vec<PairingMethod>,
    /// 网络维度 endpoint 前缀（QR 载荷组装用；阶段 B 网络层填真值）。
    pub endpoint_prefix: &'static str,
    /// 可信反代开关（false=X-Forwarded-For 不采信，源 IP=socket 对端）。
    pub trust_proxy: bool,
    /// 限速阈值（§3.2 三档；G-1 接线消费）。
    pub rate: RateLimitConfig,
    /// IP 级失败锁定（§3.2 源 IP 维；G-2/F-7 接线消费）。
    pub ip_lock: IpLockConfig,
    /// 审计句柄（§3.3 必留痕；F-3 接线消费）。
    pub audit: Arc<AuditLog>,
}

impl GatewayState {
    /// 统一装配（store+config+audit→中间件/阈值/审计句柄齐备）。
    pub fn new(store: Arc<GatewayStore>, config: &GatewayConfig, audit: Arc<AuditLog>) -> Self {
        let mut auth = AuthMiddleware::new(Arc::clone(&store));
        // G-3：滑动过期窗由配置透传（缺省 30 天）
        auth.set_session_ttl_ms(u64::from(config.session_ttl_days) * 24 * 60 * 60 * 1000);
        Self {
            auth: Arc::new(auth),
            store,
            pairing_modes: vec![PairingMethod::Qr, PairingMethod::DeviceCode],
            endpoint_prefix: config.network.endpoint_prefix(),
            trust_proxy: config.trust_proxy,
            rate: config.rate.clone(),
            ip_lock: config.ip_lock.clone(),
            audit,
        }
    }
}

/// 建路由（九端点骨架，方案 §8；healthz 匿名 D2 定案）。
pub fn router(state: GatewayState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        // /ws/chat 路由在位=OpenAPI 面完整（覆盖锁对齐）；升级编排（IPC 桥真
        // 联调）候 agent 侧 ipc-server 就位——501=「已设计未联调」诚实信号
        .route("/ws/chat", get(ws_chat_not_yet))
        .route("/pair/consume", post(pair_consume))
        .route("/pair/device-code", post(pair_device_code))
        .route("/devices", get(list_devices).delete(revoke_all_devices))
        .route("/devices/:id", axum::routing::delete(revoke_device))
        .route("/panel/tasks", get(panel_tasks))
        .route("/panel/commands", post(panel_commands))
        .with_state(state)
}

// ── 错误响应统一形态（§8：401/403/409/429+人话） ─────────────

fn error_response(status: u16, message: &str) -> (StatusCode, Json<Value>) {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(json!({ "error": message })))
}

fn auth_error_response(err: AuthError) -> (StatusCode, Json<Value>) {
    error_response(err.status_code(), &err.message())
}

/// 源 IP 提取（限速/锁定 key 单源）：缺省取 socket 对端地址（每连接独立桶，
/// 直连客户端互不挤桶——lead 真机追加件）；仅 trust_proxy=true（公网反代
/// 部署显式配置）才采信 X-Forwarded-For 首跳/X-Real-IP——防伪造头绕限速。
/// 对端地址不可得（内嵌测试等）=「local」兜底。
fn client_ip(
    trust_proxy: bool,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
) -> String {
    if trust_proxy {
        for key in ["x-forwarded-for", "x-real-ip"] {
            if let Some(value) = headers.get(key).and_then(|v| v.to_str().ok()) {
                let first = value.split(',').next().unwrap_or("").trim();
                if !first.is_empty() {
                    return first.to_string();
                }
            }
        }
    }
    if let Some(addr) = peer {
        return addr.ip().to_string();
    }
    "local".to_string()
}

fn header_list(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string()))
        .collect()
}

/// Bearer 提取+认证+业务限速（端点共用入口；§7.1 同一链）。G-1 接线：
/// 业务端点按 §3.2 档 60/分/令牌限速（key=bearer 串）；认证失败必留痕
/// （F-3/§3.3：审计写入失败=拒绝业务继续）。
fn authenticate(
    state: &GatewayState,
    headers: &HeaderMap,
    ip: &str,
) -> Result<AuthedDevice, (StatusCode, Json<Value>)> {
    let list = header_list(headers);
    let bearer = AuthMiddleware::extract_bearer(&list);
    if !state.auth.rate_limit(
        &format!("biz:{}", bearer.as_deref().unwrap_or("anon")),
        state.rate.business_per_min,
    ) {
        return Err(error_response(429, "请求过于频繁，请稍后再试"));
    }
    match state
        .auth
        .authenticate(bearer.as_deref())
        .map_err(auth_error_response)
    {
        Ok(authed) => Ok(authed),
        Err(resp) => {
            if state
                .audit
                .log(AuditEvent::AuthFailure, now_ms(), ip, None, "会话令牌认证失败")
                .is_err()
            {
                return Err(error_response(500, "审计不可用，拒绝服务"));
            }
            Err(resp)
        }
    }
}

// ── 探活（匿名 D2 定案：零信息、限速 6/分/IP——G-1 接线生效） ──

async fn healthz(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
) -> Result<&'static str, (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, &headers, peer.map(|c| c.0));
    if !state.auth.rate_limit(&format!("ip:{ip}"), state.rate.healthz_per_min) {
        return Err(error_response(429, "请求过于频繁，请稍后再试"));
    }
    Ok("ok")
}

/// /ws/chat 升级编排 stub（阶段 C 真机联调件）：路由在位=文档面完整，
/// 501=未联调（agent 侧 ipc-server 就位后换 ws.rs 桥接实现）。限速走
/// §3.2 握手档 10/分/IP（G-1 接线）。
async fn ws_chat_not_yet(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, &headers, peer.map(|c| c.0));
    if !state.auth.rate_limit(&format!("ip:{ip}"), state.rate.ws_handshake_per_min) {
        return Err(error_response(429, "请求过于频繁，请稍后再试"));
    }
    Ok(StatusCode::NOT_IMPLEMENTED)
}

// ── 配对（§4；Bearer=配对凭证本身或短码 body；G-2/F-7：源 IP 级失败锁定） ──

/// 配对前置闸（G-1/G-2）：IP 锁定中→429；配对尝试限速（§3.2 握手档 10/分/IP）。
fn pair_gate(
    state: &GatewayState,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
) -> Result<String, (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, headers, peer);
    let now = now_ms();
    if state
        .store
        .ip_lock_state(&format!("ip:{ip}"), now)
        .map_err(|_| error_response(500, "存储错误"))?
        .is_locked(now)
    {
        return Err(error_response(429, "失败锁定中，请稍后再试"));
    }
    if !state.auth.rate_limit(&format!("pair:{ip}"), state.rate.ws_handshake_per_min) {
        return Err(error_response(429, "请求过于频繁，请稍后再试"));
    }
    Ok(ip)
}

/// 配对失败统一出口：审计留痕（F-3）+ 认证类失败计源 IP 锁（G-2/F-7）。
/// 爆破面=UnknownToken/Expired（错凭证/过期凭证反复试）；AlreadyConsumed/
/// DeviceLimit/Locked 非爆破面不计。
fn pair_error_response(
    state: &GatewayState,
    ip: &str,
    err: pairing::ConsumeError,
) -> (StatusCode, Json<Value>) {
    let (code, reason, counts) = match &err {
        pairing::ConsumeError::AlreadyConsumed => (409u16, "已被使用", false),
        pairing::ConsumeError::Expired => (401, "已过期", true),
        pairing::ConsumeError::Locked { .. } => (429, "失败锁定中", false),
        pairing::ConsumeError::DeviceLimit { .. } => (409, "设备数上限", false),
        pairing::ConsumeError::UnknownToken => (401, "凭证无效", true),
        pairing::ConsumeError::Storage(_) => (500, "存储错误", false),
    };
    if state
        .audit
        .log(AuditEvent::PairRejected, now_ms(), ip, None, reason)
        .is_err()
    {
        return error_response(500, "审计不可用，拒绝服务");
    }
    if counts {
        let cfg = &state.ip_lock;
        match state.store.bump_ip_lock(
            &format!("ip:{ip}"),
            now_ms(),
            cfg.window_ms,
            cfg.fail_limit,
            cfg.lock_ms,
        ) {
            Ok(locked) if locked.locked_until_ms.is_some() => {
                // 锁定生效必留痕（§3.3 lock_engaged；审计失败仅留痕降级不阻断 429）
                let _ = state.audit.log(
                    AuditEvent::LockEngaged,
                    now_ms(),
                    ip,
                    None,
                    "源 IP 连续失败触发锁定",
                );
                return error_response(429, "失败锁定中，请稍后再试");
            }
            Ok(_) => {}
            Err(_) => return error_response(500, "存储错误"),
        }
    }
    error_response(code, reason)
}

/// 配对成功出口：审计留痕（F-3；写入失败=拒绝放行令牌，§3.3 必留痕）。
fn pair_success_response(
    state: &GatewayState,
    ip: &str,
    issued: &pairing::SessionIssued,
) -> (StatusCode, Json<Value>) {
    if state
        .audit
        .log(
            AuditEvent::PairConsumed,
            now_ms(),
            ip,
            Some(&issued.device_id),
            "配对消费成功，签发会话令牌",
        )
        .is_err()
    {
        return error_response(500, "审计不可用，拒绝服务");
    }
    (
        StatusCode::OK,
        Json(json!({
            "session_token": issued.session_token,
            "session_id": issued.session_id,
            "device_id": issued.device_id,
            "scope": issued.scope_csv,
        })),
    )
}

/// POST /pair/consume（扫码消费）：Bearer=配对令牌，body={device_name}。
async fn pair_consume(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let bearer = AuthMiddleware::extract_bearer(&header_list(&headers));
    let Some(pairing_credential) = bearer else {
        return error_response(401, "缺少 Bearer 配对令牌");
    };
    let Some(device_name) = body.get("device_name").and_then(Value::as_str) else {
        return error_response(400, "缺少 device_name");
    };
    let ip = match pair_gate(&state, &headers, peer.map(|c| c.0)) {
        Ok(ip) => ip,
        Err(resp) => return resp,
    };
    // 网络/endpoint 形态：阶段 A 已组装（QR 载荷在 issue 侧），此处消费
    match pairing::consume(&state.store, &pairing_credential, device_name, now_ms()) {
        Ok(issued) => pair_success_response(&state, &ip, &issued),
        Err(err) => pair_error_response(&state, &ip, err),
    }
}

/// POST /pair/device-code（验证码消费）：body={code, device_name}。
async fn pair_device_code(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(code) = body.get("code").and_then(Value::as_str) else {
        return error_response(400, "缺少 code");
    };
    let Some(device_name) = body.get("device_name").and_then(Value::as_str) else {
        return error_response(400, "缺少 device_name");
    };
    let ip = match pair_gate(&state, &headers, peer.map(|c| c.0)) {
        Ok(ip) => ip,
        Err(resp) => return resp,
    };
    match pairing::consume(&state.store, code, device_name, now_ms()) {
        Ok(issued) => pair_success_response(&state, &ip, &issued),
        Err(err) => pair_error_response(&state, &ip, err),
    }
}

// ── 设备管理（§3.1：GET 列表/DELETE 吊销；Bearer=会话令牌） ──

async fn list_devices(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, &headers, peer.map(|c| c.0));
    let _authed = authenticate(&state, &headers, &ip)?;
    let devices: Vec<Value> = state
        .store
        .all_active_tokens()
        .map_err(|_| error_response(500, "存储错误，请稍后重试"))?
        .into_iter()
        .map(|row| {
            json!({
                "device_id": row.device_id,
                "device_name": row.device_name,
                "scope": row.scope_csv,
                "created_at_ms": row.created_at_ms,
            })
        })
        .collect();
    Ok(Json(json!({ "devices": devices })))
}

async fn revoke_device(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    Path(device_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, &headers, peer.map(|c| c.0));
    let _authed = authenticate(&state, &headers, &ip)?;
    let revoked = state
        .store
        .revoke_device(&device_id, now_ms())
        .map_err(|_| error_response(500, "存储错误，请稍后重试"))?;
    Ok(Json(json!({ "revoked": revoked })))
}

async fn revoke_all_devices(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, &headers, peer.map(|c| c.0));
    let _authed = authenticate(&state, &headers, &ip)?;
    let revoked = state
        .store
        .revoke_all(now_ms())
        .map_err(|_| error_response(500, "存储错误，请稍后重试"))?;
    Ok(Json(json!({ "revoked": revoked })))
}

// ── 面板两端点（scope=panel:read/panel:write；IPC 桥随 B2 尾段 ws.rs 同批） ──

async fn panel_tasks(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, &headers, peer.map(|c| c.0));
    let authed = authenticate(&state, &headers, &ip)?;
    AuthMiddleware::require_scope(&authed, Scope::PanelRead).map_err(auth_error_response)?;
    // IPC 桥（panel.tasks.read→投影）在 B2 尾段 ws.rs 同批接线：agent 不可达=
    // 诚实空快照（接口面不变，方案 §2.3 投影由 agent 侧生成）
    Ok(Json(json!({ "tasks": [], "note": "投影源未接线（阶段 B 尾段）" })))
}

async fn panel_commands(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let ip = client_ip(state.trust_proxy, &headers, peer.map(|c| c.0));
    let authed = authenticate(&state, &headers, &ip)?;
    AuthMiddleware::require_scope(&authed, Scope::PanelWrite).map_err(auth_error_response)?;
    let Some(command) = body.get("command").and_then(Value::as_str) else {
        return Err(error_response(400, "缺少 command"));
    };
    // IPC 桥（panel.command.submit→入队）在 B2 尾段接线：先入队确认形态
    Ok((
        StatusCode::OK,
        Json(json!({ "queued": true, "command": command })),
    ))
}

/// unix 毫秒（时间注入点：网关进程时钟；纯函数测试在模块内）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
