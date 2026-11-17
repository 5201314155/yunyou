//! S10 重试消费点集成测试（任务18；《超时重试插件设计》§7 S10 验收②③④）。
//!
//! 验收口径对照：
//! ①默认配置零手填行为正确（连接失败重试/超时不重试/预算耗尽放弃）；
//! ②退避可取消（取消即断链）；④mock Llm 注入可控失败序列（FlakyLlm）。
//! ③observer 事件在场（RetrySpy 收集 on_retry_scheduled）。

mod common;

use common::{ensure_commands, FlakyLlm, FlakyOutcome, wait_active};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use yunyou::ServiceId;
use yunyou_agent_loop::{AgentLoop, AgentLoopPlugin, TurnEvent, TurnObserver};
use yunyou_llm::{LlmFailure, StreamDelta};
use yunyou_retry::RetryPlugin;
use yunyou_session::SessionPlugin;
use yunyou_system_prompt::SystemPromptPlugin;
use yunyou_tools::ToolsPlugin;
use yunyou_util::BoxFut;

/// 纯文本收尾脚本。
fn answer_script(text: &str) -> FlakyOutcome {
    FlakyOutcome::Script(vec![
        StreamDelta::Text(text.into()),
        StreamDelta::Done { finish_reason: Some("stop".into()), usage: None },
    ])
}

/// 连接失败（默认白名单内，可重试）。
fn connection_fail() -> FlakyOutcome {
    FlakyOutcome::Fail(LlmFailure::connection("演练断连"))
}

/// 收集重试调度事件的观察者（验证 S10 验收③「observer 事件在场」）。
#[derive(Default)]
struct RetrySpy {
    events: Mutex<Vec<(u32, u64, String)>>,
}

impl TurnObserver for RetrySpy {
    fn on_event(&self, _event: TurnEvent) -> BoxFut<'_, ()> {
        Box::pin(async {})
    }
    fn on_retry_scheduled(&self, attempt: u32, delay_ms: u64, cause: &str) -> BoxFut<'_, ()> {
        self.events.lock().unwrap().push((attempt, delay_ms, cause.to_string()));
        Box::pin(async {})
    }
}

/// 装配最小团队（对齐 turn.rs 已验证装配序：llm 在 agent-loop 之前；retry 在
/// agent-loop 之前注册=bundled 清单序的消费时序实证）。
/// `retry_config` 传 None = 不装配 RetryPlugin（禁用/缺席形态，直连直出）。
async fn assemble(
    dir: &std::path::Path,
    llm: Arc<dyn yunyou_llm::Llm>,
    retry_config: Option<serde_json::Value>,
) -> yunyou::Context {
    let root = yunyou::Context::new();
    wait_active(
        &root.plugin(
            SessionPlugin,
            serde_json::json!({ "path": dir.join("session.jsonl").to_string_lossy() }),
        ),
        "session",
    )
    .await;
    ensure_commands(&root).await;
wait_active(
        &root.plugin(ToolsPlugin, serde_json::json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(
        &root.plugin(common::FakeLlmPlugin { llm }, serde_json::json!({})),
        "fake-llm",
    )
    .await;
    wait_active(
        &root.plugin(SystemPromptPlugin, serde_json::json!({})),
        "system-prompt",
    )
    .await;
    if let Some(cfg) = retry_config {
        wait_active(&root.plugin(RetryPlugin, cfg), "retry").await;
    }
    wait_active(&root.plugin(AgentLoopPlugin, serde_json::json!({})), "agent-loop").await;
    root
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dsh-retry-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 验收①：连接失败（白名单内）→ 重试 1 次后成功；调用计数=2；observer 事件在场。
#[tokio::test(flavor = "multi_thread")]
async fn 连接失败重试后成功() {
    let dir = temp_dir("ok");
    let llm = Arc::new(FlakyLlm::new(vec![connection_fail(), answer_script("重试后成功")]));
    let root = assemble(&dir, llm.clone(), Some(serde_json::json!({}))).await;

    let spy = Arc::new(RetrySpy::default());
    let outcome = {
        let svc = root
            .clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见");
        svc.run_turn_observed("你好", tokio_util::sync::CancellationToken::new(), spy.clone())
            .await
            .expect("重试后应成功")
    };
    assert_eq!(outcome.assistant_text, "重试后成功");
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 2, "应恰 2 次调用");
    let events = spy.events.lock().unwrap();
    assert_eq!(events.len(), 1, "应恰 1 次重试调度事件");
    assert_eq!(events[0].0, 1, "首次重试 attempt=1");
    assert_eq!(events[0].2, "connection", "cause 应为类别码");
}

/// 验收①：超时（白名单外，D8 安全默认律）→ 一次即败零重试。
#[tokio::test(flavor = "multi_thread")]
async fn 超时默认不重试() {
    let dir = temp_dir("timeout");
    let llm = Arc::new(FlakyLlm::new(vec![FlakyOutcome::Fail(LlmFailure::timeout("演练超时"))]));
    let root = assemble(&dir, llm.clone(), Some(serde_json::json!({}))).await;

    let err = {
        let svc = root
            .clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见");
        svc.run_turn("你好", tokio_util::sync::CancellationToken::new())
            .await
            .expect_err("超时默认应失败直出")
    };
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "零重试铁证");
    assert!(err.message().starts_with("llm/timeout"), "错误应保留归一化类别: {}", err.message());
}

/// 验收①：opt-in（retry_on_timeout=true）后超时可重试。
#[tokio::test(flavor = "multi_thread")]
async fn opt_in超时后重试() {
    let dir = temp_dir("optin");
    let llm = Arc::new(FlakyLlm::new(vec![
        FlakyOutcome::Fail(LlmFailure::timeout("首试超时")),
        answer_script("超时重试成功"),
    ]));
    let root = assemble(&dir, llm.clone(), Some(serde_json::json!({ "retry_on_timeout": true }))).await;

    let outcome = {
        let svc = root
            .clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见");
        svc.run_turn("你好", tokio_util::sync::CancellationToken::new()).await.expect("opt-in 后应重试成功")
    };
    assert_eq!(outcome.assistant_text, "超时重试成功");
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// 验收①：恒败（白名单内）→ 预算耗尽放弃（max_retries=2 → 共 3 次尝试）。
#[tokio::test(flavor = "multi_thread")]
async fn 预算耗尽后放弃() {
    let dir = temp_dir("budget");
    let llm = Arc::new(FlakyLlm::new(vec![connection_fail(), connection_fail(), connection_fail()]));
    let root = assemble(&dir, llm.clone(), Some(serde_json::json!({ "initial_delay_ms": 1, "max_delay_ms": 1 }))).await;

    let err = {
        let svc = root
            .clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见");
        svc.run_turn("你好", tokio_util::sync::CancellationToken::new())
            .await
            .expect_err("预算耗尽应失败")
    };
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 3, "最多 3 次尝试");
    assert!(err.message().starts_with("llm/connection"), "失败类别应保留: {}", err.message());
}

/// 验收总口径：策略卡缺席（禁用形态）→ 无重试直连，一次即败。
#[tokio::test(flavor = "multi_thread")]
async fn 策略卡缺席直连直出() {
    let dir = temp_dir("absent");
    let llm = Arc::new(FlakyLlm::new(vec![connection_fail()]));
    let root = assemble(&dir, llm.clone(), None).await;

    let err = {
        let svc = root
            .clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见（不依赖 retry，禁用不瘫痪主循环）");
        svc.run_turn("你好", tokio_util::sync::CancellationToken::new())
            .await
            .expect_err("无策略卡应直出")
    };
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "无重试");
    assert!(err.message().starts_with("llm/connection"));
}

/// 验收②：退避等待可被取消打断（取消即断链，铁律③）。
#[tokio::test(flavor = "multi_thread")]
async fn 退避等待中取消即断链() {
    let dir = temp_dir("cancel");
    let llm = Arc::new(FlakyLlm::new(vec![connection_fail(), answer_script("不应到达")]));
    let root = assemble(
        &dir,
        llm.clone(),
        Some(serde_json::json!({ "initial_delay_ms": 5000, "max_delay_ms": 5000 })),
    )
    .await;

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel2 = cancel.clone();
    let calls = llm.calls.clone();
    // 首次调用失败进入 5s 退避窗口后取消（窗口内断链，5s 内必返回）
    tokio::spawn(async move {
        while calls.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel2.cancel();
    });
    let started = Instant::now();
    let svc = root
        .clone()
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("AgentLoop 应可见");
    let err = svc
        .run_turn("你好", cancel)
        .await
        .expect_err("取消应使本轮失败收场");
    assert!(started.elapsed() < Duration::from_secs(4), "退避应被取消打断而非睡满 5s");
    assert!(err.message().contains("loop/cancelled"), "取消断链错误: {}", err.message());
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "取消链上不产生新尝试");
}

/// 铁律①：取消态直通——预取消的轮次不发起任何调用。
#[tokio::test(flavor = "multi_thread")]
async fn 预取消不发起调用() {
    let dir = temp_dir("precancel");
    let llm = Arc::new(FlakyLlm::new(vec![connection_fail()]));
    let root = assemble(&dir, llm.clone(), Some(serde_json::json!({}))).await;

    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let err = {
        let svc = root
            .clone()
            .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
            .expect("AgentLoop 应可见");
        svc.run_turn("你好", cancel).await.expect_err("预取消应失败")
    };
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 0, "取消链上零调用");
    assert!(err.message().contains("cancelled"));
}
