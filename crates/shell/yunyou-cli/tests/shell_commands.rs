//! 壳命令集成测试（task-36④ 自 shell_commands.rs tests 外迁——源文件 400 行
//! 硬顶纪律；rebuild_session_to 提 pub 后自集成测试可达）。

use yunyou::{Context, ServiceId};
use yunyou_commands::CommandsService;
use yunyou_cli::shell_commands::{register_shell_commands, ActiveSession, SessionSlot, ShellCore};
use yunyou_config::DshConfig;
use yunyou_commands::{CommandDefinition, CommandInvocation, CommandKind, CommandOutput};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
mod tests {
    use super::*;

    /// 建测试装配：真 CommandsPlugin 装载 + 壳三命令注册（真装配自证）。
    async fn setup() -> (Context, Arc<ShellCore>, CancellationToken) {
        let root = Context::new();
        let handle = root.plugin(yunyou_commands::CommandsPlugin, serde_json::json!({}));
        handle.await_ready().await.expect("commands 插件应装载");
        let exit = CancellationToken::new();
        let slot = SessionSlot::new(ActiveSession {
            root: root.clone(),
            handler: Arc::new(|_line: &str, _cancel: CancellationToken| {
                Box::pin(async { Ok::<(), yunyou_cli::run::AppError>(()) })
            }),
            plugin_status: Vec::new(),
        });
        let shell = Arc::new(ShellCore {
            config: DshConfig::default(),
            home: std::env::temp_dir().join("yunyou-shell-cmd-test"),
            config_path: std::env::temp_dir().join("yunyou-shell-cmd-test/yunyou.toml"),
            session_dir: std::env::temp_dir().join("yunyou-shell-cmd-test/sessions"),
            slot,
            render_config: yunyou_cli::repl::RenderConfig {
                profile: yunyou_cli::render::Profile::NoColor,
                interactive: false,
                model: "test-model".into(),
                thinking: yunyou_cli::repl::ThinkingMode::Collapse,
            },
            current_session_path: std::sync::Mutex::new(
                std::env::temp_dir().join("yunyou-shell-cmd-test/sessions/default.jsonl"),
            ),
            exit: exit.clone(),
        });
        register_shell_commands(&root, shell.clone()).expect("壳命令注册应成功");
        (root, shell, exit)
    }

    /// 三命令经缝可解析（find），别名与精确名同源；同批重复注册 fail-loud。
    #[tokio::test(flavor = "multi_thread")]
    async fn 壳三命令经缝注册与解析() {
        let (root, _shell, _exit) = setup().await;
        let commands = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>()).unwrap();
        for (token, name) in [("help", "help"), ("帮助", "help"), ("quit", "quit"), ("新建", "new")] {
            let found = commands.find(token).unwrap_or_else(|| panic!("/{token} 应可解析"));
            assert_eq!(found.name, name, "token {token} 应解析到 {name}");
            assert!(found.kind == CommandKind::Action, "壳命令均为动作类");
        }
        // 重复注册 fail-loud（同名 Err，不静默覆盖）
        let err = commands
            .register(CommandDefinition {
                name: "help".into(),
                aliases: vec![],
                description: "重复".into(),
                kind: CommandKind::Action,
                accepts_args: false,
                // 原引 src 私有 help_handler（集成测试不可达）——同名重复注册
                // 只验缝的 fail-loud 语义，handler 本体用自足闭包即可
                handler: Arc::new(|_inv: CommandInvocation| Box::pin(async { Ok(CommandOutput::success(String::new())) })),
            })
            .expect_err("同名注册必须报错");
        assert!(err.message().contains("help"), "应点名命令名：{err}");
    }

    /// /help 文案从注册表生成：含三条壳命令（禁硬编码文案——目录变化自动跟随）。
    #[tokio::test(flavor = "multi_thread")]
    async fn help文案从注册表生成() {
        let (root, _shell, _exit) = setup().await;
        let commands = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>()).unwrap();
        // 注册一条额外命令验证「新增命令自动出现在帮助里」
        commands
            .register(CommandDefinition {
                name: "extra".into(),
                aliases: vec!["额外".into()],
                description: "验证命令".into(),
                kind: CommandKind::Action,
                accepts_args: false,
                handler: Arc::new(|_inv| Box::pin(async { Ok(CommandOutput::success(String::new())) })),
            })
            .unwrap();
        let handler = commands.handler_of("help").expect("/help 应有 handler");
        let output = handler(CommandInvocation { raw_input: "/help".into() }).await.unwrap();
        assert_eq!(output.kind, yunyou_commands::CommandResultKind::Success);
        for token in ["/help", "/quit", "/new", "/extra（额外）", "验证命令"] {
            assert!(output.text.contains(token), "帮助文案应含 {token}：\n{}", output.text);
        }
    }

    /// /quit 触发退出令牌（迁移语义：REPL 收 token 即退）。
    #[tokio::test(flavor = "multi_thread")]
    async fn quit触发退出信号() {
        let (_root, _shell, exit) = setup().await;
        let root = _root.clone();
        let commands = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>()).unwrap();
        let handler = commands.handler_of("quit").expect("/quit 应有 handler");
        assert!(!exit.is_cancelled());
        handler(CommandInvocation { raw_input: "/quit".into() }).await.unwrap();
        assert!(exit.is_cancelled(), "handler 执行后退出令牌应触发");
    }

    /// /new 重建链路（D3 回退路径真装配自证，复核 P2 补零覆盖）：
    /// 旧 teardown→新 assemble→换槽+壳命令重注册+退出令牌不触发（成功路径）。
    #[tokio::test(flavor = "multi_thread")]
    async fn new重建会话真装配换槽与壳命令重注册() {
        let (root, shell, exit) = setup().await;
        let old_root = root.clone();
        let output = shell.rebuild_session_to(None).await.expect("rebuild 应返回 Output");
        assert_eq!(
            output.kind,
            yunyou_commands::CommandResultKind::Success,
            "重建应成功: {}",
            output.text
        );
        assert!(!exit.is_cancelled(), "成功路径不触发退出令牌");
        // 槽已换：新 root 的注册表里有壳命令（重注册验证——新装配注册表为空，
        // 不重注册则 /new 后 /help 变「未知命令」）
        let cur = shell.slot.load();
        let commands = cur
            .root
            .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
            .expect("新装配 commands 服务应就绪");
        assert!(commands.find("help").is_some(), "/new 后 /help 应仍在注册表");
        // 旧装配已 teardown：旧 root 的服务查询应失败（fiber 已卸载不复活）
        assert!(
            old_root
                .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
                .is_err(),
            "旧装配 dispose 后服务应不可达"
        );
    }

    /// rebuild 失败路径（旧已拆新未立）：发退出信号收尾防半死状态——
    /// config_path 指向**目录**：启动迁移 read_to_string 得 IsADirectory 错
    ///（区别于「文件缺失=无操作」的容忍语义）→assemble 失败→task-58 先立后拆：
    /// 断言失败可逆（Error 文案+当前会话路径未变+退出令牌不触发——旧会话态保留，
    /// REPL 继续对话；v7 压测 ①d 旧语义「先拆后立强退」已废）。
    #[tokio::test(flavor = "multi_thread")]
    async fn rebuild失败可逆留原会话() {
        let (_root, shell, exit) = setup().await;
        let cfg_dir = shell.home.join("cfg-dir");
        std::fs::create_dir_all(&cfg_dir).expect("目录应建成");
        let before = shell.current_session_path.lock().expect("会话路径锁").clone();
        let bad = Arc::new(ShellCore {
            config: shell.config.clone(),
            home: shell.home.clone(),
            config_path: cfg_dir,
            session_dir: shell.session_dir.clone(),
            slot: shell.slot.clone(),
            render_config: shell.render_config.clone(),
            current_session_path: std::sync::Mutex::new(before.clone()),
            exit: exit.clone(),
        });
        let output = bad.rebuild_session_to(None).await.expect("rebuild 应返回 Output");
        assert_eq!(output.kind, yunyou_commands::CommandResultKind::Error, "坏路径应 Error");
        assert!(
            output.text.contains("当前会话未受影响"),
            "文案应声明可逆: {}",
            output.text
        );
        assert_eq!(
            bad.current_session_path.lock().expect("会话路径锁").clone(),
            before,
            "失败路径不得改当前会话路径（task-58 原子性）"
        );
        assert!(!exit.is_cancelled(), "失败可逆：退出令牌不得触发（旧语义已废）");
    }
}
