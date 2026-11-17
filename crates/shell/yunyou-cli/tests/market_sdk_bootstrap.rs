//! hello-sdk 自举全链集成测试（task-10 验收标尺）：
//! SDK 写 → **独立编译**（独立 target-dir，内核零重编译）→ 落盘插件目录 →
//! 内核发现层扫描（v2 manifest 不拒）→ ProxyPlugin spawn+握手+tools_list →
//! 代理工具注册 → 真调用成功。
//!
//! spawn 形态=Direct（宿主侧文档化测试缝：proxy_plugin::SpawnForm；生产恒
//! Sandboxed——OS 级沙箱链由 task-24 forked 探针独立验证，本测试不重复覆盖）。

// 测试面豁免（K-14 观察期口径；expect/unwrap 失败=断言红，语义等价）
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::process::Command;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use yunyou::{Context, ServiceId};
use yunyou_tools::contract::ToolExec;

/// 独立编译 hello-sdk（独立 target-dir：不经 workspace 共享产物=「内核零重编译」
/// 的机制证据；产物拷贝进插件目录供 runtime.entry 相对引用）。
fn build_hello_sdk_independent(home: &std::path::Path) -> std::path::PathBuf {
    let manifest = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../sdk/examples/hello-sdk/Cargo.toml"
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
        "hello-sdk 独立编译失败（SDK 面 = 插件作者唯一依赖的验收根）:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let binary = target_dir.join("release/hello-sdk");
    assert!(binary.is_file(), "产物应存在：{}", binary.display());
    binary
}

/// 落盘插件目录（install 归类语义：plugins/<category>/<id>/）。
fn install_hello_sdk(home: &std::path::Path, binary: &std::path::Path) -> std::path::PathBuf {
    let plugin_dir = home.join("plugins/tools/hello-sdk");
    std::fs::create_dir_all(&plugin_dir).expect("插件目录应可建");
    std::fs::copy(binary, plugin_dir.join("hello-sdk")).expect("产物拷贝应成功");
    std::fs::write(
        plugin_dir.join("manifest.toml"),
        format!(
            "id = \"hello-sdk\"\nversion = \"0.1.0\"\nformat = 2\nlevel = \"market\"\ncontracts = [\"tools.v1\"]\ncategory = \"tools\"\nauto_enable = true\n\n[runtime]\ntype = \"subprocess\"\nentry = [\"hello-sdk\"]\nrequest_timeout_ms = 30000\n"
        ),
    )
    .expect("manifest 应可写");
    plugin_dir
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_sdk自举全链_独立编译到真调用() {
    let home = std::env::temp_dir().join(format!("hello-sdk-boot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("临时 home 应可建");

    // ① SDK 写的插件 → 独立编译（内核零重编译）
    let binary = build_hello_sdk_independent(&home);
    // ② install 语义落盘（归类 plugins/<category>/<id>/）
    let plugin_dir = install_hello_sdk(&home, &binary);
    // ③ 内核发现层扫描：v2 manifest 不拒、可发现（L1 发现层 + v2 字段兼容）
    let discovered =
        yunyou_config::discover_plugins(&home.join("plugins")).expect("v2 manifest 应过发现层");
    assert!(
        discovered.iter().any(|d| d.id == "hello-sdk"),
        "发现层应找到 hello-sdk，实际 ids={:?}",
        discovered.iter().map(|d| d.id.clone()).collect::<Vec<_>>()
    );

    // ④ 装配最小前置（commands+tools=ProxyPlugin 的 inject 依赖）→ ProxyPlugin(Direct)
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
        },
        serde_json::Value::Null,
    );
    // ⑤ spawn+握手+tools_list+代理注册（await_ready 至稳定；失败即自举链断）
    handle
        .await_ready()
        .await
        .expect("hello-sdk 代理装载应成功（spawn→握手→清单→注册全链）");
    assert!(handle.is_active(), "代理 fiber 应 Active，实际 {:?}", handle.state());

    // ⑥ 模型面可见 + 真调用成功
    let registry = root
        .get::<dyn yunyou_tools::RegistryService>(ServiceId::of::<dyn yunyou_tools::RegistryService>())
        .expect("注册表应可见");
    let names: Vec<String> = registry.schemas().into_iter().map(|s| s.name).collect();
    assert!(names.iter().any(|n| n == "hello"), "模型面应含 hello 工具，实际 {names:?}");
    assert!(names.iter().any(|n| n == "add"), "模型面应含 add 工具，实际 {names:?}");
    eprintln!("[boot-diag] 调用前 fiber 状态={:?}，工具数={}", handle.state(), names.len());
    let tool = registry.get("hello").expect("hello 应可取");
    let outcome = tool
        .execute(
            serde_json::json!({ "name": "小云" }),
            ToolExec { call_id: "it".into(), cancel: CancellationToken::new(), workspace: None },
        )
        .await;
    match outcome {
        yunyou_tools::contract::ToolOutcome::Ok(value) => {
            let greeting = value["greeting"].as_str().unwrap_or_default();
            assert!(greeting.contains("小云"), "真调用应返回含名字的问候，实际 {value}");
        }
        yunyou_tools::contract::ToolOutcome::Err(err) => {
            panic!("真调用应成功，实际失败 {err:?}");
        }
    }
    // 收尾：整树 dispose（含 shutdown+killpg，进程不残留）
    let _ = root.dispose().await;
    let _ = std::fs::remove_dir_all(&home);
}

/// 发现层 v2 兼容反例锚：v1 声明（无 format/runtime）行为不变 + v2 缺 runtime 段
/// 不在发现层炸（权威校验在 market 分支）。
#[test]
fn 发现层v1v2共存() {
    let home = std::env::temp_dir().join(format!("hello-sdk-disc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("plugins/v1a")).unwrap();
    std::fs::write(
        home.join("plugins/v1a/manifest.toml"),
        "id = \"v1a\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(home.join("plugins/v2b")).unwrap();
    std::fs::write(
        home.join("plugins/v2b/manifest.toml"),
        "id = \"v2b\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\nformat = 2\nlevel = \"market\"\n[runtime]\ntype = \"subprocess\"\nentry = [\"v2b\"]\n",
    )
    .unwrap();
    let discovered = yunyou_config::discover_plugins(&home.join("plugins"))
        .expect("v1+v2 共存不应拒装");
    let ids: Vec<String> = discovered.iter().map(|d| d.id.clone()).collect();
    assert!(ids.contains(&"v1a".to_string()) && ids.contains(&"v2b".to_string()), "实际 {ids:?}");
    let _ = std::fs::remove_dir_all(&home);
}

// Arc 引用保持（抑制未用导入告警面）：enforcer 槽位在正式装配为 Some(Arc)
#[allow(unused)]
fn _arc_shape(_: Arc<()>) {}
