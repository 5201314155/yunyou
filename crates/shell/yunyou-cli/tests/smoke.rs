//! yunyou-cli 冒烟回归（五件套真装配，不依赖外部 LLM 端点）：
//! 1. §2.5 装配序列可跑通且 `dyn AgentLoop` 服务接线可见；
//! 2. 装配失败路径整树卸干净（无残留 fiber）；
//! 3. 就绪标记生命周期（创建/删除/Drop 兜底/落点约定）；
//! 4. 单任务模式端到端：LLM 指向必然快速失败的端点（对话失败被 REPL 吞并提示，
//!    不 panic、不残留），验证「装配→就绪→处理→优雅关闭」外壳全流程。
//!
//! 对话正确性（真实模型回复、工具执行、会话恢复）由 3A 的 live 测试与联调验收覆盖；
//! 本文件只验证宿主外壳：装配、服务接线、标记生命周期、优雅关闭。

use std::path::PathBuf;
use yunyou::ServiceId;
use yunyou_agent_loop::AgentLoop;
use yunyou_cli::{assemble, cli::Cli, ready, run};
use yunyou_config::DshConfig;

/// 装配序列：五件套真装配全部 Active，`dyn AgentLoop` 服务可从内核取到，
/// 随后整树优雅关闭（先消费者后提供者）。
#[tokio::test(flavor = "multi_thread")]
async fn 装配序列全部就绪且_agentloop_服务可见() {
    let home = temp_dir("yunyou-cli-smoke-assemble");
    let assembled = assemble::assemble(&DshConfig::default(), &home, &home.join("yunyou.toml"))
        .await
        .expect("五件套装配应成功（不联网）");
    // S2 起句柄为 Option（agent-loop 可被 [plugins] 段禁用）；Default 配置未禁用应在位
    let agent_loop = assembled.agent_loop.as_ref().expect("agentLoop 未禁用应在位");
    assert!(agent_loop.is_active(), "agentLoop 应 Active");
    // 接线验收：宿主能从内核取到 agentLoop provide 的对话服务
    let loop_service = assembled
        .root
        .get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>())
        .expect("agentLoop 服务应接线可见");
    let _ = loop_service;
    assembled.root.dispose().await.expect("应优雅关闭");
    agent_loop.wait_disposed().await;
    std::fs::remove_dir_all(&home).ok();
}

/// 失败路径：装载失败的组件链不留半装配状态（await_or_teardown 的整树卸载）。
#[tokio::test(flavor = "multi_thread")]
async fn 失败路径整树卸干净() {
    use std::sync::Arc;
    use yunyou::{ConfigValue, Context, CordisError, ErrorKind, FiberOutput, Plugin, Result};
    use yunyou_util::BoxFut;

    /// setup 必败的插件（模拟组件装载失败，如 LLM 端点不可达）。
    struct FailingPlugin;
    impl Plugin for FailingPlugin {
        fn name(&self) -> &str {
            "failing"
        }
        fn setup(
            self: Arc<Self>,
            _ctx: Context,
            _config: ConfigValue,
        ) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async { Err(CordisError::new(ErrorKind::Internal, "模拟装载失败")) })
        }
    }

    let root = Context::new();
    let handle = root.plugin(FailingPlugin, serde_json::json!({}));
    let err = handle.await_ready().await.expect_err("应装载失败");
    assert!(err.message().contains("模拟装载失败"));
    // 外壳失败路径的等价动作：整树卸载应成功（root 上无残留活跃 fiber）
    root.dispose().await.expect("失败后应能整树卸载");
    handle.wait_disposed().await;
}

/// 就绪标记：创建即存在、删除即消失，落点必须是 `<home>/run/ready-<pid>`。
#[tokio::test]
async fn 就绪标记落点符合约定() {
    let dir = temp_dir("yunyou-cli-smoke-ready");
    let mark = ready::ReadyMark::create(&dir).expect("应能创建");
    let expected = dir.join("run").join(format!("ready-{}", std::process::id()));
    assert_eq!(mark.path(), expected.as_path(), "落点必须是 <yunyou-home>/run/ready-<pid>");
    assert!(mark.path().exists());
    drop(mark);
    assert!(!expected.exists(), "Drop 兜底应删除");
    std::fs::remove_dir_all(&dir).ok();
}

/// 单任务模式端到端（run 总控全流程，不读 stdin）：
/// home 建立 → 装配 → 就绪标记 → 对话（LLM 指向必然快速失败端口，错误被吞并提示）
/// → 优雅关闭 → 无标记残留。
#[tokio::test(flavor = "multi_thread")]
async fn 单任务模式全流程跑通() {
    let home = temp_dir("yunyou-cli-smoke-run");
    std::fs::create_dir_all(&home).expect("应能建临时 home");
    // LLM 指向 127.0.0.1:1（保留端口，连接必然被拒且立即失败，测试不依赖外部服务）
    let config_path = home.join("yunyou.toml");
    std::fs::write(
        &config_path,
        "[llm]\nbase_url = \"http://127.0.0.1:1\"\ntimeout_secs = 2\n",
    )
    .expect("应能写配置");
    let command = Cli {
        config: Some(config_path),
        home: Some(home.clone()),
        task: Some("冒烟：一句话".to_string()),
        resume: None,        // task-9 新字段编译性补参：冒烟走单任务槽，恢复面不涉
        continue_recent: false,
        fork: false,
        verbose: false, // 任务 30-⑤ 新增字段的编译性补参，测试逻辑零改动（lead 备案）
        command: None,  // web 子命令（云幕）新增字段：冒烟走终端模式，编译性补参
    };
    // task-36⑤ 复核理由（lead 令 commit 信息同批写明）：本测试目的是单任务
    // 模式进程卫生（就绪标记零残留+优雅收尾），非 LLM 断言——CI 无 LLM 时
    // 对话失败是常态。失败退出码改非零后，此处改为双态断言：Ok=正常收尾；
    // Err=失败信令（脚本可判别）同样必须优雅收尾零残留。真机失败非零
    // 语义由 ptydrive 验收⑤e 覆盖（--task 端点不通 exit≠0 实测）。
    match run::run(command).await {
        Ok(()) => {}
        Err(err) => {
            // 失败路径同样走 run() 的统一收尾（mark.remove+dispose），此处
            // 断言其可发生且带人话——防「失败静默退出码 0」回归的反向锁
            let msg = err.to_string();
            assert!(
                msg.contains("对话失败") || msg.contains("已取消") || msg.contains("启动失败"),
                "失败信令人话形态异常：{msg}"
            );
        }
    }
    // 关闭后：run 目录里不应残留任何就绪标记
    let run_dir = home.join("run");
    if run_dir.exists() {
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&run_dir)
            .expect("run 目录应可读")
            .map(|e| e.expect("条目可读").path())
            .collect();
        assert!(leftovers.is_empty(), "就绪标记应已删除，残留：{leftovers:?}");
    }
    std::fs::remove_dir_all(&home).ok();
}

/// 测试临时目录（带 pid 防并行互踩）。
fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{tag}-{}", std::process::id()))
}
