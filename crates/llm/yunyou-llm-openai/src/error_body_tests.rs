//! 非 2xx 错误体读取治理测试（task-13，发现人=云垒挑刺 P1-1；分型裁决=云枢预审）：
//! ①500+挂 body → timeout_secs 限时返回，分型保持 from_http_status(500)=ServerError
//!（分型跟事实走：状态码在 text() 之前已到手且为真，挂 body 的 429 与可读 body
//! 的 429 必须同分型同重试语义），message 带「错误体读取超时」注记；
//! ②挂 body 中取消 → 即时断（Cancelled，铁律取消优先于分型）；
//! ③500+完整 body → 行为不回归（detail 进 message、ServerError、无注记）——
//! 防修 A 破 B（D1 判别力先例）。mock 与 plugin_tests.rs 既有离线 mock 同款形态
//!（raw TCP 先读请求头再答；辅助函数自足不跨 mod 依赖）。

use super::*;

/// 构造「500 头+不完整体+永不完成」mock：content-length 声明 64 只发 5 字节后
/// 挂住（读端 text() 等满声明长度，mock 不再发=读体悬置永不完成，复现缺口本体；
/// 预算给小即可判别——旧代码裸 await 必挂死，新代码限时返回）。分离线程随进程
/// 退出，不 join。
fn error_body_stall_server() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
    let addr = listener.local_addr().expect("取端口");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let Ok((mut sock, _)) = listener.accept() else { return };
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf); // 读走请求头（对齐真实服务器时序，先例同款）
        let _ = sock.write_all(
            b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 64\r\nconnection: close\r\n\r\n{\"err",
        );
        let _ = sock.flush();
        std::thread::sleep(Duration::from_secs(30)); // 悬置不发余量；测试限时先落定
    });
    addr
}

/// 构造「500 头+完整 body」mock：content-length 精确声明并写完 body 后关连接
///（reqwest 读满声明长度收束）——行为不回归基线（用例③）。
fn error_body_complete_server() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
    let addr = listener.local_addr().expect("取端口");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let Ok((mut sock, _)) = listener.accept() else { return };
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf);
        let body = r#"{"error":{"message":"配额尽"}}"#.as_bytes();
        let head = format!(
            "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = sock.write_all(head.as_bytes());
        let _ = sock.write_all(body);
        let _ = sock.flush();
        drop(sock); // body 写完即关：读端读满 content-length 正常收束
    });
    addr
}

/// 与 plugin_tests.rs 同款最小构造（本 mod 自足：跨 mod 私有项不可达）。
fn stall_inner(addr: std::net::SocketAddr, timeout_secs: u64) -> LlmInner {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("客户端构建");
    LlmInner {
        client,
        config: OpenAiConfig {
            base_url: format!("http://{addr}"),
            timeout_secs,
            stream_idle_timeout_ms: 0,
            ..Default::default()
        },
    }
}

fn bare_request() -> GenerateRequest {
    GenerateRequest { model: "test-model".into(), messages: vec![], tools: vec![], temperature: None }
}

/// 用例①（验收④-限时返回+卡③分型细则）：错误体悬置永不完成 → timeout_secs=1
/// 预算耗尽限时返回；分型**保持 from_http_status(500)=ServerError**（云枢裁决：
/// 状态码已到手为真，分型跟事实走；Timeout 重分类=把真 5xx 藏进默认不重试档=
/// 语义降级），message 带「错误体读取超时」注记。外层 5s 超时保护：若退回裸
/// await（无预算），测试以超时失败而非挂死 CI。
#[tokio::test(flavor = "multi_thread")]
async fn 错误体挂住限时返回且分型保持server_error() {
    use yunyou_llm::LlmFailureCategory;
    let addr = error_body_stall_server();
    let inner = stall_inner(addr, 1);
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        stream_inner(&inner, bare_request(), CancellationToken::new()),
    )
    .await;
    let elapsed = started.elapsed();
    let err = match outcome {
        Err(_) => panic!("错误体悬置必须限时落定（实耗 {elapsed:?}）——退回裸 await 则挂死"),
        Ok(Err(err)) => err,
        Ok(Ok(_)) => panic!("500+挂 body 场景必须 Err，实得 Ok(流)"),
    };
    assert!(
        elapsed < Duration::from_secs(3),
        "应在 1s 预算级返回（实耗 {elapsed:?}）"
    );
    let back = LlmFailure::from_cordis(&err).expect("应可还原 LlmFailure");
    assert_eq!(
        back.category,
        LlmFailureCategory::ServerError,
        "body 超时不得改分型——from_http_status(500) 原 ServerError 必须保持（同分型同重试语义），实得 {:?}: {err}",
        back.category
    );
    assert!(
        back.message.contains("错误体读取超时"),
        "message 应带超时注记供诊断: {err}"
    );
}

/// 用例②（验收④-取消即时生效）：错误体悬置中 cancel 100ms 触发 → <500ms 断，
/// 不等 300s 预算——Ctrl-C 可解性回归，分型=Cancelled（铁律①取消优先于一切，
/// 先于分型）。外层 2s 超时保护（先例同款形态）。
#[tokio::test(flavor = "multi_thread")]
async fn 错误体挂住取消立即断() {
    use yunyou_llm::LlmFailureCategory;
    let addr = error_body_stall_server();
    let inner = stall_inner(addr, 300);
    let cancel = CancellationToken::new();
    let canceller = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        })
    };
    let started = std::time::Instant::now();
    let outcome =
        tokio::time::timeout(Duration::from_secs(2), stream_inner(&inner, bare_request(), cancel))
            .await;
    let elapsed = started.elapsed();
    let err = match outcome {
        Err(_) => panic!("取消后 2s 内必须落定（实耗 {elapsed:?}）——错误体读对 cancel 零感知则挂满 300s 预算"),
        Ok(Err(err)) => err,
        Ok(Ok(_)) => panic!("取消场景必须 Err，实得 Ok(流)"),
    };
    assert!(elapsed < Duration::from_millis(500), "取消应立即断（实耗 {elapsed:?}）");
    let back = LlmFailure::from_cordis(&err).expect("应可还原 LlmFailure");
    assert_eq!(
        back.category,
        LlmFailureCategory::Cancelled,
        "取消形态应与 send 段/读循环同构，实得 {:?}: {err}",
        back.category
    );
    canceller.abort();
}

/// 用例③（云枢预审【二】②：防修 A 破 B）：500+完整 body → 行为不回归——
/// detail（error.message）进 message、分型 ServerError、无「错误体读取」注记
///（body 可读时零新行为）。预算 1s 足够本地读完整体。
#[tokio::test(flavor = "multi_thread")]
async fn 错误体完整时行为不回归() {
    use yunyou_llm::LlmFailureCategory;
    let addr = error_body_complete_server();
    let inner = stall_inner(addr, 1);
    let err = match stream_inner(&inner, bare_request(), CancellationToken::new()).await {
        Err(err) => err,
        Ok(_) => panic!("500+完整 body 场景必须 Err，实得 Ok(流)"),
    };
    let back = LlmFailure::from_cordis(&err).expect("应可还原 LlmFailure");
    assert_eq!(
        back.category,
        LlmFailureCategory::ServerError,
        "可读 body 的 500 原分型必须保持，实得 {:?}: {err}",
        back.category
    );
    assert!(back.message.contains("配额尽"), "detail 应进 message: {err}");
    assert!(
        !back.message.contains("错误体读取"),
        "完整 body 场景不得出现读取注记（行为不回归）: {err}"
    );
}
