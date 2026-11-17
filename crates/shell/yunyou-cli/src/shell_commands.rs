//! 壳自有命令（任务29 C2）：/help /quit /new 经命令缝注册。
//!
//! 职责边界（设计 §三）：退出/帮助/会话管理是壳的行为，但**禁硬编码进装配
//! 序列**——三个命令与插件命令同构，走 register 缝进注册表，REPL 循环只认
//! 注册表分发（/quit 迁移自 repl.rs 的硬编码分支，作既有命令迁移示范件）。
//!
//! /new 的 D3 裁决落点（实现批自证，台账在案）：优先路径「内核 update/reload
//! 缝换 store」不成立——AgentLoopRunner.sessions 是装载期一次 get 的强 Arc
//!（yunyou-agent-loop/src/turn.rs:33），session 插件经 update 缝换 store 后旧
//! Arc 依旧有效、agent-loop 继续写旧文件；要新 store 生效须逐个重启消费方，
//! 而「谁消费 SessionStore」是插件间依赖知识，宿主枚举=把插件依赖硬编码进
//! 装配序列，违反主板论。故走设计明文回退路径：**重启整个装配**——
//! 旧 root.dispose() 完整 teardown（内核整树逆序卸载，防半卸树）→ 重新
//! assemble（新会话文件路径），毫秒级、行为等价全新会话。
//!
//! 会话槽（SessionSlot）：/new 重建后 REPL 换装配的载体——槽里是当前装配的
//! root+行 handler，REPL 每轮从槽读，重建只换槽不动 REPL 循环结构。

use crate::repl::{LineHandler, RenderConfig};
use crate::{assemble, render::Renderer};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use yunyou::ServiceId;
use yunyou_session::SessionStore;
use yunyou_commands::{
    CommandDefinition, CommandHandler, CommandInvocation, CommandKind, CommandOutput,
    CommandsService,
};
use yunyou_config::DshConfig;
use yunyou_util::BoxFut;
use yunyou::Context;

/// 当前活跃会话（装配产物中 REPL 依赖的两件：root 与行 handler）。
pub struct ActiveSession {
    /// 装配 root（服务查询与优雅关闭入口）。
    pub root: Context,
    /// 行 handler（普通消息的对话执行入口）。
    pub handler: LineHandler,
    /// 插件状态行（K3 /plugins 观测面数据源；随 /new 换槽自动携带新装配状态）。
    pub plugin_status: Vec<crate::status::PluginStatus>,
}

/// 会话槽：/new 重建后 REPL 读新装配的切换点。
pub struct SessionSlot {
    current: std::sync::RwLock<Arc<ActiveSession>>,
}

impl SessionSlot {
    /// 用初始装配建槽。
    pub fn new(session: ActiveSession) -> Arc<Self> {
        Arc::new(Self { current: std::sync::RwLock::new(Arc::new(session)) })
    }

    /// 当前会话快照（Arc，读不锁久持安全）。
    pub fn load(&self) -> Arc<ActiveSession> {
        self.current.read().expect("会话槽锁").clone()
    }

    /// 换会话（/new 重建后调用）。
    pub fn store(&self, session: ActiveSession) {
        *self.current.write().expect("会话槽锁") = Arc::new(session);
    }
}

/// 壳命令的执行环境（闭包捕获；/new 重建所需材料全集）。
pub struct ShellCore {
    /// 启动配置（重建时改 session.path 后复用）。
    pub config: DshConfig,
    /// yunyou-home。
    pub home: PathBuf,
    /// 配置文件路径（assemble 的迁移改写对象）。
    pub config_path: PathBuf,
    /// 会话目录（/new 的新文件落点）。
    pub session_dir: PathBuf,
    /// 会话槽。
    pub slot: Arc<SessionSlot>,
    /// 渲染配置（新装配的行 handler 与横幅同源）。
    pub render_config: RenderConfig,
    /// 壳退出令牌（/quit）。
    pub exit: CancellationToken,
    /// 当前活跃会话文件路径（task-36④：/sessions 列表 ► 标与切换判定的数据源；
    /// 启动时由 run.rs 初始化，rebuild_session_to 换会话时更新）。
    pub current_session_path: std::sync::Mutex<PathBuf>,
}

impl ShellCore {
    /// D3 回退路径：完整 teardown → 重新装配（会话文件）→ 换槽+清屏重印横幅+回放。
    ///
    /// task-36④ 参数化：`None`=/new 语义（新 uuid 文件）；`Some(path)`=/sessions
    /// 切换到指定会话文件（flock 时序与 /new 同构：teardown 先释放旧锁，
    /// assemble 期 session 插件抢新锁）。
    pub async fn rebuild_session_to(
        self: &Arc<Self>,
        target: Option<PathBuf>,
    ) -> Result<CommandOutput, yunyou::CordisError> {
        let switched = target.is_some();
        let new_path = match target {
            Some(path) => path,
            None => {
                let new_id = uuid::Uuid::new_v4().simple().to_string();
                self.session_dir.join(format!("{new_id}.jsonl"))
            }
        };
        let mut config = self.config.clone();
        config.session.path = Some(new_path.display().to_string());
        // ① task-58 先立后拆：**先装配新（含目标会话 flock 抢锁=前置试锁语义内嵌
        // ——assemble 内 session 插件抢锁失败即自卸新树返回 Err），成功才动旧**。
        // 切换失败（目标被另一实例锁定等）完全可逆：报错+留在当前会话继续对话
        // （v7 压测 ①d 实锤：旧「先拆后立」在锁冲突时卸掉当前装配+REPL 强退=不可逆）。
        let assembled = match assemble::assemble(&config, &self.home, &self.config_path).await {
            Ok(assembled) => assembled,
            Err(err) => {
                return Ok(CommandOutput::error(format!(
                    "切换失败（当前会话未受影响，可继续对话）：{err}"
                )))
            }
        };
        // 旧装配 root 先抓（store 换槽后就取不到了），换槽成功后异步卸载
        let old_root = self.slot.load().root.clone();
        // task-74 修法 a/v3：换槽后新装配的 store 显式注入（同 run.rs 装配点
        // 口径——上下文查找类失效一类根除）。
        let repl_store = assembled
            .root
            .get::<dyn yunyou_session::SessionStore>(yunyou::ServiceId::of::<dyn yunyou_session::SessionStore>())
            .ok();
        let handler = crate::repl::loop_line_handler(assembled.root.clone(), self.render_config.clone(), repl_store);
        self.slot.store(ActiveSession {
            root: assembled.root.clone(),
            handler,
            plugin_status: assembled.plugin_status,
        });
        // 新装配的注册表是空的（CommandsPlugin 只 provide 服务）——壳命令必须
        // 重新过 register 缝，否则 /new 之后 /help 变「未知命令」
        if let Err(err) = register_shell_commands(&assembled.root, self.clone()) {
            eprintln!("新装配壳命令注册失败（不影响会话本体）：{err}");
        }
        // task-36④：当前会话路径跟随切换（/sessions 列表 ► 标的数据源）
        *self.current_session_path.lock().expect("会话路径锁") = new_path.clone();
        // ③ 清屏+重印横幅+尾部回放（设计 §六④ 收尾；task-36④ 切换场景同构——
        // 切到有历史的会话时回放尾部 10 条，恢复上下文可见性）
        let mut stdout = std::io::stdout();
        if self.render_config.interactive {
            let _ = write!(stdout, "\x1b[2J\x1b[1;1H");
        }
        let mut renderer =
            Renderer::new(stdout, self.render_config.profile, self.render_config.interactive);
        let session_dir =
            new_path.parent().map(|d| d.display().to_string()).unwrap_or_default();
        if let Err(err) = renderer.banner(&config.llm.model, &session_dir, None, true) {
            eprintln!("横幅重印失败：{err}");
        }
        let replay_events = yunyou_session::JsonlStore::new(&new_path)
            .load_all()
            .await
            .unwrap_or_default();
        let mut out = renderer.into_output();
        if let Err(err) =
            crate::sessions_cmd::render_replay(&replay_events, &mut out, self.render_config.profile)
        {
            eprintln!("会话回放渲染失败：{err}");
        }
        // ③ 后拆旧：新装配已生效（换槽+重注册+重印完成），此刻卸载旧树——
        // 先消费者后提供者的逆序卸载语义由 dispose 内部保证；卸载失败仅警告
        // （新会话已立，旧树泄漏面=进程退出即回收，非会话态丢失）
        if let Err(err) = old_root.dispose().await {
            eprintln!("旧会话装配卸载警告：{err}");
        }
        if switched {
            let name = new_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(CommandOutput::success(format!("已切换会话 {name}")))
        } else {
            Ok(CommandOutput::success(format!(
                "已新开会话 {}",
                new_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            )))
        }
    }
}

/// 注册壳命令（/help /quit /new ＋ /plugins 观测面，K3；走命令缝；注册即
/// effect——disposer 登记为 root 的 effect）。
///
/// 装配完成后调用一次；重复调用同名 register 会 Err（fail-loud 防重复注册）。
pub fn register_shell_commands(root: &Context, shell: Arc<ShellCore>) -> yunyou::Result<()> {
    let commands = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())?;
    for def in shell_command_definitions(&shell) {
        let name = def.name.clone();
        let disposer = commands.register(def)?;
        // 注册即 effect：root 卸载（进程退出）时自动除名；这里同批登记保证
        // 「注册即 effect」纪律在壳面同样成立
        root.effect(format!("壳命令 /{name}"), move || Ok(Some(disposer)))?;
    }
    // /plugins 观测面（K3，plugins_cmd.rs 域）：数据源=会话槽的 plugin_status，
    // /new 换槽自动携带新装配状态
    crate::plugins_cmd::register_plugins_command(root, shell.slot.clone())?;
    Ok(())
}

/// 壳三命令定义（/help 从注册表生成文案、/quit 发退出信号、/new 重建会话）。
fn shell_command_definitions(shell: &Arc<ShellCore>) -> Vec<CommandDefinition> {
    vec![
        CommandDefinition {
            name: "help".into(),
            aliases: vec!["帮助".into()],
            description: "看命令（列出全部斜杠命令）".into(),
            kind: CommandKind::Action,
            accepts_args: false,
            handler: help_handler(shell.clone()),
        },
        CommandDefinition {
            name: "quit".into(),
            aliases: vec!["退出".into()],
            description: "退出云悠".into(),
            kind: CommandKind::Action,
            accepts_args: false,
            handler: quit_handler(shell.clone()),
        },
        CommandDefinition {
            name: "new".into(),
            aliases: vec!["新建".into()],
            description: "新开全新会话（旧会话文件保留）".into(),
            kind: CommandKind::Action,
            accepts_args: false,
            handler: new_handler(shell.clone()),
        },
        // task-36④：/sessions 列表与切换（定义与 handler 在 sessions_cmd.rs，
        // 本文件行数贴硬顶，增量外置）
        crate::sessions_cmd::definition(shell),
        // task-27：/compact 手动压缩（同款增量外置）
        crate::compact_cmd::definition(shell),
    ]
}

/// /help：文案从注册表生成（新增命令自动出现，禁硬编码——设计 §三）。
fn help_handler(shell: Arc<ShellCore>) -> CommandHandler {
    Arc::new(move |_inv: CommandInvocation| {
        let shell = shell.clone();
        Box::pin(async move {
            let session = shell.slot.load();
            let commands = session
                .root
                .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())?;
            let mut text = String::from("可用命令：");
            for cmd in commands.list() {
                let aliases = if cmd.aliases.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", cmd.aliases.join("/"))
                };
                text.push_str(&format!("\n  /{}{}  {}", cmd.name, aliases, cmd.description));
            }
            Ok(CommandOutput::success(text))
        }) as BoxFut<'static, _>
    })
}

/// /quit：发退出信号（迁移自 repl.rs:115 硬编码分支，设计 §三示范件）。
fn quit_handler(shell: Arc<ShellCore>) -> CommandHandler {
    Arc::new(move |_inv: CommandInvocation| {
        let shell = shell.clone();
        Box::pin(async move {
            shell.exit.cancel();
            Ok(CommandOutput::success(String::new()))
        }) as BoxFut<'static, _>
    })
}

/// /new：会话切换（D3 回退路径，见模块注释自证）。
fn new_handler(shell: Arc<ShellCore>) -> CommandHandler {
    Arc::new(move |_inv: CommandInvocation| {
        let shell = shell.clone();
        Box::pin(async move { shell.rebuild_session_to(None).await })
    })
}
