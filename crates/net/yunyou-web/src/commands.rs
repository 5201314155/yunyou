//! 斜杠命令执行（P3）：经命令缝（CommandsService）服务端执行——与 REPL 同源
//! 分发（同注册表、同 handler、同 D2 落盘配对纪律），输出通道从终端换成 WS。
//!
//! # /quit 不经此模块（web 语义裁决）
//! 壳 /quit 的退出令牌会杀服务殃及其他连接——web 里 /quit=断开本连接，
//! 由 ws.rs 拦截处置。
//!
//! # 与 REPL 命令链的差异声明（同缝异壳）
//! - 输出：REPL 上屏 stdout；web 走 command-output 帧（ok=Success/Error）；
//! - 展开类（技能）：REPL 回填输入框待确认；web 一期不代发——上屏原文并
//!   注明回填语义（与 CLI 非交互管道行为同规：展示、不代发）；
//! - 未知命令/参数余段：提示语义照搬 command_exec（fail-visible，不落盘）。

use crate::protocol::ServerMessage;
use crate::server::WebState;
use std::sync::Arc;
use yunyou::{Context, ServiceId};
use yunyou_commands::{
    CommandInvocation, CommandKind, CommandOutput, CommandResultKind, CommandsService,
};
use yunyou_session::events::CommandOutcome;
use yunyou_session::{SessionEvent, SessionStore};

/// 执行一条斜杠命令（raw 含前导斜杠；/quit 已被 ws.rs 拦截不会到达）。
/// 全程持回合闸门：与回合、与其他命令串行（/new 重建装配的安全前提）。
pub(crate) async fn execute(state: &Arc<WebState>, raw: &str) -> Option<ServerMessage> {
    let _gate = state.turns.lock_gate().await;
    let token = raw
        .strip_prefix('/')
        .and_then(|body| body.split_whitespace().next())
        .unwrap_or("");
    let root = state.turns.root();
    let Ok(commands) = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
    else {
        return Some(ServerMessage::Error {
            text: "commands 服务未就绪（装配异常），命令未处理".into(),
        });
    };
    let Some(descriptor) = commands.find(token) else {
        return Some(ServerMessage::CommandOutput {
            text: format!("未知命令 /{token}（/help 看命令）"),
            ok: false,
        });
    };
    let Some(handler) = commands.handler_of(token) else {
        // 防御位：find 与 handler_of 同解析规则，理论不可达；不可达也 fail-visible
        return Some(ServerMessage::CommandOutput {
            text: format!("命令 /{token} 有目录无执行入口（装配异常）"),
            ok: false,
        });
    };
    // task-38⑤ 语义照搬：动作类不收参数，参数余段非空即提示（不阻断）
    let mut prefix = String::new();
    if descriptor.kind == CommandKind::Action {
        if let Some(extra) = extra_args(raw, token) {
            prefix = format!("（提示：/{token} 未接受参数，已忽略：{extra}）\n");
        }
    }
    // D2：run 先于 handler 落当前会话
    let command_id = uuid::Uuid::new_v4().simple().to_string();
    append_event(
        &root,
        SessionEvent::CommandRun {
            command_id: command_id.clone(),
            name: descriptor.name.clone(),
            raw_input: raw.to_string(),
        },
    )
    .await;
    // 执行（thrown → Error 结算，? 链兜底；命令 handler 毫秒级——/new 重建除外，
    // 由回合闸门保证不与回合并发）
    let output = match handler(CommandInvocation { raw_input: raw.to_string() }).await {
        Ok(output) => output,
        Err(err) => CommandOutput::error(format!("{err}")),
    };
    // D2：done 后于结算，重新取当前会话（/new 已换槽则落新文件——配对键跨文件）
    let done_kind = match output.kind {
        CommandResultKind::Success => CommandOutcome::Success,
        CommandResultKind::Error => CommandOutcome::Error,
    };
    append_event(
        &state.turns.root(),
        SessionEvent::CommandDone { command_id, kind: done_kind, text: output.text.clone() },
    )
    .await;
    let ok = output.kind == CommandResultKind::Success;
    // 空成功文本静默（REPL 同规：动作类空输出不上屏）
    if ok && output.text.is_empty() {
        return None;
    }
    // 展开类：web 一期不代发，注明回填语义（差异声明见模块头）
    let text = if ok && descriptor.kind == CommandKind::Expand {
        format!("{prefix}{}（回填文本：如需发送请自行编辑后发送——web 一期不代发）", output.text)
    } else {
        format!("{prefix}{}", output.text)
    };
    Some(ServerMessage::CommandOutput { text, ok })
}

/// 命令参数余段提取（照搬 command_exec 同名函数语义）。
fn extra_args<'a>(raw: &'a str, token: &str) -> Option<&'a str> {
    let body = raw.strip_prefix('/')?.trim_start();
    let rest = body.strip_prefix(token)?.trim_start();
    let rest = rest.strip_prefix(token).unwrap_or(rest); // 防御：token 恰为空串
    (!rest.is_empty()).then_some(rest)
}

/// 命令日志事件落盘：store 缺失/失败仅告警不阻断（纯审计配对，command_exec 同纪律）。
async fn append_event(root: &Context, event: SessionEvent) {
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
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::server::WebState;
    use crate::turns::TurnRunner;
    use std::path::PathBuf;
    use tokio::sync::broadcast;
    use yunyou_commands::{CommandDefinition, CommandsPlugin};
    use yunyou_session::SessionPlugin;

    /// 测试装配：真 CommandsPlugin+SessionPlugin（temp jsonl）+ 测试命令注册
    /// （rpc_commands.rs 测试复用——同一装配语义）。
    pub(crate) async fn setup() -> (Arc<WebState>, PathBuf) {
        // 目录按原子序号隔离：多测试并发 setup 各持 flock，共享路径会互锁
        //（sessions.rs 同款隔离手段）。
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("yunyou-web-cmd-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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
        let commands = root
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
        let getter_root = root.clone();
        let (sender, _) = broadcast::channel(16);
        let state = Arc::new(WebState {
            token: "t".into(),
            model: "test-model".into(),
            turns: Arc::new(TurnRunner::new(
                Arc::new(move || getter_root.clone()),
                "test-model".into(),
            )),
            broadcast: sender,
            session_dir: dir.join("sessions"),
            wire: Arc::new(crate::bridge::WireBridge::new("test-model".into())),
            scheduler: Arc::new(yunyou_plugin_sdk::scheduler::Scheduler::new(|| 1_000_000)),
            config_store: Arc::new(crate::config_store::ConfigStore::new(std::path::PathBuf::from("/tmp/yunyou-test-yunyou.toml"))),
        });
        (state, session_file)
    }

    /// 命令执行闭环：动作类成功/未知命令报错不落盘/展开类注明不代发/D2 落盘配对。
    #[tokio::test(flavor = "multi_thread")]
    async fn 命令执行闭环与落盘配对() {
        let (state, file) = setup().await;
        // 动作类：Success 帧
        let msg = execute(&state, "/ping").await.expect("应有输出");
        let ServerMessage::CommandOutput { text, ok } = &msg else { panic!("应回命令帧：{msg:?}") };
        assert_eq!((text.as_str(), *ok), ("pong", true));
        // 参数余段：提示前缀（人话不阻断）
        let msg = execute(&state, "/ping 多余参数").await.expect("应有输出");
        let ServerMessage::CommandOutput { text, ok } = &msg else { panic!("应回命令帧：{msg:?}") };
        assert!(*ok && text.contains("未接受参数") && text.contains("多余参数"), "{text}");
        // 未知命令：ok=false 且不落盘
        let msg = execute(&state, "/nope").await.expect("应有输出");
        let ServerMessage::CommandOutput { text, ok } = &msg else { panic!("应回命令帧：{msg:?}") };
        assert!(!*ok && text.contains("未知命令 /nope"), "{text}");
        // 展开类：原文+不代发注记
        let msg = execute(&state, "/skill1").await.expect("应有输出");
        let ServerMessage::CommandOutput { text, ok } = &msg else { panic!("应回命令帧：{msg:?}") };
        assert!(*ok && text.contains("# 技能正文") && text.contains("不代发"), "{text}");
        // D2：run/done 配对落盘（ping×2 + skill1×1；未知命令不计）
        let lines = std::fs::read_to_string(&file).unwrap_or_default();
        let runs: Vec<&str> = lines.lines().filter(|l| l.contains("\"type\":\"command-run\"")).collect();
        let dones: Vec<&str> = lines.lines().filter(|l| l.contains("\"type\":\"command-done\"")).collect();
        assert_eq!(runs.len(), 3, "run 应 3 行：{runs:?}");
        assert_eq!(dones.len(), 3, "done 应 3 行：{dones:?}");
        assert!(runs[0].contains("\"name\":\"ping\""), "name=解析后规范名：{}", runs[0]);
        assert!(dones[2].contains("\"kind\":\"success\""), "kebab-case 结算：{}", dones[2]);
    }

    /// /quit 不经此模块的语义由 ws.rs 拦截保证（此处验 token 解析对别名不误判）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 参数余段提取规则() {
        assert_eq!(extra_args("/tools-info /new", "tools-info"), Some("/new"));
        assert_eq!(extra_args("/help", "help"), None);
        assert_eq!(extra_args("/乒 参数", "乒"), Some("参数"), "中文别名同规则");
    }
}
