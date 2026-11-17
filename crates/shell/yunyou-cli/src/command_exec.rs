//! 命令执行链与 D2 落盘（任务29 C2-repl，CLI 壳面；自 repl.rs 拆出——分发循环
//! 与命令执行两职责分行数红线，拆分不改逻辑）。
//!
//! run 先于 handler 落**当前**会话、done **重新取**当前会话落盘：/new 若在
//! handler 内换了会话槽（会话切换命令形态），done 落新文件，run/done 跨文件
//! 以 `command_id` 配对（tool_call/tool_result 同纪律）。未知命令 stderr 提示
//! 且不落任何日志——官方语义：语法/未知名未进 handler 即不落盘。

use crate::shell_commands::SessionSlot;
use std::sync::Arc;
use yunyou::{Context, ServiceId};
use yunyou_commands::{
    CommandInvocation, CommandKind, CommandOutput, CommandResultKind, CommandsService,
};
use yunyou_session::events::CommandOutcome;
use yunyou_session::{SessionEvent, SessionStore};

/// 命令执行结算（REPL 分发层的处置依据）。
pub(crate) enum CommandSettled {
    /// 动作类已完成（text 已上屏或静默）。
    Action,
    /// 展开类：回填文本（下一轮 read_line 的 initial，用户增删改后确认发送）。
    Expand(String),
}

/// 命令执行链+D2 落盘（交接 §六-3）：结果处置（交接 §六-4）Error→stderr；
/// 动作类 Success→上屏（空则静默）；展开类 Success→回填输入框待用户确认。
pub(crate) async fn execute_command(slot: &Arc<SessionSlot>, raw: &str) -> CommandSettled {
    // token 解析：去斜杠后取首空白段（split_whitespace 自忽略首尾空白；
    // `/help 附加参数` → token=help）
    let token = raw[1..].split_whitespace().next().unwrap_or("");
    let session = slot.load();
    let Ok(commands) =
        session.root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
    else {
        eprintln!("[错误] commands 服务未就绪（装配异常），命令未处理");
        return CommandSettled::Action;
    };
    let Some(descriptor) = commands.find(token) else {
        eprintln!("未知命令 /{token}（/help 看命令）");
        return CommandSettled::Action;
    };
    let Some(handler) = commands.handler_of(token) else {
        // 防御位：find 与 handler_of 同解析规则，理论不可达；不可达也 fail-visible
        eprintln!("命令 /{token} 有目录无执行入口（装配异常）");
        return CommandSettled::Action;
    };
    // task-38⑤（lead 裁方案 A：执行照常+一行提示，stderr 通道不落盘）：动作类
    // 命令参数余段非空即提示——语义分层：未知命令=意图不明必须问（上方
    // fail-loud），未知参数=意图明确只是多余（提示即可，不阻断）。展开类（技能）
    // 的参数是回填载荷的一部分，不提示。
    // task-57：per-command arity 声明落地（38⑤ 卡上留痕兑现）——accepts_args=true
    // 的命令（/sessions <序号>）参数是合法载荷，移出本提示名单。
    if descriptor.kind == CommandKind::Action && !descriptor.accepts_args {
        if let Some(extra) = extra_args(raw, token) {
            eprintln!("（提示：/{token} 未接受参数，已忽略：{extra}）");
        }
    }
    // D2：run 先于 handler 落当前会话
    let command_id = uuid::Uuid::new_v4().simple().to_string();
    append_command_event(
        &session.root,
        SessionEvent::CommandRun {
            command_id: command_id.clone(),
            name: descriptor.name.clone(),
            raw_input: raw.to_string(),
        },
    )
    .await;
    // 执行（thrown → Error 结算，? 链兜底；命令 handler 毫秒级，Ctrl-C 窗口忽略）
    let output = match handler(CommandInvocation { raw_input: raw.to_string() }).await {
        Ok(output) => output,
        Err(err) => CommandOutput::error(format!("{err}")),
    };
    // D2：done 后于结算，**重新取**当前会话（/new 已换槽则落新文件）
    let done_kind = match output.kind {
        CommandResultKind::Success => CommandOutcome::Success,
        CommandResultKind::Error => CommandOutcome::Error,
    };
    append_command_event(
        &slot.load().root,
        SessionEvent::CommandDone { command_id, kind: done_kind, text: output.text.clone() },
    )
    .await;
    match (descriptor.kind, output.kind) {
        (_, CommandResultKind::Error) => {
            eprintln!("{}", output.text);
            CommandSettled::Action
        }
        (CommandKind::Action, CommandResultKind::Success) => {
            if !output.text.is_empty() {
                println!("{}", output.text);
            }
            CommandSettled::Action
        }
        (CommandKind::Expand, CommandResultKind::Success) => CommandSettled::Expand(output.text),
    }
}

/// 命令参数余段提取（task-38⑤）：去掉斜杠与首 token 后的剩余非空段。
/// `"/tools-info /new"` → `Some("/new")`；`"/help"` → `None`；token 不在行首
/// （防御，理论不可达——token 即首段）→ `None`。
/// task-17① 起 pub(crate)：sessions_cmd 的别名感知参数解析复用同款提取
/// （token 由调用方按「斜杠后首空白段」自行得出——主名/别名均适用）。
pub(crate) fn extra_args<'a>(raw: &'a str, token: &str) -> Option<&'a str> {
    let body = raw.strip_prefix('/')?.trim_start();
    let rest = body.strip_prefix(token)?.trim_start();
    let rest = rest.strip_prefix(token).unwrap_or(rest); // 防御：token 恰为空串
    (!rest.is_empty()).then_some(rest)
}

/// 命令日志事件落盘：store 缺失/失败仅告警不阻断——会话日志是纯审计配对
///（设计 §五），命令执行本体不依赖它成功。
async fn append_command_event(root: &Context, event: SessionEvent) {
    match root.get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>()) {
        Ok(store) => {
            if let Err(err) = store.append(&event).await {
                eprintln!("命令日志落盘失败（不影响执行）：{err}");
            }
        }
        Err(err) => eprintln!("命令日志落盘失败（store 未就绪，不影响执行）：{err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_commands::ActiveSession;
    use tokio_util::sync::CancellationToken;
    use yunyou_commands::{CommandDefinition, CommandsPlugin};
    use yunyou_session::SessionPlugin;

    /// 测试装配：真 SessionPlugin（temp jsonl）+CommandsPlugin+会话槽。
    /// 真装配自证：分发与落盘走的是内核服务缝，非 mock；execute_command
    /// 只消费槽内 root 的服务，ShellCore（run_repl 的壳核）不在被测面。
    ///
    /// 目录按原子序号隔离：并发 #[tokio::test] 共享路径会让两个 JsonlStore
    /// 写同一文件——append 的 line 与换行是两次 write，交错即物理行粘连、
    /// 落盘断言互踩（真实场景单会话单 store 有内部锁串行，无此问题）。
    async fn setup() -> (Arc<SessionSlot>, std::path::PathBuf) {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yunyou-cmdexec-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // 清上轮残留（断言失败遗留的旧文件）
        std::fs::create_dir_all(&dir).expect("temp dir");
        let session_file = dir.join("test-session.jsonl");
        let root = Context::new();
        let handle = root.plugin(
            SessionPlugin,
            serde_json::json!({ "path": session_file.display().to_string() }),
        );
        handle.await_ready().await.expect("session 插件应装载");
        let handle = root.plugin(CommandsPlugin, serde_json::json!({}));
        handle.await_ready().await.expect("commands 插件应装载");
        let slot = SessionSlot::new(ActiveSession {
            root: root.clone(),
            handler: Arc::new(|_line: &str, _cancel: CancellationToken| {
                Box::pin(async { Ok::<(), crate::run::AppError>(()) })
            }),
            plugin_status: Vec::new(),
        });
        (slot, session_file)
    }

    /// 注册测试命令：动作类 ping（上屏文本）+展开类 skill1（回填文本）。
    async fn register_test_commands(slot: &Arc<SessionSlot>) {
        let session = slot.load();
        let commands = session
            .root
            .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
            .expect("commands 服务应就绪");
        commands
            .register(CommandDefinition {
                name: "ping".into(),
                aliases: vec!["乒".into()],
                description: "动作类测试命令".into(),
                kind: CommandKind::Action,
                accepts_args: false,
                handler: Arc::new(|_inv: CommandInvocation| {
                    Box::pin(async { Ok(CommandOutput::success("pong")) })
                        as yunyou_util::BoxFut<'static, yunyou::Result<CommandOutput>>
                }),
            })
            .expect("ping 注册");
        commands
            .register(CommandDefinition {
                name: "skill1".into(),
                aliases: vec![],
                description: "展开类测试命令".into(),
                kind: CommandKind::Expand,
                accepts_args: false,
                handler: Arc::new(|_inv: CommandInvocation| {
                    Box::pin(async { Ok(CommandOutput::success("# 技能正文")) })
                        as yunyou_util::BoxFut<'static, yunyou::Result<CommandOutput>>
                }),
            })
            .expect("skill1 注册");
    }

    /// 会话文件里的命令事件行（tag → 原始 json 行列表）。
    fn command_lines(file: &std::path::Path, tag: &str) -> Vec<String> {
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&format!("\"type\":\"{tag}\"")))
            .map(str::to_string)
            .collect()
    }

    /// D2 落盘配对：run/done 各一行、command_id 相等、name=解析后规范名；
    /// 未知命令不落任何日志；展开类结算 Expand。
    #[tokio::test(flavor = "multi_thread")]
    async fn 命令执行落盘配对与未知命令不落日志() {
        let (slot, file) = setup().await;
        register_test_commands(&slot).await;
        // 动作类：run+done 配对落盘，结算 Success
        let settled = execute_command(&slot, "/ping 附加参数").await;
        assert!(matches!(settled, CommandSettled::Action));
        let run = command_lines(&file, "command-run");
        let done = command_lines(&file, "command-done");
        assert_eq!(run.len(), 1, "run 恰一行: {run:?}");
        assert_eq!(done.len(), 1, "done 恰一行: {done:?}");
        assert!(run[0].contains("\"name\":\"ping\""), "name=解析后规范名: {}", run[0]);
        assert!(run[0].contains("\"raw_input\":\"/ping 附加参数\""), "原始输入恒记录");
        assert!(done[0].contains("\"kind\":\"success\""), "kebab-case 结算: {}", done[0]);
        let extract = |line: &str| -> String {
            line.split("\"command_id\":\"").nth(1).unwrap().split('"').next().unwrap().into()
        };
        assert_eq!(extract(&run[0]), extract(&done[0]), "command_id 跨行配对");
        // 展开类：结算 Expand 且落盘照常
        let settled = execute_command(&slot, "/skill1").await;
        assert!(matches!(settled, CommandSettled::Expand(text) if text == "# 技能正文"));
        assert_eq!(command_lines(&file, "command-run").len(), 2);
        // 未知命令：stderr 提示（不可断言），但会话文件零新增
        let settled = execute_command(&slot, "/nope").await;
        assert!(matches!(settled, CommandSettled::Action));
        assert_eq!(command_lines(&file, "command-run").len(), 2, "未知命令不落 run");
        assert_eq!(command_lines(&file, "command-done").len(), 2, "未知命令不落 done");
    }

    /// task-38⑤：参数余段提取——带参命中、无参 None、多空格容忍。
    #[test]
    fn 参数余段提取() {
        assert_eq!(extra_args("/tools-info /new", "tools-info"), Some("/new"));
        assert_eq!(extra_args("/help", "help"), None);
        assert_eq!(extra_args("/ping 附加参数", "ping"), Some("附加参数"));
        assert_eq!(extra_args("/ping   多段  参数", "ping"), Some("多段  参数"));
        assert_eq!(extra_args("/乒 参数", "乒"), Some("参数"), "中文别名同规则");
        assert_eq!(extra_args("/quit", "quit"), None);
    }

    /// thrown 结算：handler 返回 Err → done 落 Error（? 链兜底）。
    #[tokio::test(flavor = "multi_thread")]
    async fn handler抛错结算为error() {
        let (slot, file) = setup().await;
        let session = slot.load();
        let commands = session
            .root
            .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
            .expect("commands 服务应就绪");
        commands
            .register(CommandDefinition {
                name: "boom".into(),
                aliases: vec![],
                description: "抛错测试命令".into(),
                kind: CommandKind::Action,
                accepts_args: false,
                handler: Arc::new(|_inv: CommandInvocation| {
                    Box::pin(async {
                        Err(yunyou::CordisError::new(
                            yunyou::ErrorKind::Internal,
                            "炸了",
                        ))
                    })
                }),
            })
            .expect("boom 注册");
        let settled = execute_command(&slot, "/boom").await;
        assert!(matches!(settled, CommandSettled::Action));
        let done = command_lines(&file, "command-done");
        assert_eq!(done.len(), 1);
        assert!(done[0].contains("\"kind\":\"error\""), "thrown→Error: {}", done[0]);
    }
}
