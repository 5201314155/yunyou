//! 网关防线接线测试（G-1 限速/G-2 IP 锁/F-7 爆破锁定/F-5 存储错误映射/F-3 审计留痕）。
#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试断言惯例（同包既有测试同律）
//!
//! 对端地址经 `ConnectInfo` 扩展注入（axum 请求扩展=make service 注入位，
//! oneshot 测试等效）；审计用内存档 snapshot 回读断言。

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;
use yunyou_gateway::{
    audit::{AuditEvent, AuditLog},
    config::GatewayConfig,
    http::{self, GatewayState},
    GatewayStore,
};

/// 装配（可调配置；审计句柄外提供断言）。
fn assemble(config: &GatewayConfig) -> (axum::Router, Arc<AuditLog>) {
    let audit = Arc::new(AuditLog::in_memory());
    let state = GatewayState::new(Arc::new(GatewayStore::open_in_memory().unwrap()), config, Arc::clone(&audit));
    (http::router(state), audit)
}

/// 发请求（方法/路径/body/对端/XFF）→ (status, body)。
async fn req(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<String>,
    peer: Option<SocketAddr>,
    xff: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(addr) = peer {
        builder = builder.extension(ConnectInfo(addr));
    }
    if let Some(ip) = xff {
        builder = builder.header("x-forwarded-for", ip);
    }
    let request = builder
        .header("content-type", "application/json")
        .body(Body::from(body.unwrap_or_default()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn peer(last_octet: u8) -> Option<SocketAddr> {
    Some(SocketAddr::from(([10, 0, 0, last_octet], 1000)))
}

// ── G-1：限速接线 ──────────────────────────────────────────────

/// healthz 限速生效：默认 30/分/IP，第 31 次=429。
#[tokio::test(flavor = "multi_thread")]
async fn g1_healthz超限429() {
    let (app, _audit) = assemble(&GatewayConfig::default());
    for _ in 0..30 {
        let (status, _) = req(&app, "GET", "/healthz", None, peer(10), None).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, body) = req(&app, "GET", "/healthz", None, peer(10), None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body.contains("频繁"), "{body}");
}

/// lead 追加件回归锁：两个不同对端地址互不挤桶（A 触顶后 B 仍放行）。
#[tokio::test(flavor = "multi_thread")]
async fn g1_不同对端互不挤桶() {
    let (app, _audit) = assemble(&GatewayConfig::default());
    for _ in 0..30 {
        let (status, _) = req(&app, "GET", "/healthz", None, peer(1), None).await;
        assert_eq!(status, StatusCode::OK);
    }
    // A（10.0.0.1）已触顶；B（10.0.0.2=不同对端）首次请求必须放行
    let (status, body) = req(&app, "GET", "/healthz", None, peer(2), None).await;
    assert_eq!(status, StatusCode::OK, "B 不得被 A 的桶挤爆：{body}");
}

/// trust_proxy 两态：关=伪造 X-Forwarded-For 不采信（共享对端桶会触顶）；
/// 开=按 XFF 首跳分桶（不同 XFF 互不影响）。
#[tokio::test(flavor = "multi_thread")]
async fn g1_trust_proxy开关两态() {
    // 关（默认）：无对端地址（内嵌/反代未传）→回退 local 共桶；伪造 XFF 不生效
    let (app, _audit) = assemble(&GatewayConfig::default());
    for _ in 0..30 {
        let (status, _) =
            req(&app, "GET", "/healthz", None, None, Some("9.9.9.9")).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = req(&app, "GET", "/healthz", None, None, Some("9.9.9.9")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "关档伪造 XFF 不得分桶");
    // 开：不同 XFF=不同桶，各自放行
    let config = GatewayConfig { trust_proxy: true, ..GatewayConfig::default() };
    let (app, _audit) = assemble(&config);
    let (a, _) = req(&app, "GET", "/healthz", None, None, Some("9.9.9.9")).await;
    let (b, _) = req(&app, "GET", "/healthz", None, None, Some("8.8.8.8")).await;
    assert_eq!(a, StatusCode::OK);
    assert_eq!(b, StatusCode::OK);
}

// ── G-2 + F-7：IP 级失败锁定端到端（连续错配对 N 次→被锁） ──────

/// 连续错凭证爆破：默认 fail_limit=5，第 5 次失败即触发锁定→后续 429；
/// 另一对端不受牵连；锁定解除（伪造未来窗口）语义由 store 单测锁。
#[tokio::test(flavor = "multi_thread")]
async fn f7_连续错配对触发ip锁定() {
    let (app, audit) = assemble(&GatewayConfig::default());
    let body = r#"{"code":"wrong-code","device_name":"爆破机"}"#;
    for i in 0..4 {
        let (status, _) = req(&app, "POST", "/pair/device-code", Some(body.to_string()), peer(3), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "第 {} 次", i + 1);
    }
    // 第 5 次失败即触发锁定（fail_limit=5）：该次直接 429（锁定生效）
    let (status, resp_body) =
        req(&app, "POST", "/pair/device-code", Some(body.to_string()), peer(3), None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{resp_body}");
    // 锁定后继续打：仍 429（锁定中）
    let (status, _) =
        req(&app, "POST", "/pair/device-code", Some(body.to_string()), peer(3), None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // 另一对端不受牵连（仍是 401 非锁定 429）
    let (status, _) = req(&app, "POST", "/pair/device-code", Some(body.to_string()), peer(4), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // F-3 断言：配对被拒留痕 ≥5 + lock_engaged 留痕 ≥1
    let snapshot = String::from_utf8(audit.snapshot().expect("内存档")).unwrap();
    assert!(snapshot.contains("\"event\":\"pair_rejected\""), "{snapshot}");
    assert!(snapshot.contains("\"event\":\"lock_engaged\""), "{snapshot}");
    assert!(AuditEvent::LockEngaged.as_str() == "lock_engaged");
}

// ── F-3：审计留痕（鉴权失败必留痕） ────────────────────────────

/// 无令牌打业务端点：401+auth_failure 审计留痕（写入失败=500 拒绝服务语义
/// 由内存档恒成功，失败分支在 audit.rs 单测锁「不可静默失败」）。
#[tokio::test(flavor = "multi_thread")]
async fn f3_鉴权失败审计留痕() {
    let (app, audit) = assemble(&GatewayConfig::default());
    let (status, _) = req(&app, "GET", "/devices", None, peer(50), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let snapshot = String::from_utf8(audit.snapshot().unwrap()).unwrap();
    assert!(snapshot.contains("\"event\":\"auth_failure\""), "{snapshot}");
    // 留痕含对端 IP（非 local 兜底）
    assert!(snapshot.contains("10.0.0.50"), "{snapshot}");
}

// ── F-5：存储错误映射（500 人话，不泄内部文本） ─────────────────

/// AuthError::Storage：状态 500+人话不含内部 err 文本（middleware 存储故障
/// 不再映射 401——修前 `令牌表不可用：{err}` 直出）。
#[test]
fn f5_存储错误映射500人话() {
    use yunyou_gateway::AuthError;
    let err = AuthError::Storage;
    assert_eq!(err.status_code(), 500);
    let msg = err.message();
    assert!(msg.contains("存储错误"), "{msg}");
    assert!(!msg.contains("rusqlite"), "内部细节不得进响应体：{msg}");
    assert!(!msg.contains("Unauthorized"), "{msg}");
}
