//! scheduler.v1 真投递端到端（task-14 续，真机验收的进程内等价锚）：
//! scheduler-sample 独立编译→spawn→握手→模型面调 schedule_create（at=now+1.5s）→
//! 到期插件经 DeliverMessage 投递→宿主回调（测试闭包=REPL 注入队列等价面）→
//! DeliverAck 回执链闭环。
//!
//! 真机验收（lead 跑）=REPL 注入臂合成用户输入→模型继续干活；本测试锁
//! 「宿主回调被调用且收到注入文本」段（REPL 段=lineedit 注入臂，手工真机）。

use std::process::Command;
use yunyou::registry::Plugin;
use yunyou::ServiceId;
use yunyou_tools::contract::{ToolDefinition, ToolExec};
use yunyou_tools::RegistryService;
use tokio_util::sync::CancellationToken;
use yunyou::{Context, FiberState};

fn build_scheduler_sample(home: &std::path::Path) -> std::path::PathBuf {
    let manifest = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../sdk/examples/scheduler/Cargo.toml"
    );
    let target_dir = home.join("sdk-target");
    let output = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--manifest-path",
            manifest,
            "--target-dir",
            target_dir.to_str().expect("target 路径合法"),
        ])
        .output()
        .expect("cargo 应可执行");
    assert!(
        output.status.success(),
        "scheduler-sample 独立编译失败:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let binary = target_dir.join("release/scheduler-sample");
    assert!(binary.is_file(), "产物应存在：{}", binary.display());
    binary
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduler真投递_创建到到期回调全链() {
    let home = std::env::temp_dir().join(format!("sched-deliver-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("临时 home 应可建");
    let binary = build_scheduler_sample(&home);

    let plugin_dir = home.join("plugins/workflows/scheduler");
    std::fs::create_dir_all(&plugin_dir).expect("插件目录应可建");
    std::fs::copy(&binary, plugin_dir.join("scheduler-sample")).expect("产物拷贝应成功");
    std::fs::write(
        plugin_dir.join("manifest.toml"),
        "id = \"scheduler\"\nversion = \"0.1.0\"\nformat = 2\nlevel = \"market\"\ncontracts = [\"tools.v1\"]\ncategory = \"workflows\"\nauto_enable = true\n\n[runtime]\ntype = \"subprocess\"\nentry = [\"scheduler-sample\"]\nrequest_timeout_ms = 30000\n",
    )
    .expect("manifest 应可写");

    // 投递面（REPL 注入队列的测试等价面）：闭包→mpsc
    let (deliver_tx, mut deliver_rx) = tokio::sync::mpsc::channel::<String>(8);
    let deliver: Option<std::sync::Arc<
        dyn Fn(Option<String>, String) -> Result<String, String> + Send + Sync,
    >> = Some(std::sync::Arc::new(move |_session, text| {
        deliver_tx
            .try_send(text)
            .map(|_| "已入注入队列".to_string())
            .map_err(|err| format!("投递失败：{err}"))
    }));

    let root = Context::new();
    root.plugin(yunyou_commands::CommandsPlugin, serde_json::json!({}))
        .await_ready()
        .await
        .expect("commands 应装载");
    root.plugin(yunyou_tools::ToolsPlugin, serde_json::json!({}))
        .await_ready()
        .await
        .expect("tools 应装载");
    let manifest_text = std::fs::read_to_string(plugin_dir.join("manifest.toml")).unwrap();
    let manifest_v2 = yunyou_cli::market_manifest::parse_and_validate(&manifest_text, &plugin_dir)
        .expect("manifest 五类校验应过");
    let handle = root.plugin(
        yunyou_cli::proxy_plugin::ProxyPlugin {
            manifest_v2,
            plugin_dir: plugin_dir.clone(),
            enforcer: None,
            spawn_form: yunyou_cli::proxy_plugin::SpawnForm::Direct,
            deliver,
        },
        serde_json::Value::Null,
    );
    handle
        .await_ready()
        .await
        .expect("scheduler 代理装载应成功");
    assert!(handle.state() == FiberState::Active);

    // 模型面调 schedule_create：at=now+1.5s
    let registry = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("注册表应可见");
    let tool = registry.get("schedule_create").expect("schedule_create 应在模型面");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let outcome = tool
        .execute(
            serde_json::json!({
                "at_ms": now_ms + 1_500,
                "message": "该喝水了",
                "durable": false
            }),
            ToolExec { call_id: "it".into(), cancel: CancellationToken::new(), workspace: None },
        )
        .await;
    let ok = match &outcome {
        yunyou_tools::contract::ToolOutcome::Ok(v) => v.clone(),
        other => panic!("schedule_create 应成功，实际 {other:?}"),
    };
    assert!(
        ok.get("next_fire_ms").is_some(),
        "回显即确认：应含下一次触发时刻，实际 {ok}"
    );

    // 到期（1.5s）→ DeliverMessage→回调→注入队列（等 10s 护栏）
    let received =
        tokio::time::timeout(std::time::Duration::from_secs(10), deliver_rx.recv()).await;
    let text = received
        .expect_or_panic()
        .expect("注入队列应收到消息");
    assert!(
        text.contains("定时") && text.contains("该喝水了"),
        "注入消息应带宿主前缀+原文，实际：{text}"
    );

    // 收尾：整树 dispose（shutdown+killpg，进程不残留）
    let _ = root.dispose().await;
    let _ = std::fs::remove_dir_all(&home);
}

trait ExpectOrPanic<T> {
    fn expect_or_panic(self) -> T;
}
impl<T> ExpectOrPanic<T> for Result<T, tokio::time::error::Elapsed> {
    fn expect_or_panic(self) -> T {
        match self {
            Ok(v) => v,
            Err(_) => panic!("到期投递超时（10s 护栏）——真投递链未闭环"),
        }
    }
}
