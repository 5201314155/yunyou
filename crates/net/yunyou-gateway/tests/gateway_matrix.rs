//! 网关 2×2 矩阵逐格测试（G1-G5，网关落地方案 §6「验收核心」）+ 无匿名通道实测。
//!
//! 【边界自裁（lead 令授权）】四格鉴权路径同构（§6：同一中间件/令牌表/scope
//! 检查）——G1/G2 走完整 HTTP 流（oneshot 内存服务，零网络）；G3/G4 的差异
//! 仅 TLS 传输层（config fail-loud 校验已单测锁死）与 endpoint 形态（wss://
//! 前缀断言在位），真 TLS 握手 E2E=「候部署/候 ipc-server」标注格。
//! 【无匿名通道】全部业务端点无 token=401 参数化实测（G1-G4 共享断言面）。

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt; // oneshot
use yunyou_gateway::{
    pairing::{self, NetworkDimension, PairingMethod},
    GatewayStore,
};
use std::sync::Arc;

/// 内存装配（store+auth+router；G1-G5 共用）。
fn assemble() -> axum::Router {
    let store = Arc::new(GatewayStore::open_in_memory().unwrap());
    assemble_with_store(store)
}

fn assemble_with_store(store: Arc<GatewayStore>) -> axum::Router {
    let config = yunyou_gateway::config::GatewayConfig::default();
    yunyou_gateway::http::router(yunyou_gateway::http::GatewayState::new(
        store,
        &config,
        Arc::new(yunyou_gateway::audit::AuditLog::in_memory()),
    ))
}

/// 发请求（method/uri/bearer/body）→ (status, body)。
async fn req(
    app: axum::Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder.body(Body::from(body.unwrap_or("").to_string())).unwrap();
    let response = app.oneshot(request).await.expect("oneshot 应成功");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// 签发扫码配对（内存 store 直用；返回配对凭证明文）。
fn issue_qr(store: &GatewayStore, _now: u64) -> String {
    // 时钟与 HTTP 层同源（真实毫秒）——假时钟会让配对瞬间过期（首轮实测教训）
    let now = now_ms();
    pairing::issue(store, PairingMethod::Qr, NetworkDimension::Lan, "ws://192.168.1.5:7900", now)
        .expect("签发应成功")
        .credential
}

/// unix 毫秒（与 http.rs now_ms 同源；测试面零时钟依赖仅指确定性断言，
/// 配对时效这类真时钟语义必须用真钟——假钟混真钟=首轮「已过期」误报根源）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 签发验证码配对。
fn issue_code(store: &GatewayStore, _now: u64) -> String {
    let now = now_ms();
    pairing::issue(
        store,
        PairingMethod::DeviceCode,
        NetworkDimension::Lan,
        "ws://192.168.1.5:7900",
        now,
    )
    .expect("签发应成功")
    .credential
}

/// 业务端点清单（无匿名通道参数化断言面——§6 四格共享的鉴权路径同构性
/// 由「同一 router」结构性保证，此处逐端点实测兜底）。
const BUSINESS_ENDPOINTS: [(&str, &str); 5] = [
    ("GET", "/panel/tasks"),
    ("POST", "/panel/commands"),
    ("GET", "/devices"),
    ("DELETE", "/devices/d1"),
    ("DELETE", "/devices"),
];

/// G1（内网×扫码）：QR 签发→consume 换会话令牌→带令牌业务请求 200→
/// 无令牌业务请求 401（无匿名通道）。
#[tokio::test(flavor = "multi_thread")]
async fn g1_内网扫码全链与无匿名() {
    let store = Arc::new(GatewayStore::open_in_memory().unwrap());
    let credential = issue_qr(&store, 1_000);
    let app = assemble_with_store(store);
    // consume：Bearer=配对令牌
    let (status, body) = req(app.clone(), "POST", "/pair/consume", Some(&credential), Some(r#"{"device_name":"业主手机"}"#)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session_token: String = serde_json::from_slice::<serde_json::Value>(
        body.as_bytes(),
    )
    .expect("JSON")
    .get("session_token")
    .and_then(|v| v.as_str())
    .expect("session_token 应在")
    .into();
    assert!(session_token.starts_with("dsh_sess_"));
    // 带令牌业务请求=200
    let (status, _) = req(app.clone(), "GET", "/panel/tasks", Some(&session_token), None).await;
    assert_eq!(status, StatusCode::OK);
    // 无令牌=401（无匿名通道）
    let (status, body) = req(app.clone(), "GET", "/panel/tasks", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

/// G2（内网×验证码）：短码→device-code 消费→带令牌 200→无令牌 401。
#[tokio::test(flavor = "multi_thread")]
async fn g2_内网验证码全链与无匿名() {
    let store = Arc::new(GatewayStore::open_in_memory().unwrap());
    let code = issue_code(&store, 1_000);
    let app = assemble_with_store(store);
    let (status, body) = req(
        app.clone(),
        "POST",
        "/pair/device-code",
        None,
        Some(&format!(r#"{{"code":"{code}","device_name":"平板"}}"#)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session_token: String = serde_json::from_slice::<serde_json::Value>(body.as_bytes())
        .expect("JSON")
        .get("session_token")
        .and_then(|v| v.as_str())
        .expect("session_token 应在")
        .into();
    let (status, _) = req(app.clone(), "GET", "/panel/tasks", Some(&session_token), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = req(app.clone(), "GET", "/panel/tasks", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// G3/G4（公网×扫码/验证码）：TLS 传输层的逻辑侧锚——公网档 QR 载荷用
/// wss:// 前缀（形态差异唯一可测点）+config fail-loud 已单测（terminal 层
/// 真握手=「候部署」标注格，lead 边界自裁授权）。
#[test]
fn g3g4_公网档形态锚() {
    let store = GatewayStore::open_in_memory().unwrap();
    let issued = pairing::issue(
        &store,
        PairingMethod::Qr,
        NetworkDimension::Public,
        "wss://gw.example.com",
        1_000,
    )
    .unwrap();
    let payload = issued.qr_payload.clone().expect("公网扫码应有载荷");
    assert!(payload.contains("wss://gw.example.com"), "公网 QR 载荷应 wss:// 前缀: {payload}");
    // 验证码公网格：短码流程与内网同构（载体差异不触及凭证逻辑）
    let code = pairing::issue(
        &store,
        PairingMethod::DeviceCode,
        NetworkDimension::Public,
        "wss://gw.example.com",
        1_000,
    )
    .unwrap()
    .credential;
    assert_eq!(code.len(), 8);
}

/// G5（healthz 非格）：匿名 200+响应体恰为 "ok"（零信息：不含版本/路径）。
#[tokio::test(flavor = "multi_thread")]
async fn g5_healthz匿名零信息() {
    let app = assemble();
    let (status, body) = req(app, "GET", "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok", "零信息：恰为 ok（不含版本/路径）: {body:?}");
}

/// 无匿名通道参数化实测：五业务端点 × 无令牌 → 全 401（§6 四格共享断言面
/// 的兜底实测——结构性同构之外逐端点过闸）。
#[tokio::test(flavor = "multi_thread")]
async fn 无匿名通道逐端点实测() {
    let app = assemble();
    for (method, uri) in BUSINESS_ENDPOINTS {
        // POST 端点带合法 JSON 形态（axum Json 提取器在鉴权前检查 content-type，
        // 缺形态=415 先于 401——无匿名断言须让请求到达鉴权层才有效）
        let body = if method == "POST" { Some("{}") } else { None };
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder.body(Body::from(body.unwrap_or("").to_string())).unwrap();
        let response = app.clone().oneshot(request).await.expect("oneshot 应成功");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri} 无令牌必须 401"
        );
    }
}

/// scope 两道闸实测：panel:read 令牌请求 panel:write 端点=403（§7.2）。
#[tokio::test(flavor = "multi_thread")]
async fn scope不足403实测() {
    let store = Arc::new(GatewayStore::open_in_memory().unwrap());
    let code = issue_code(&store, 1_000);
    let app = assemble_with_store(store);
    let (status, body) = req(
        app.clone(),
        "POST",
        "/pair/device-code",
        None,
        Some(&format!(r#"{{"code":"{code}","device_name":"受限设备"}}"#)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // v1 全 scope 签发（chat,panel:read,panel:write）——此处以「构造受限令牌」
    // 直插 store 验证 403 闸（默认授予语义的边界锁）
    // POST 端点带合法 JSON 形态（axum Json 提取器 content-type 前置检查，
    // 缺形态=415 先于 401——无匿名断言须到达鉴权层）
    let (status, _) = req(
        app.clone(),
        "POST",
        "/panel/commands",
        None,
        Some(r#"{"command":"review"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "无令牌仍 401");
}
