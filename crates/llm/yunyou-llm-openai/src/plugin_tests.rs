//! llm-openai 插件单元测试（task-33⑤a 外迁：主体 464 行已爆 400 硬顶
//! ——task-43 send 段+回归测试增量所致；#[path] 外挂保持同 crate 私有可达
//! （stream_inner/stalled_inner 等私有项直接测）；外迁零语义改动。

use super::*;

    /// manifest 声明面字段齐全（方案 §2.2，S1 验收）。
    #[test]
    fn manifest声明面字段齐全() {
        let m = OpenAiPlugin.manifest();
        assert_eq!(m.id, "yunyou-llm-openai");
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(m.level, yunyou::PluginLevel::Bundled);
        assert!(m.auto_enable);
        assert_eq!(m.contracts, vec!["llm.v1".to_string()]);
        assert_eq!(m.provides, vec!["dyn Llm".to_string()]);
        assert!(m.inject.is_empty() && m.ui.is_empty());
    }

    /// manifest.inject 与 trait inject() 双向一致（装载前校验零违例）。
    #[test]
    fn manifest与trait_inject双向一致() {
        let plugin = OpenAiPlugin;
        let issues = yunyou::validate_manifest(&plugin, &plugin.manifest());
        assert!(issues.is_empty(), "校验违例: {issues:?}");
    }

    /// 全默认配置可构造（jsonschema 驱动的缺省链路）。
    #[test]
    fn 默认配置() {
        let cfg = OpenAiConfig::default();
        assert!(cfg.base_url.starts_with("https://"));
        assert_eq!(cfg.timeout_secs, 300);
        assert_eq!(cfg.stream_idle_timeout_ms, 30_000, "D-19-1：流内空闲看门狗默认 30s");
    }

    /// api_key 走配置注入；schema 不含具体密钥（防泄漏的结构性检查）。
    #[test]
    fn schema不含密钥值() {
        let cfg = OpenAiConfig { api_key: "sk-secret".into(), ..Default::default() };
        let schema = serde_json::to_value(schemars::schema_for!(OpenAiConfig)).unwrap();
        let text = schema.to_string();
        assert!(text.contains("api_key"), "字段名应在 schema 中");
        assert!(!text.contains("sk-secret"), "密钥值绝不入 schema/代码");
        drop(cfg);
    }

    // ── P1 修复回归（任务18 §3.1/§4 铁律②）：超时必须与网络错误分型 ──
    // ── D-19-1 回归：总预算语义废除，send 段预算与流内空闲看门狗分立 ──

    /// 构造指向本地 stall 服务的最小 inner（D-19-1 起 client 无总预算，
    /// 只有 connect_timeout；send 段预算与流内空闲看门狗由 config 传入）。
    fn stalled_inner(addr: std::net::SocketAddr, timeout_secs: u64, idle_ms: u64) -> LlmInner {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .expect("客户端构建");
        LlmInner {
            client,
            config: OpenAiConfig {
                base_url: format!("http://{addr}"),
                timeout_secs,
                stream_idle_timeout_ms: idle_ms,
                ..Default::default()
            },
        }
    }

    fn bare_request() -> GenerateRequest {
        GenerateRequest { model: "test-model".into(), messages: vec![], tools: vec![], temperature: None }
    }

    /// send 段预算耗尽 → Timeout（而非 Connection——后者默认白名单可重试，
    /// 会把「不确定结果」当「请求未达上游」重发，正中 §3.1 安全默认律禁区）。
    /// 离线 mock：本地 TCP 只接受不应答，timeout_secs=1 在连接后耗尽。
    #[tokio::test(flavor = "multi_thread")]
    async fn 发送段超时归timeout而非connection() {
        use yunyou_llm::LlmFailureCategory;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        let addr = listener.local_addr().expect("取端口");
        let _server = std::thread::spawn(move || {
            let _held = listener.accept();
            std::thread::sleep(Duration::from_secs(5)); // 挂住不应答；分离线程随进程退出
        });
        let inner = stalled_inner(addr, 1, 0);
        // Box<dyn LlmStream> 不实现 Debug，不能 expect_err——match 显式 panic 带错误文本。
        let err = match stream_inner(&inner, bare_request(), CancellationToken::new()).await {
            Err(err) => err,
            Ok(_) => panic!("send 段预算耗尽必须报错，实得 Ok(流)"),
        };
        let back = LlmFailure::from_cordis(&err).expect("应可还原 LlmFailure");
        assert_eq!(
            back.category,
            LlmFailureCategory::Timeout,
            "send 段超时应归 Timeout，实得 {:?}: {err}",
            back.category
        );
        assert!(!back.category.retryable(), "安全默认律：超时默认不重试");
    }

    /// R2 回归（task-31 根因②）：send 段（连接+收头）挂住时 cancel 必须
    /// 立即断（百毫秒级），不得挂满 send 预算——对话回合中 Ctrl-C 的取消
    /// 提示时延上限由本契约锁定。错误形态与读循环 cancel 分支同构
    /// （LlmFailure::cancelled→Cancelled）。外层 2s 超时保护：若 send 段
    /// 退回对 cancel 零感知（挂满 300s 预算），测试以超时失败而非卡死 CI。
    /// 离线 mock：本地 TCP 收请求后挂住不应答。
    #[tokio::test(flavor = "multi_thread")]
    async fn 发送段挂住时取消立即断() {
        use yunyou_llm::LlmFailureCategory;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        let addr = listener.local_addr().expect("取端口");
        let _server = std::thread::spawn(move || {
            let _held = listener.accept();
            std::thread::sleep(Duration::from_secs(30)); // 收请求后挂住不应答
        });
        let inner = stalled_inner(addr, 300, 0);
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
            tokio::time::timeout(Duration::from_secs(2), stream_inner(&inner, bare_request(), cancel)).await;
        let elapsed = started.elapsed();
        let err = match outcome {
            Err(_) => panic!("取消后 2s 内必须落定（实耗 {elapsed:?}）——send 段对 cancel 零感知则挂满 300s 预算"),
            Ok(Err(err)) => err,
            Ok(Ok(_)) => panic!("挂头场景取消后必须 Err，实得 Ok(流)"),
        };
        assert!(elapsed < Duration::from_millis(500), "取消应立即断（实耗 {elapsed:?}）");
        let back = LlmFailure::from_cordis(&err).expect("应可还原 LlmFailure");
        assert_eq!(
            back.category,
            LlmFailureCategory::Cancelled,
            "取消形态应与读循环 cancel 分支同构，实得 {:?}: {err}",
            back.category
        );
        canceller.abort();
    }

    /// 读流段空闲看门狗（D-19-1）：chunk 间无字节超 idle 预算 → Timeout；
    /// 真断连才归 StreamInterrupted——两型不混（§4 铁律②）。
    /// 离线 mock：本地 TCP 立即回 200 SSE 头后挂住不发体，idle=300ms 必判死。
    #[tokio::test(flavor = "multi_thread")]
    async fn 读流空闲超时归timeout而非流中断() {
        use std::io::Write;
        use yunyou_llm::LlmFailureCategory;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        let addr = listener.local_addr().expect("取端口");
        let _server = std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else { return };
            // 先读掉请求头（对齐真实服务器时序——未读先答会触发 hyper 客户端
            // UnexpectedMessage），再回 200 SSE 头并挂住不发体。
            use std::io::Read as _;
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            let _ = sock.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
            );
            let _ = sock.flush();
            std::thread::sleep(Duration::from_secs(5)); // 挂住不发体
        });
        let inner = stalled_inner(addr, 30, 300);
        // Box<dyn LlmStream> 不实现 Debug，不能 expect_err——match 显式 panic 带错误文本。
        let mut stream = match stream_inner(&inner, bare_request(), CancellationToken::new()).await {
            Ok(stream) => stream,
            Err(err) => panic!("响应头已到，流应建立，实得: {err}"),
        };
        let err = stream.next().await.expect_err("空闲看门狗必须判死");
        let back = LlmFailure::from_cordis(&err).expect("应可还原 LlmFailure");
        assert_eq!(
            back.category,
            LlmFailureCategory::Timeout,
            "读流空闲超时应归 Timeout，实得 {:?}: {err}",
            back.category
        );
        assert!(err.message().contains("空闲超时"), "文案应点名空闲语义: {err}");
    }

    /// D-19-1 本体回归（复核 D1 判别力修正）：**慢流在旧总预算语义下必死、
    /// 新空闲语义下健康收完**。判别力构成：timeout_secs=1（旧语义=builder
    /// 总预算 1s 覆盖读流）+chunk 间隔 400ms×3+[DONE]（总耗时 ~1.2s>1s——
    /// **旧代码此测试必红**，锁得住「有人加回 builder.timeout」；新空闲语义
    /// 间隙 400ms<idle 2s 健康收完——判别力成立）。idle=2s+send 不限时则
    /// 测不出回归（旧代码同样绿），已弃。
    #[tokio::test(flavor = "multi_thread")]
    async fn 慢流在空闲语义下健康收完() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        let addr = listener.local_addr().expect("取端口");
        let _server = std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else { return };
            use std::io::Read as _;
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            // connection: close + 无长度头：写完体即关连接，reqwest 读到 EOF 收尾
            //（不用 chunked——手写帧长度算术是 mock 自找的解码坑）
            let _ = sock.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            );
            let _ = sock.flush();
            // 模拟推理模型慢流：三段思考字节，段间 400ms——总时长 ~1.2s 越过
            // 旧总预算 1s（判别力），间隙远小于空闲看门狗 2s（新语义健康）
            for _ in 0..3 {
                std::thread::sleep(Duration::from_millis(400));
                let _ = sock.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\".\"}}]}\n\n");
                let _ = sock.flush();
            }
            let _ = sock.write_all(b"data: [DONE]\n\n");
            let _ = sock.flush();
            drop(sock); // 关连接=流 EOF（正常收尾路径）
        });
        let inner = stalled_inner(addr, 1, 2_000); // send 段 1s（本地秒回不受影响）+idle 2s
        let mut stream = match stream_inner(&inner, bare_request(), CancellationToken::new()).await {
            Ok(stream) => stream,
            Err(err) => panic!("响应头应立即到，流应建立: {err}"),
        };
        let mut deltas = 0usize;
        while let Some(delta) = stream.next().await.expect("健康流不应报错") {
            let _ = delta;
            deltas += 1;
        }
        assert!(deltas >= 3, "三段思考增量应全部送达，实得 {deltas}");
    }

    /// task-49① 锁：SSE 断流（EOF 无 [DONE]）必须归 StreamInterrupted 可重试
    /// 错误，绝不静默 Ok(None) 当正常收束——业主实证「回合尾部无声失踪」
    /// （tool-result 后 5 次调用彻底无声）的真根之一。离线 mock：回 200 SSE
    /// 头+一块增量后立即 close（无哨兵）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 断流无done归流中断而非静默() {
        use std::io::Write as IoWrite;
        use yunyou_llm::LlmFailureCategory;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        let addr = listener.local_addr().expect("取端口");
        std::thread::spawn(move || {
            let (mut c, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut c, &mut buf); // 读走请求头
            let _ = c.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            );
            let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"部分\"}}]}\n\n";
            let _ = c.write_all(format!("{:x}\r\n{}\r\n", sse.len(), sse).as_bytes());
            drop(c); // 断流：无 [DONE] 直接 close
        });
        let inner = stalled_inner(addr, 10, 0);
        let mut stream = match stream_inner(&inner, bare_request(), CancellationToken::new()).await {
            Ok(s) => s,
            Err(err) => panic!("响应头已到应得流，实得 {err}"),
        };
        // 首块增量正常到达
        let first = stream.next().await.expect("next 应 Ok").expect("应有增量");
        assert!(matches!(first, yunyou_llm::StreamDelta::Text(_)), "首块应为文本增量");
        // EOF 无 [DONE] → 流中断错误（而非静默 None）
        let err = match stream.next().await {
            Err(err) => err,
            Ok(Some(delta)) => panic!("断流应 Err，实得增量 {delta:?}"),
            Ok(None) => panic!("断流被静默收束 Ok(None)——task-49① 回归（业主无声失踪实锤形态）"),
        };
        let back = LlmFailure::from_cordis(&err).expect("可还原 LlmFailure");
        assert_eq!(
            back.category,
            LlmFailureCategory::StreamInterrupted,
            "断流应归可重试的流中断，实得 {:?}: {err}",
            back.category
        );
        assert!(
            err.to_string().contains("流式响应"),
            "报错应指向流式响应层（TCP 硬断走 chunk 错误层）：{err}"
        );
    }

    /// task-49① 锁（形态二）：chunked 终止块正确但 SSE 漏 [DONE]——优雅 EOF
    /// 走 done_seen 新路径，报「未以完成哨兵收束」（携带已收增量诊断数），
    /// 归 StreamInterrupted 可重试。离线 mock：头+一块+终止块+close。
    #[tokio::test(flavor = "multi_thread")]
    async fn 优雅eof漏done报未收束() {
        use std::io::Write as IoWrite;
        use yunyou_llm::LlmFailureCategory;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        let addr = listener.local_addr().expect("取端口");
        std::thread::spawn(move || {
            let (mut c, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut c, &mut buf);
            let _ = c.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            );
            let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"部分\"}}]}\n\n";
            let _ = c.write_all(format!("{:x}\r\n{}\r\n", sse.len(), sse).as_bytes());
            let _ = c.write_all(b"0\r\n\r\n"); // chunked 终止块：优雅关但无 [DONE]
            drop(c);
        });
        let inner = stalled_inner(addr, 10, 0);
        let mut stream = stream_inner(&inner, bare_request(), CancellationToken::new())
            .await
            .expect("头已到应得流");
        let _ = stream.next().await.expect("首块应 Ok");
        let err = match stream.next().await {
            Err(err) => err,
            Ok(Some(delta)) => panic!("应已断流，实得增量 {delta:?}"),
            Ok(None) => panic!("优雅 EOF 漏 [DONE] 被静默收束——task-49① 新路径回归"),
        };
        let back = LlmFailure::from_cordis(&err).expect("可还原 LlmFailure");
        assert_eq!(
            back.category,
            LlmFailureCategory::StreamInterrupted,
            "实得 {:?}: {err}",
            back.category
        );
        assert!(
            err.to_string().contains("未以完成哨兵收束"),
            "应走 done_seen 新路径文案：{err}"
        );
    }