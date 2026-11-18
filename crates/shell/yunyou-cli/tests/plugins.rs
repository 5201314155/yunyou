//! S2 插件化装配验收测试（方案 §6.1 S2 行 + §5.3 验收 1，业主封板口径前置）：
//! 1. 存量五段配置不破坏（迁移测试：旧 yunyou.toml 零改动可用）；
//! 2. 禁用任一非核心插件安全启动（消费者 Pending=诊断「依赖未就绪」而非崩溃，
//!    注册表条目无残尸）；
//! 3. 重新启用自动恢复（enable 拨回 → 重启装配即 ACTIVE，epoch 机制接线）；
//! 4. 状态表渲染（id/version/enabled/state 四列，纯函数断言见 status.rs 单测，
//!    此处验装配产物的 enabled/state 语义与注册表序）。
//!
//! 全部不依赖外部 LLM 端点（对话正确性归 live 测试，此处只验宿主装配面）。

use std::path::PathBuf;
use yunyou::ServiceId;
use yunyou_agent_loop::AgentLoop;
use yunyou_cli::assemble;
use yunyou_config::DshConfig;
use yunyou_tools::RegistryService;

/// 存量五段配置（S2 之前的 yunyou.toml 形态：无 [plugins] 段）——零改动可用。
const LEGACY_TOML: &str = "\
[llm]\n\
base_url = \"http://127.0.0.1:1\"\n\
timeout_secs = 2\n\
\n\
[session]\n\
\n\
[system_prompt]\n\
\n\
[tools]\n\
root_dir = \".\"\n\
\n\
[agent_loop]\n";

/// 存量配置 + [plugins] 禁用 tools（非核心插件；agent-loop 消费其 RegistryService）。
const DISABLE_TOOLS_TOML: &str = "\
[llm]\n\
base_url = \"http://127.0.0.1:1\"\n\
\n\
[plugins.yunyou-tools]\n\
enable = false\n";

/// 存量配置 + [plugins] 显式重新启用 tools（与禁用版同形态，仅开关拨回）。
const REENABLE_TOOLS_TOML: &str = "\
[llm]\n\
base_url = \"http://127.0.0.1:1\"\n\
\n\
[plugins.yunyou-tools]\n\
enable = true\n";

/// 迁移测试（验收：存量五段配置不破坏）：旧配置零改动加载并装配成功，
/// 五件套全 Active，agent-loop 服务接线可见。
#[tokio::test(flavor = "multi_thread")]
async fn 存量五段配置零改动可用() {
    let config: DshConfig = yunyou_config::load_from_str(LEGACY_TOML).expect("旧配置应零改动加载");
    assert!(config.plugins.entries.is_empty(), "无 [plugins] 段应落空表");
    let home = temp_dir("yunyou-cli-plugins-legacy");
    let config_path = home.join("yunyou.toml");
    let assembled = assemble::assemble(&config, &home, &config_path).await.expect("存量配置应装配成功");
    let agent_loop = assembled.agent_loop.as_ref().expect("未禁用应在位");
    assert!(agent_loop.is_active(), "agentLoop 应 Active");
    assert!(
        assembled.root.get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>()).is_ok(),
        "agentLoop 服务应接线可见"
    );
    assembled.root.dispose().await.expect("应优雅关闭");
    agent_loop.wait_disposed().await;
    std::fs::remove_dir_all(&home).ok();
}

/// 禁用非核心插件（§5.3 验收 1 + §6.1 S2 验收）：安全启动不崩不挂死；
/// 消费者 agent-loop 保持 Pending（await_ready 稳定态立即返回）；
/// 被禁插件未注册=注册表服务无残尸（§5.3 验收 3 启动态）。
#[tokio::test(flavor = "multi_thread")]
async fn 禁用tools安全启动_消费者pending不崩() {
    let config: DshConfig =
        yunyou_config::load_from_str(DISABLE_TOOLS_TOML).expect("[plugins] 禁用配置应加载");
    assert_eq!(
        config.plugins.entries.get("yunyou-tools").and_then(|e| e.enable),
        Some(false),
        "禁用开关应入表"
    );
    let home = temp_dir("yunyou-cli-plugins-disable");
    // 不崩不挂死：Pending 属稳定态，装配应正常返回 Ok（消费者诊断而非失败）
    let config_path = home.join("yunyou.toml");
    let assembled = assemble::assemble(&config, &home, &config_path)
        .await
        .expect("禁用非核心插件应安全启动（Pending 诊断而非崩溃）");
    let agent_loop = assembled.agent_loop.as_ref().expect("agent-loop 未被禁用，句柄应在位");
    assert!(!agent_loop.is_active(), "消费者应保持 Pending（依赖未就绪）");
    assert_eq!(agent_loop.state().as_str(), "pending", "state 应可观测为 pending");
    // 无残尸（§5.3 验收 3 启动态）：被禁插件的注册表服务不存在
    assert!(
        assembled
            .root
            .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
            .is_err(),
        "被禁插件的服务不得存在（开关拨下无残尸）"
    );
    assembled.root.dispose().await.expect("应优雅关闭");
    agent_loop.wait_disposed().await;
    std::fs::remove_dir_all(&home).ok();
}

/// 重新启用自动恢复（验收口径）：同一 [plugins] 形态、开关拨回 true，
/// 重启装配即全链 ACTIVE（epoch 机制自动接线）。
#[tokio::test(flavor = "multi_thread")]
async fn 重新启用自动恢复全链active() {
    let config: DshConfig =
        yunyou_config::load_from_str(REENABLE_TOOLS_TOML).expect("重新启用配置应加载");
    let home = temp_dir("yunyou-cli-plugins-reenable");
    let config_path = home.join("yunyou.toml");
    let assembled = assemble::assemble(&config, &home, &config_path)
        .await
        .expect("重新启用后应装配成功");
    let agent_loop = assembled.agent_loop.as_ref().expect("agent-loop 应在位");
    assert!(agent_loop.is_active(), "重新启用后 agent-loop 应恢复 Active");
    assert!(
        assembled
            .root
            .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
            .is_ok(),
        "重新启用后注册表服务应恢复接线"
    );
    assembled.root.dispose().await.expect("应优雅关闭");
    agent_loop.wait_disposed().await;
    std::fs::remove_dir_all(&home).ok();
}

/// 禁用另一非核心插件（llm-openai，核心依赖链头部）：同样安全启动，
/// 消费者 Pending——「任一非核心插件」语义的第二个样本。
#[tokio::test(flavor = "multi_thread")]
async fn 禁用llm安全启动_主循环pending() {
    let config: DshConfig = yunyou_config::load_from_str(
        "[llm]\nbase_url = \"http://127.0.0.1:1\"\n\n[plugins.yunyou-llm-openai]\nenable = false\n",
    )
    .expect("禁用 llm 配置应加载");
    let home = temp_dir("yunyou-cli-plugins-disable-llm");
    let config_path = home.join("yunyou.toml");
    let assembled = assemble::assemble(&config, &home, &config_path)
        .await
        .expect("禁用 llm 应安全启动");
    let agent_loop = assembled.agent_loop.as_ref().expect("agent-loop 应在位");
    assert!(!agent_loop.is_active(), "消费者 agent-loop 应保持 Pending");
    assembled.root.dispose().await.expect("应优雅关闭");
    agent_loop.wait_disposed().await;
    std::fs::remove_dir_all(&home).ok();
}

/// [plugins] 缺省回落语义（方案 §2.4 第 2 步）：只写插件键不写 enable，
/// 装配按 manifest.auto_enable（true）处理——「未知 id 宽容」与「缺省回落」的边界。
#[tokio::test(flavor = "multi_thread")]
async fn plugins_键缺省enable回落auto_enable() {
    let config: DshConfig = yunyou_config::load_from_str(
        "[llm]\nbase_url = \"http://127.0.0.1:1\"\n\n[plugins.yunyou-tools]\n",
    )
    .expect("仅插件键的配置应加载");
    let entry = config.plugins.entries.get("yunyou-tools").expect("键应入表");
    assert_eq!(entry.enable, None, "未写 enable 应落 None");
    let home = temp_dir("yunyou-cli-plugins-default-enable");
    let config_path = home.join("yunyou.toml");
    let assembled = assemble::assemble(&config, &home, &config_path).await.expect("缺省回落应装配成功");
    let agent_loop = assembled.agent_loop.as_ref().expect("agent-loop 应在位");
    assert!(agent_loop.is_active(), "auto_enable 缺省应全链 Active");
    assembled.root.dispose().await.expect("应优雅关闭");
    agent_loop.wait_disposed().await;
    std::fs::remove_dir_all(&home).ok();
}

/// S5 迁移→解析全链（离线等价，lead 裁决④）：存量明文 key 首次装配 = 入托管
/// 存储＋配置文件定点改写为引用（迁移日志不落值）；第二次装配（重启）= 宿主
/// store 解析 `$REF` 为真值透传 llm——迁移与解析面同批生效，无 401 断裂窗口。
#[tokio::test(flavor = "multi_thread")]
async fn 明文key迁移后重启_引用解析全链闭合() {
    let home = temp_dir("yunyou-cli-plugins-migrate");
    std::fs::create_dir_all(&home).expect("应能建临时 home");
    let config_path = home.join("yunyou.toml");
    std::fs::write(
        &config_path,
        "# 注释保留\n[llm]\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"sk-明文-存量\"\n",
    )
    .expect("应能写存量配置");
    // 第一次装配：迁移发生（文件改写+托管入库），本次启动内存段值仍明文直通
    let config: DshConfig = yunyou_config::load_from_str(
        &std::fs::read_to_string(&config_path).expect("应能读回"),
    )
    .expect("存量配置应加载");
    let first = assemble::assemble(&config, &home, &config_path)
        .await
        .expect("首次装配应成功（迁移+明文直通）");
    let rewritten = std::fs::read_to_string(&config_path).expect("应能读回");
    assert!(
        rewritten.contains("api_key = \"$YUNYOU_LLM_API_KEY\""),
        "配置文件应已定点改写为引用：{rewritten}"
    );
    assert!(rewritten.contains("# 注释保留"), "注释应字节级保留");
    let store_text = std::fs::read_to_string(home.join("credentials.toml")).expect("托管存储应存在");
    assert!(store_text.contains("YUNYOU_LLM_API_KEY"), "托管存储应含迁移槽");
    assert!(!store_text.is_empty() && store_text.contains("sk-明文-存量"), "托管存储应持真值");
    first.root.dispose().await.expect("应优雅关闭");
    // 第二次装配（重启）：文件已是 $REF → 宿主 store 解析为真值（装配不报
    // 「凭据引用未配置」即解析成功；解析面单测锁值语义，此处锁全链贯通）
    let config2: DshConfig =
        yunyou_config::load_from_str(&rewritten).expect("改写后配置应加载");
    assert_eq!(
        config2.llm.api_key, "$YUNYOU_LLM_API_KEY",
        "重启后配置面只持引用（key 不再明文进 yunyou.toml，S5 验收）"
    );
    let second = assemble::assemble(&config2, &home, &config_path)
        .await
        .expect("重启装配应成功（引用解析闭合，无 401 断裂）");
    second.root.dispose().await.expect("应优雅关闭");
    std::fs::remove_dir_all(&home).ok();
}

/// S5 引用缺失 fail-loud（裁决②）：api_key 为引用但托管存储与环境均无值 →
/// 装配失败并明确报「凭据引用未配置」——不静默空串（静默＝请求 401 难排查）。
#[tokio::test(flavor = "multi_thread")]
async fn 引用未配置_fail_loud明确报错() {
    let home = temp_dir("yunyou-cli-plugins-ref-missing");
    let config_path = home.join("yunyou.toml");
    let config: DshConfig = yunyou_config::load_from_str(
        "[llm]\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"$NO_SUCH_REF_XYZ\"\n",
    )
    .expect("引用配置应加载");
    let err = match assemble::assemble(&config, &home, &config_path).await {
        Err(err) => err,
        Ok(assembled) => {
            let _ = assembled.root.dispose().await;
            panic!("未配置引用应装配失败（fail-loud，不静默空串）")
        }
    };
    assert!(
        err.to_string().contains("未找到 API key")
            && err.to_string().contains("$NO_SUCH_REF_XYZ"),
        "报错应点名引用：{err}"
    );
    std::fs::remove_dir_all(&home).ok();
}

/// S5 store_path 自定义：credentials 段显式路径生效（宿主持有的存储与迁移
/// 都落该路径，不写死 home 默认值）。
#[tokio::test(flavor = "multi_thread")]
async fn credentials_自定义store_path生效() {
    let home = temp_dir("yunyou-cli-plugins-store-path");
    let config_path = home.join("yunyou.toml");
    std::fs::create_dir_all(&home).expect("应能建临时 home");
    let custom = home.join("custom-cred").join("store.toml");
    let text = format!(
        "[llm]\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"sk-明文-x\"\n\n[credentials]\nstore_path = {:?}\n",
        custom.display().to_string()
    );
    std::fs::write(&config_path, &text).expect("应能写配置");
    let config: DshConfig = yunyou_config::load_from_str(&text)
        .expect("自定义路径配置应加载");
    let assembled = assemble::assemble(&config, &home, &config_path)
        .await
        .expect("装配应成功");
    // 迁移走自定义路径：明文 key 入托管 → store 文件落自定义位置
    assert!(custom.exists(), "托管存储应落自定义路径（含父目录创建）");
    assembled.root.dispose().await.expect("应优雅关闭");
    std::fs::remove_dir_all(&home).ok();
}

/// 装载器 L1（K2）：plugins/ 目录 manifest 文件命中 bundled → auto_enable 覆盖
/// 代码缺省 + 来源标 discovered（任务16 自举标尺装配侧断言：零 Rust 改动，
/// 丢目录 → 重启生效 → 可开关；「拨下无残尸」复验）。
#[tokio::test(flavor = "multi_thread")]
async fn 装载器发现命中_文件auto_enable覆盖与来源标注() {
    let home = temp_dir("yunyou-cli-l1-match");
    // 验收标尺形态：目录名与 id 可不同（id 以文件内字段为准），命中 bundled 实现
    let hello = home.join("plugins").join("hello");
    std::fs::create_dir_all(&hello).unwrap();
    std::fs::write(
        hello.join("manifest.toml"),
        "id = \"yunyou-tools\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\nauto_enable = false\n",
    )
    .unwrap();
    let config: DshConfig =
        yunyou_config::load_from_str("[llm]\nbase_url = \"http://127.0.0.1:1\"\n").unwrap();
    let config_path = home.join("yunyou.toml");
    let assembled =
        assemble::assemble(&config, &home, &config_path).await.expect("发现命中应装配成功");
    let tools =
        assembled.plugin_status.iter().find(|r| r.id == "yunyou-tools").expect("tools 行应在");
    assert!(!tools.enabled, "文件 auto_enable=false 应覆盖代码缺省 true");
    assert_eq!(tools.source, "discovered", "命中声明者来源应标 discovered");
    assert_eq!(tools.state, "disabled", "禁用=不注册无残尸");
    // 无残尸（自举标尺「拨下无残尸」）
    assert!(assembled
        .root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .is_err());
    assembled.root.dispose().await.expect("应优雅关闭");
    std::fs::remove_dir_all(&home).ok();
}

/// enable 三级优先级（K2）：`[plugins.<id>]` 显式值 > 文件 auto_enable > 代码
/// auto_enable——配置段仍最高优先（设计 §3 原文锁死）。
#[tokio::test(flavor = "multi_thread")]
async fn enable三级优先级_显式段最高() {
    let home = temp_dir("yunyou-cli-l1-priority");
    let hello = home.join("plugins").join("hello");
    std::fs::create_dir_all(&hello).unwrap();
    std::fs::write(
        hello.join("manifest.toml"),
        "id = \"yunyou-tools\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\nauto_enable = false\n",
    )
    .unwrap();
    let config: DshConfig = yunyou_config::load_from_str(
        "[llm]\nbase_url = \"http://127.0.0.1:1\"\n\n[plugins.yunyou-tools]\nenable = true\n",
    )
    .unwrap();
    let config_path = home.join("yunyou.toml");
    let assembled = assemble::assemble(&config, &home, &config_path).await.expect("应装配成功");
    let tools =
        assembled.plugin_status.iter().find(|r| r.id == "yunyou-tools").expect("tools 行应在");
    assert!(tools.enabled, "[plugins] 显式 true 应压过文件 auto_enable=false");
    assert_eq!(tools.source, "discovered");
    assembled.root.dispose().await.expect("应优雅关闭");
    std::fs::remove_dir_all(&home).ok();
}

/// 装载器 L1 未命中（真外部包）：明确诊断不静默、不拒装（能力边界非装配失败：
/// fail-loud 适用对象是已启用插件的装载失败，这里没有可装载的实现），
/// 状态行 state=unsupported 入观测面（K4 L2 接缝预留，设计 §4）。
#[tokio::test(flavor = "multi_thread")]
async fn 装载器未命中_unsupported行不拒装() {
    let home = temp_dir("yunyou-cli-l1-unmatched");
    let ghost = home.join("plugins").join("ghost");
    std::fs::create_dir_all(&ghost).unwrap();
    std::fs::write(
        ghost.join("manifest.toml"),
        "id = \"external-pkg\"\nversion = \"1.0.0\"\ncontracts = [\"tools.v1\"]\n",
    )
    .unwrap();
    let config: DshConfig =
        yunyou_config::load_from_str("[llm]\nbase_url = \"http://127.0.0.1:1\"\n").unwrap();
    let config_path = home.join("yunyou.toml");
    let assembled = assemble::assemble(&config, &home, &config_path).await.expect("未命中不拒装");
    let ghost = assembled
        .plugin_status
        .iter()
        .find(|r| r.id == "external-pkg")
        .expect("未命中行应入观测面");
    assert_eq!(ghost.state, "unsupported");
    assert_eq!(ghost.source, "discovered");
    assert!(!ghost.enabled, "无实现可启用，恒 false 不误导");
    // bundled 其余插件不受影响
    assert!(assembled
        .plugin_status
        .iter()
        .any(|r| r.id == "yunyou-llm-openai" && r.source == "bundled"));
    assembled.root.dispose().await.expect("应优雅关闭");
    std::fs::remove_dir_all(&home).ok();
}

/// 装载器 L1 坏声明 fail-loud：解析失败聚合带路径逐条报，拒装不静默。
#[tokio::test(flavor = "multi_thread")]
async fn 装载器坏声明_fail_loud聚合报() {
    let home = temp_dir("yunyou-cli-l1-bad");
    let bad = home.join("plugins").join("broken");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("manifest.toml"), "version = \"0.1.0\"\n").unwrap(); // 缺 id
    let config: DshConfig =
        yunyou_config::load_from_str("[llm]\nbase_url = \"http://127.0.0.1:1\"\n").unwrap();
    let config_path = home.join("yunyou.toml");
    let err = match assemble::assemble(&config, &home, &config_path).await {
        Err(err) => err,
        Ok(assembled) => {
            let _ = assembled.root.dispose().await;
            panic!("坏声明应拒装（fail-loud 不静默）")
        }
    };
    let text = err.to_string();
    assert!(text.contains("manifest.toml") && text.contains("TOML 解析失败"), "应带路径逐条报：{text}");
    std::fs::remove_dir_all(&home).ok();
}

/// 测试临时目录（带 pid 防并行互踩）。
fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{tag}-{}", std::process::id()))
}

/// task-24 禁用矩阵（传递 Pending 死锁回归锁）：禁用**任一** bundled 插件→
/// 装配秒级完成（15s 护栏）+禁用行 disabled+无 hang——传递闭包预检的穷举面。
#[tokio::test(flavor = "multi_thread")]
async fn 禁用矩阵_任一bundled禁用装配秒级() {
    for id in ["yunyou-llm-openai", "yunyou-session", "yunyou-system-prompt", "yunyou-commands", "yunyou-sandbox", "yunyou-tools", "yunyou-retry", "yunyou-permission", "yunyou-agent-loop", "yunyou-tool-archive", "yunyou-credentials"] {
        let toml = format!("[plugins.{id}]\nenable = false\n");
        let config: DshConfig = yunyou_config::load_from_str(&toml).expect("配置应加载");
        let home = temp_dir("yunyou-cli-disable-matrix");
        let config_path = home.join("yunyou.toml");
        let assembled = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            assemble::assemble(&config, &home, &config_path),
        )
        .await
        .expect_or_hang(&format!("禁用 {id} 装配超时（传递闭包预检未短路）"))
        .expect("禁用非核心插件应安全启动");
        let row = assembled
            .plugin_status
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("禁用 {id} 的状态行应在"));
        assert_eq!(row.state, "disabled", "禁用 {id} 应标 disabled");
        assembled.root.dispose().await.expect("应优雅关闭");
        std::fs::remove_dir_all(&home).ok();
    }
}

trait ExpectNoHang<T> {
    fn expect_or_hang(self, msg: &str) -> T;
}
impl<T> ExpectNoHang<T> for Result<T, tokio::time::error::Elapsed> {
    fn expect_or_hang(self, msg: &str) -> T {
        match self {
            Ok(v) => v,
            Err(_) => panic!("{msg}"),
        }
    }
}
