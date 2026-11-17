//! OpenAPI 骨架与路由覆盖锁（网关落地方案 §8 阶段 C）。
//!
//! 生成化升级路径：方案 §8 定案「utoipa derive + xtask doc-api 生成」——xtask
//! crate 未建（工作区无此件），v1 骨架=手写 yaml+**路由覆盖测试锁**（本文档
//! 所列路径必须全部在路由器上挂载，漂移即红——手写与实现的单向绑定闸，生成
//! 化升级时由 utoipa 生成物替代并保留覆盖锁）。

use axum::{body::Body, http::{Request, StatusCode}};
use tower::ServiceExt; // oneshot
use std::sync::Arc;
use yunyou_gateway::{http, GatewayStore};

/// OpenAPI 骨架（方案 §8 端点清单；security 统一 bearerAuth，healthz 除外）。
const OPENAPI_PATHS: &[(&str, &[&str])] = &[
    ("/pair/consume", &["post"]),
    ("/pair/device-code", &["post"]),
    ("/devices", &["get", "delete"]),
    ("/devices/{id}", &["delete"]),
    ("/panel/tasks", &["get"]),
    ("/panel/commands", &["post"]),
    ("/ws/chat", &["get"]),
    ("/healthz", &["get"]),
];

/// OpenAPI 骨架 yaml（入库物；生成化升级前的人工单源，覆盖锁防漂移）。
pub const OPENAPI_YAML: &str = r#"openapi: 3.0.3
info:
  title: yunyou-gateway API
  version: 0.1.0
  description: 网关两维矩阵（内网/公网×扫码/验证码）——四格鉴权路径同构（方案 §6），无匿名通道（业主 D2 定案：healthz 除外）
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
      description: 会话令牌（dsh_sess_ 前缀，SHA-256 哈希存储，generation 吊销立即生效）
security:
  - bearerAuth: []
paths:
  /pair/consume:
    post:
      summary: 扫码配对消费（一次性配对令牌换会话令牌）
      security: []          # 配对凭证本身即凭证（dsh_pair_ Bearer），会话令牌签发前
      requestBody:
        content:
          application/json:
            schema:
              type: object
              required: [device_name]
              properties:
                device_name: { type: string }
      responses:
        "200": { description: 会话令牌签发 }
        "401": { description: 配对凭证无效/过期 }
        "409": { description: 已消费（重放拒绝）/设备数上限 }
        "429": { description: 失败锁定中（5 次→15 分钟） }
  /pair/device-code:
    post:
      summary: 验证码配对消费（设备码流程，对标 OAuth Device Flow 简化版）
      security: []
      requestBody:
        content:
          application/json:
            schema:
              type: object
              required: [code, device_name]
              properties:
                code: { type: string }
                device_name: { type: string }
      responses:
        "200": { description: 会话令牌签发 }
        "401": { description: 验证码无效/过期 }
        "429": { description: 失败锁定中 }
  /devices:
    get:
      summary: 设备列表（活跃令牌设备）
      responses:
        "200": { description: 设备清单 }
    delete:
      summary: 吊销全部设备（立即生效）
      responses:
        "200": { description: 吊销枚数 }
  /devices/{id}:
    delete:
      summary: 吊销单设备（立即生效）
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
      responses:
        "200": { description: 吊销枚数 }
  /panel/tasks:
    get:
      summary: 面板任务只读投影（scope=panel:read；agent 侧编排层生成）
      responses:
        "200": { description: 只读投影快照 }
        "401": { description: 未认证 }
        "403": { description: scope 不足 }
  /panel/commands:
    post:
      summary: 面板命令入队（scope=panel:write；只入队不直写，§2.3 铁律）
      requestBody:
        content:
          application/json:
            schema:
              type: object
              required: [command]
              properties:
                command: { type: string }
      responses:
        "200": { description: 入队确认 }
        "401": { description: 未认证 }
        "403": { description: scope 不足 }
  /ws/chat:
    get:
      summary: 对话 WebSocket（scope=chat；连接绑定设备，吊销即断连 §3.1）
      responses:
        "101": { description: 升级为 WebSocket }
        "401": { description: 未认证 }
  /healthz:
    get:
      summary: 探活（匿名 D2 定案；零信息=响应体恰为 ok；限速 6 次/分）
      security: []
      responses:
        "200": { description: ok }
"#;

/// 路由覆盖锁：OPENAPI_PATHS 逐条实测路由器挂载（方法+路径精确匹配；
/// yaml 手写与实现的单向绑定闸——文档列了路由上没有的路径=红）。
#[tokio::test(flavor = "multi_thread")]
async fn openapi_骨架与路由覆盖一致() {
    let store = Arc::new(GatewayStore::open_in_memory().unwrap());
    let config = yunyou_gateway::config::GatewayConfig::default();
    let app = http::router(http::GatewayState::new(
        store,
        &config,
        Arc::new(yunyou_gateway::audit::AuditLog::in_memory()),
    ));
    for (path, methods) in OPENAPI_PATHS {
        for method in *methods {
            let request = Request::builder()
                .method(*method)
                .uri(*path)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            // 覆盖判据：非 404（405 同端点方法不符也视为挂载——GET-only 端点
            // POST 探测=405 恰证明路径在位）。healthz 匿名=200。
            assert_ne!(
                status,
                StatusCode::NOT_FOUND,
                "OpenAPI 列了路由上不存在的路径: {method} {path}"
            );
        }
    }
}

/// 骨架 yaml 入库物与常量一致（防两处手写漂移；生成化升级时本测试改为
/// 比对 utoipa 生成物）。
#[test]
fn yaml入库物与单源一致() {
    let checked_in = std::fs::read_to_string("../../docs/接口/openapi-gateway.yaml")
        .expect("openapi-gateway.yaml 应在接口区入库");
    assert_eq!(checked_in.trim(), OPENAPI_YAML.trim(), "入库 yaml 与单源漂移");
}
