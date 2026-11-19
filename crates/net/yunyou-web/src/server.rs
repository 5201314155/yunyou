//! axum 服务装配：静态资产路由 + token 鉴权中间件（web 安全线；实例起停面在 instance.rs，任务40 期间拆出——行数纪律）。
//!
//! # 路由面（P2 起）
//! `/`（页面）、`/app.css`、`/app.js`、`/tokens.js`（设计令牌生成物）、`/ws`
//! （TurnObserver 事件流推送 + 输入提交；鉴权中间件先行，未带 token 连 /ws
//! 同样 403，无匿名通道）。
//!
//! # 安全（网关 D7）
//! 默认绑 127.0.0.1 + 随机 token；启动打印 `http://<addr>/?token=…` 完整地址；
//! 全路由过同一鉴权中间件（页面与 WS 握手无差别）。LAN 暴露需显式 --addr，
//! token 恒开，非回环地址启动时追加外泄提醒。

use crate::protocol::ServerMessage;
use crate::token;
use crate::ws;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

/// 静态资产（嵌入二进制；零 node 构建链——vanilla JS/CSS，按职责拆文件）。
const INDEX_HTML: &str = include_str!("../assets/index.html");
const APP_CSS: &str = include_str!("../assets/app.css");
const APP_JS: &str = include_str!("../assets/app.js");
const RENDER_JS: &str = include_str!("../assets/render.js");
const WS_CLIENT_JS: &str = include_str!("../assets/ws-client.js");
/// 设计令牌生成物（build.rs 从 docs/设计/tokens.json 产出，勿手改）。
const TOKENS_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/tokens.css"));
const TOKENS_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/tokens.js"));

/// 服务状态（鉴权中间件与 WS 层共享；进程级单例；构造在 instance.rs）。
pub(crate) struct WebState {
    pub(crate) token: String,
    /// 模型名（连接问候/落款）。
    pub(crate) model: String,
    /// 回合执行器（回合闸门内含）。
    pub(crate) turns: Arc<crate::turns::TurnRunner>,
    /// 事件广播（全部连接同看同源事件流）。
    pub(crate) broadcast: broadcast::Sender<ServerMessage>,
    /// 会话目录（P4 列表与按名回放）。
    pub(crate) session_dir: PathBuf,
    /// 对端 wire 桥（ServerMessage→对端帧；v2 mux follow 流与 prompt 桥共用，
    /// 任务40 适配层第二片）。
    pub(crate) wire: Arc<crate::bridge::WireBridge>,
    /// 定时调度器（W4 schedule 域；到期触发器注入回合——instance.rs 装配）。
    pub(crate) scheduler: Arc<yunyou_plugin_sdk::scheduler::Scheduler>,
    /// 配置修订写（W5 settings 域；revision 单源+原子写——config_store.rs）。
    pub(crate) config_store: Arc<crate::config_store::ConfigStore>,
    /// 监听地址回环性（devices/current loopback；W6——instance 解析后传入）。
    pub(crate) loopback: bool,
    /// 会话换根口（D2-b；真源=slot 换根结果，wire 仅被动投影派生）。
    pub(crate) switcher: SessionSwitcher,
}

/// 会话换根口类型（D2-b 云枢裁 b 案）：target=None=铸新会话路径；
/// 返回换根结果人话（CommandOutput）。web 不依赖 ShellCore 类型（闭包解耦）。
pub(crate) type SessionSwitcher = Arc<
    dyn Fn(Option<std::path::PathBuf>) -> yunyou_util::BoxFut<'static, std::result::Result<(), yunyou::CordisError>>
        + Send
        + Sync,
>;

/// 路由表：全部路由先过 token 鉴权中间件（from_fn_with_state），无匿名通道。
/// v2 形态（dist 可用）：`/` 与 `/{*path}` 服务官方移植前端产物；v1 vanilla
/// 路由保留作回退（dist 缺席时的 v1 形态，v2 收敛批裁决去留）。
pub(crate) fn router(state: Arc<WebState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.css", get(app_css))
        .route("/app.js", get(app_js))
        .route("/render.js", get(render_js))
        .route("/ws-client.js", get(ws_client_js))
        .route("/tokens.js", get(tokens_js))
        .route("/api/remote.mux", get(crate::gateway::mux_upgrade))
        .route("/api", post(crate::rpc::rpc_unary))
        // 方法即路径：对端 client 按 method 名 POST /api/<method>（线体同名）
        .route("/api/{*path}", post(crate::rpc::rpc_unary))
        // P0 修复（云尺 E2E 实测）：catch-all .route("/{*path}") 抢占全部 POST
        // 匹配（matchit 优先级，POST /api 实测 405=web RPC 全阻）——dist 静态面
        // 改 Router::fallback（axum 惯用法，优先级最低，静态与 POST 天然不抢）
        .fallback(fallback_asset)
        .route("/ws", get(ws_upgrade))
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state)
}

/// token 鉴权中间件：查询串优先、Bearer 回退、会话 cookie 兜底（见
/// token::from_request）。校验失败一律 403（不区分「缺令牌/错令牌」，不给
/// 可猜测面）。校验通过且请求未带 cookie 时，响应追加 Set-Cookie 下发会话
/// cookie（v2 页内子资源与 WS 重连均走 cookie——R2 token 抹除：浏览器此后
/// 可用无 token 的干净 URL 访问，URL 里的 token 一次性消费后即弃）。
async fn require_token(State(state): State<Arc<WebState>>, req: Request, next: Next) -> Response {
    let presented = token::from_request(req.uri().query(), req.headers());
    let has_cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.split(';').any(|p| p.trim().starts_with(token::SESSION_COOKIE)));
    if presented.is_some_and(|t| token::verify(&t, &state.token)) {
        let mut resp = next.run(req).await;
        if !has_cookie {
            let cookie = format!(
                "{}={}; Path=/; HttpOnly; SameSite=Lax",
                token::SESSION_COOKIE,
                state.token
            );
            if let Ok(value) = header::HeaderValue::from_str(&cookie) {
                resp.headers_mut().append(header::SET_COOKIE, value);
            }
        }
        resp
    } else {
        deny()
    }
}

/// 拒绝页（403）：人话指路——用终端打印的完整地址重开。
fn deny() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Html(
            "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\">\
             <title>令牌无效 · 云悠</title><p>令牌无效：请用 <code>yunyou web</code> \
             终端打印的完整地址（含 <code>?token=…</code>）重新打开。</p></html>",
        ),
    )
        .into_response()
}

/// 资产响应统一形态：no-store——业主实测期改码刷新即见效，不与浏览器缓存搏斗。
fn asset(body: String, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

async fn index() -> Response {
    // v2 优先：官方移植前端入口；dist 缺席回退 v1 vanilla 页。
    if let Some(bytes) = crate::dist::index() {
        return asset(
            String::from_utf8_lossy(bytes).into_owned(),
            "text/html; charset=utf-8",
        );
    }
    Html(INDEX_HTML).into_response()
}

/// v2 产物服务：dist 内相对路径精确命中 → 字节+按扩展名 Content-Type；
/// 未命中且路径无扩展名（深链形态）→ SPA 回退 index.html；其余 404。
/// dist 缺席 → 404（v1 形态下无此路由语义）。
async fn fallback_asset(
    State(state): State<Arc<WebState>>,
    req: axum::extract::Request,
) -> Response {
    // 静态面只应答 GET/HEAD（原 catch-all 同语义：其余方法 405——POST 打资产名
    // 不再被 catch-all 吞成 405 全阻，但也不冒充资产成功）
    if req.method() != axum::http::Method::GET && req.method() != axum::http::Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "静态资产仅支持 GET/HEAD").into_response();
    }
    let _ = state; // 鉴权中间件已先行（layer 覆盖 fallback）
    fallback_asset_impl(req.uri().path())
}

/// fallback 静态资产（原 dist_asset；路径自取 uri——fallback 无 Path extractor）。
fn fallback_asset_impl(path: &str) -> Response {
    if !crate::dist::available() {
        return (StatusCode::NOT_FOUND, "v2 前端未构建（dist 缺席）").into_response();
    }
    let full = path.to_string();
    let hit = match crate::dist::get(&full) {
        Some(bytes) => Some((bytes, crate::dist::content_type(&full))),
        // 无产物扩展名 = 页面深链（如 /sessions/abc）→ 回单页入口
        None if !looks_like_file(&full) => {
            crate::dist::index().map(|bytes| (bytes, "text/html; charset=utf-8"))
        }
        None => None,
    };
    let Some((bytes, content_type)) = hit else {
        return (StatusCode::NOT_FOUND, "产物不存在").into_response();
    };
    let resp = if content_type.starts_with("text/") {
        (
            [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
            String::from_utf8_lossy(bytes).into_owned(),
        )
            .into_response()
    } else {
        (
            [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
            bytes.to_vec(),
        )
            .into_response()
    };
    with_no_store(resp)
}

/// 「像文件」的扩展名白名单（vite 产物全量；命中即不做 SPA 回退）。
const EXT_LIKE: &[&str] = &[
    "html", "js", "mjs", "css", "json", "webmanifest", "svg", "png", "jpg", "jpeg", "gif",
    "ico", "woff", "woff2", "ttf", "otf", "txt", "map", "wasm", "bin", "tar", "gz",
];

/// 尾段扩展名是否在白名单（防把页面深链误当产物）。
fn looks_like_file(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|ext| EXT_LIKE.contains(&ext))
}

/// 给响应追加 no-store（与 asset() 同纪律：实测期改码刷新即见效）。
fn with_no_store(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// CSS = 令牌生成物（变量定义）+ 壳层样式（变量消费），拼装顺序即优先级。
async fn app_css() -> Response {
    asset(format!("{TOKENS_CSS}\n{APP_CSS}"), "text/css; charset=utf-8")
}

async fn app_js() -> Response {
    asset(APP_JS.to_string(), "text/javascript; charset=utf-8")
}

async fn render_js() -> Response {
    asset(RENDER_JS.to_string(), "text/javascript; charset=utf-8")
}

async fn ws_client_js() -> Response {
    asset(WS_CLIENT_JS.to_string(), "text/javascript; charset=utf-8")
}

async fn tokens_js() -> Response {
    asset(TOKENS_JS.to_string(), "text/javascript; charset=utf-8")
}

/// WS 升级（鉴权中间件已先行放行）：连接生命周期移交 ws::handle_socket。
async fn ws_upgrade(State(state): State<Arc<WebState>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| ws::handle_socket(socket, state))
}

#[cfg(test)]
#[path = "server_tests.rs"]
pub(crate) mod tests;
