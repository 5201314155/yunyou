//! server.rs 测试（外挂：#[path] 先例，源文件行数纪律——295cc25 同款）。
//! 真 HTTP/WS 打真端口（start_instance/start_with_state 底座，不 mock 通道）。

use super::*;
use crate::instance::{start_instance, start_with_state, WebParams};
use yunyou::Context;

/// 测试辅助（gateway 测试共用）：传入态直接起服务（start_with_state——
/// 测试态即服务态，事件广播走同一通道）→ 真 WS 连 /api/remote.mux。
pub(crate) async fn test_ws_connect(
    state: Arc<WebState>,
) -> tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
> {
    let instance = start_with_state("127.0.0.1:0".parse().expect("回环地址"), state)
        .await
        .expect("启动实例");
    let url = format!(
        "ws://{}/api/remote.mux?token={}",
        instance.addr, instance.token
    );
    let (ws, _) = tokio_tungstenite::connect_async(url).await.expect("mux 握手");
    // 实例句柄随流生命周期（测试内不做优雅收场，进程结束即回收）
    std::mem::forget(instance);
    ws
}

/// 测试参数：root_getter 给一次性 root（P1 无对话路径，不触达）。
fn params() -> WebParams {
    WebParams {
        scheduled_rx: None,
        config_path: std::env::temp_dir().join("yunyou-web-server-test.toml"),
        root_getter: Arc::new(Context::new),
        session_dir: PathBuf::from("/tmp/yunyou-web-test-sessions"),
        model: "test-model".into(),
        addr: None,
    }
}

/// 鉴权两态 + 资产可达（真机 HTTP：reqwest 打真实端口，不 mock）。
#[tokio::test(flavor = "multi_thread")]
async fn 鉴权两态与静态资产可达() {
    let mut params = params();
    let instance = start_instance(&mut params).await.expect("启动实例");
    let base = format!("http://{}/", instance.addr);

    // 无 token / 错 token → 403（页面、资产、WS 缝同判，无匿名通道）
    for path in ["", "?token=wrong", "ws", "app.css", "render.js", "ws-client.js"] {
        let url = format!("{base}{path}");
        let resp = reqwest::get(&url).await.expect("请求发出");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "无有效令牌应 403：{url}");
    }
    // Bearer 头形态同通行
    let ok = reqwest::Client::new()
        .get(format!("{base}app.js"))
        .header("Authorization", format!("Bearer {}", instance.token))
        .send()
        .await
        .expect("请求发出");
    assert_eq!(ok.status(), StatusCode::OK, "Bearer 应通行");

    // 会话 cookie 传递：带 token 首访下发 Set-Cookie，此后子资源（无查询
    // 串 token）凭 cookie 通行——v2 页内模块束/资产请求形态（R2 抹除语义）
    let client = reqwest::Client::new();
    let first = client
        .get(format!("{base}?token={}", instance.token))
        .send()
        .await
        .expect("请求发出");
    let cookie = first
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .expect("带 token 首访应下发会话 cookie")
        .to_string();
    assert!(cookie.starts_with("yunyou-web-token="), "cookie 名：{cookie}");
    assert!(cookie.contains("HttpOnly"), "会话 cookie 必须 httpOnly：{cookie}");
    let sub = client
        .get(format!("{base}tokens.js"))
        .header(header::COOKIE, cookie)
        .send()
        .await
        .expect("请求发出");
    assert_eq!(sub.status(), StatusCode::OK, "凭 cookie 的子资源应通行");

    // 带 token：页面 + v1 回退资产（CSS 令牌变量/JS 符号表）可达
    let page = reqwest::get(format!("{base}?token={}", instance.token))
        .await
        .expect("请求发出");
    assert_eq!(page.status(), StatusCode::OK);
    let html = page.text().await.expect("页面文本");
    if crate::dist::available() {
        // v2 形态：官方移植前端入口（模块脚本+清单），不再断言 v1 文案
        assert!(html.contains("assets/") || html.contains("manifest.webmanifest"),
            "应为官方壳入口页：{html}");
        // dist 产物经通配路由可达（清单件小而稳定）
        let manifest = reqwest::get(format!("{base}manifest.webmanifest?token={}", instance.token))
            .await
            .expect("请求发出");
        assert_eq!(manifest.status(), StatusCode::OK, "dist 清单应可达");
        // 产物缺失路径 404（.js 形态不做 SPA 回退）
        let missing = reqwest::get(format!("{base}no/such/file.js?token={}", instance.token))
            .await
            .expect("请求发出");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND, "产物缺失应 404");
    } else {
        assert!(html.contains("云悠"), "应为 hello 页：{html}");
    }
    let css = reqwest::get(format!("{base}app.css?token={}", instance.token))
        .await
        .expect("请求发出")
        .text()
        .await
        .expect("CSS 文本");
    assert!(css.contains("--yunyou-color-brand-primary"), "CSS 应含令牌变量");
    let js = reqwest::get(format!("{base}tokens.js?token={}", instance.token))
        .await
        .expect("请求发出")
        .text()
        .await
        .expect("JS 文本");
    assert!(js.contains("YUNYOU_SPINNER_FRAMES"), "JS 应含 spinner 帧");

    // WS 缝：带 token 但无升级头 → 升级提取器拒绝（400）；无 token → 403 已在上方判定
    let ws = reqwest::get(format!("{base}ws?token={}", instance.token))
        .await
        .expect("请求发出");
    assert_eq!(ws.status(), StatusCode::BAD_REQUEST, "非升级请求不应建立 WS");

    // 优雅收场
    instance.stop.cancel();
    instance.task.await.expect("任务 join").expect("serve 正常返回");
}

/// WS 端到端（真握手）：hello 首帧带模型名；提交一回合 → 无装配 root 下
/// 收到人话失败帧（闸门/服务缺失路径全走 Error 词汇，不静默）。
#[tokio::test(flavor = "multi_thread")]
async fn ws握手hello与回合失败路径() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let mut params = params();
    let instance = start_instance(&mut params).await.expect("启动实例");
    let url = format!("ws://{}/ws?token={}", instance.addr, instance.token);
    let (mut ws_stream, _resp) = tokio_tungstenite::connect_async(url)
        .await
        .expect("WS 握手应成功（token 有效）");

    // 首帧：hello 带模型名
    let first = ws_stream.next().await.expect("应有首帧").expect("帧应可读");
    let WsMessage::Text(data) = first else { panic!("首帧应为文本：{first:?}") };
    assert!(data.contains(r#""type":"hello""#), "首帧应为 hello：{data}");
    assert!(data.contains(r#""model":"test-model""#), "hello 应带模型名：{data}");

    // 连接自动回放：无 SessionStore 的测试 root → 会话服务未就绪 error 帧（P4 语义在场）
    let second = ws_stream.next().await.expect("应有回放帧").expect("帧应可读");
    let WsMessage::Text(data) = second else { panic!("回放帧应为文本：{second:?}") };
    assert!(
        data.contains(r#""type":"error""#) && data.contains("会话服务未就绪"),
        "连接自动回放在无装配下应给人话错：{data}"
    );

    // 提交回合：测试 root 无装配 → Error 人话帧（agentLoop 未就绪）
    ws_stream
        .send(WsMessage::text(r#"{"type":"submit","text":"你好"}"#))
        .await
        .expect("提交应可写");
    let reply = ws_stream.next().await.expect("应有回帧").expect("帧应可读");
    let WsMessage::Text(data) = reply else { panic!("回帧应为文本：{reply:?}") };
    assert!(data.contains(r#""type":"error""#), "无装配应回 error：{data}");
    assert!(data.contains("agentLoop"), "error 应带人话原因：{data}");

    // /quit：Bye 帧后服务端主动收尾（本端读到 Close 或流结束）
    ws_stream
        .send(WsMessage::text(r#"{"type":"submit","text":"/quit"}"#))
        .await
        .expect("/quit 应可写");
    let mut saw_bye = false;
    for _ in 0..8 {
        match ws_stream.next().await {
            Some(Ok(WsMessage::Text(data))) if data.contains(r#""type":"bye""#) => {
                saw_bye = true;
                break;
            }
            Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
            Some(Ok(_)) => {}
        }
    }
    assert!(saw_bye, "/quit 应收到 bye 帧");

    let _ = ws_stream.close(None).await;
    instance.stop.cancel();
    instance.task.await.expect("任务 join").expect("serve 正常返回");
}
