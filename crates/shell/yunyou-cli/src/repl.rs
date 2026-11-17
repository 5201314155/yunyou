//! 交互循环（REPL）与单任务模式；对话经 `dyn AgentLoop` 服务执行，
//! 事件经 [`TerminalObserver`] 实时渲染（task-10：流式/工具卡/思考块）。
//!
//! Ctrl-C 语义：空闲期直接退出；对话期先 cancel 本轮（run_turn 在流/工具
//! 边界干净收尾），本轮落定后退出。渲染态零残留：观察者 [`TerminalObserver::finish`]
//! 在 run_turn 返回后 Ok/Err 两路统一收尾——落定未满行并清活动行；
//! Err 路径 A 线不发 TurnComplete（turn.rs 事件时序契约），收尾由宿主兜底。
//!
//! 命令分发（任务29 C2-repl，设计 §三）：REPL 循环只认注册表——以 `/` 开头
//! 的输入经 [`CommandsService::handler_of`] 分发（/quit 的硬编码分支已迁出，
//! 本文件不再认识任何具体命令）；命令执行链带 D2 落盘（command-run 先于
//! handler、command-done 后于结算，跨文件配对见 [`execute_command`]）。

use crate::command_exec::{execute_command, CommandSettled};
use crate::input_source::InputSource;
use crate::lineedit::{Editor, LineInput};
use crate::observe::TerminalObserver;
use yunyou_util::BoxFut;
use crate::render::{Profile, Renderer};
pub use crate::render::ThinkingMode;
use crate::shell_commands::{ActiveSession, SessionSlot, ShellCore};
use crate::AppError;
use std::io::{stdout, Write};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use yunyou::{Context, ServiceId};
use yunyou_session::SessionStore;
use yunyou_agent_loop::AgentLoop;
use yunyou_commands::CommandsService;
use yunyou_llm::LlmFailure;

/// 渲染相关配置（REPL/单任务共用：档位 + 交互性 + 模型名 + 思考流模式）。
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// 终端能力档位（NO_COLOR/YUNYOU_PLAIN/TERM=dumb/非 TTY 降级矩阵）。
    pub profile: Profile,
    /// 交互终端（TTY 且 TERM≠dumb）：false 时一切暂态静默，只出落定行。
    pub interactive: bool,
    /// 模型名（横幅与状态落款）。
    pub model: String,
    /// 思考流模式（任务 30-②：默认收敛；YUNYOU_THINKING=full 全文流式）。
    pub thinking: ThinkingMode,
}

impl ThinkingMode {
    /// 从环境探测：`YUNYOU_THINKING=full` → 全文流式逃生口；其余一律收敛。
    pub fn from_env() -> Self {
        if std::env::var("YUNYOU_THINKING").ok().as_deref() == Some("full") {
            Self::Full
        } else {
            Self::Collapse
        }
    }
}

/// 一行用户输入的处置（token 供 REPL 在 Ctrl-C 时中断本轮对话）。
pub type LineHandler = Arc<dyn Fn(&str, CancellationToken) -> BoxFut<'static, Result<(), AppError>> + Send + Sync>;

/// 对话 handler：取 `dyn AgentLoop` 服务，`run_turn_observed` 驱动实时渲染。
///
/// 持 root 强句柄合法：REPL 是宿主装配层，root 与进程同生命周期（方案 §2.1.2
/// 的「仅组件受限」契约不约束宿主）。
pub fn loop_line_handler(
    root: Context,
    render: RenderConfig,
    store: Option<Arc<dyn SessionStore>>,
) -> LineHandler {
    // task-48 显式静态档：YUNYOU_STATIC=1（高延迟链路用户显式选择；env 在装配
    // 点读一次，Renderer 经 set_selfheal_forced 接线，出厂即暂态静默不自动恢复）。
    let selfheal_forced = std::env::var("YUNYOU_STATIC").as_deref() == Ok("1");
    // task-48 验收/调试逃生口（规范 §4.14 分层标注）：低阈值触发自愈真机验收
    // 用，非生产旋钮（生产阈值日后经 [render] 配置段）。0 或非法值=回默认。
    let selfheal_threshold = std::env::var("YUNYOU_SELFHEAL_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(crate::render::selfheal::DEFAULT_WINDOW_BYTES);
    Arc::new(move |line: &str, cancel: CancellationToken| {
        let root = root.clone();
        let render = render.clone();
        let store = store.clone();
        let line = line.to_string();
        Box::pin(async move {
            let Ok(agent_loop) = root.get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>()) else {
                // task-36⑤：失败信令（展示职责归消费方，单点打印）
                return Err(AppError::Turn(
                    "agentLoop 服务未就绪（装配异常），本轮输入未处理".into(),
                ));
            };
            // task-53（P1）：回合起点 drain lead 信箱（K-7 消费缝闭环）——成员完工
            // Report 等 hub 受理后在本轮注入（【来自 <成员名>】前缀，与成员侧
            // 「【来自 lead】」形态同构），根除「hub 受理≠对话看见」非对称缝。
            // 团队插件未启用=直通；drain 失败=诚实提示不吞错（消息留箱下轮再注）。
            let line = match root.get::<dyn yunyou_team::TeamService>(ServiceId::of::<
                dyn yunyou_team::TeamService,
            >()) {
                Ok(team) => match team.drain_lead_inbox() {
                    Ok(messages) => yunyou_team::compose_turn_input(&messages, &line),
                    Err(err) => {
                        eprintln!("[提示] lead 信箱读取失败（消息留箱下轮再注）：{err}");
                        line
                    }
                },
                Err(_) => line, // 团队插件未启用：无信箱语义，直通
            };
            let mut renderer = Renderer::new(stdout(), render.profile, render.interactive);
            renderer.set_thinking(render.thinking);
            renderer.set_selfheal_threshold(selfheal_threshold); // task-48：阈值注入（默认或验收低阈）
            if selfheal_forced {
                renderer.set_selfheal_forced(); // task-48：显式静态档接线
            }
            let observer = Arc::new(TerminalObserver::new(renderer, render.model));
            observer.set_cancel(cancel.clone()); // task-38④：下游管道关闭→提前收场
            let outcome = agent_loop
                .run_turn_observed(&line, cancel.clone(), observer.clone())
                .await;
            // 收尾两路统一（审查 P1-2）：Ok 路径 TurnComplete 已落定落款行，finish 幂等；
            // Err 路径 A 线不发 TurnComplete，未满行/活动行的渲染收尾由 finish 兜底。
            observer.finish();
            match outcome {
                Ok(_outcome) => {} // 可见内容已由事件流渲染；TurnOutcome 供程序化消费
                Err(err) => {
                    if cancel.is_cancelled() {
                        // task-38④：取消可能源自下游管道关闭（--task | head），stdout
                        // 或已断——println! 遇 Broken pipe 会 panic，改安全写（失败静默）
                        let _ = writeln!(std::io::stdout(), "（已取消本轮对话）");
                        // task-74 v3 对账补写+v6 SystemMessage：取消回合的
                        // UserMessage 在 turn 内 persist（开头第一步）可能被 turn
                        // future drop 连带丢失——handler 层显式 store 对账（load_all
                        // 末行≠本行 user-message 即补写；已在=幂等跳过）。
                        // **SystemMessage「对话已取消」同批落盘（v6）**：SIGINT 时序
                        // 竞态下 turn 收流落定快于 select 轮询到 ctrl_c 臂——
                        // run_dialogue_turn 的 arm 级 append 未必执行（真中转复刻
                        // 实证：UserMessage 落/SystemMessage 缺）——handler 层
                        // Err-cancel 分支是取消事实的唯一必经点，SystemMessage
                        // 在此对账落盘=两态（arm 级/handler 级）全覆盖。
                        if let Some(store) = &store {
                            let already = store
                                .load_all()
                                .await
                                .ok()
                                .and_then(|events| {
                                    events.last().map(|last| {
                                        matches!(last, yunyou_session::SessionEvent::UserMessage { text } if *text == line)
                                    })
                                })
                                .unwrap_or(false);
                            if !already {
                                let _ = store
                                    .append(&yunyou_session::SessionEvent::UserMessage {
                                        text: line.clone(),
                                    })
                                    .await;
                            }
                            let _ = store
                                .append(&yunyou_session::SessionEvent::SystemMessage {
                                    text: "对话已取消（Ctrl-C）".to_string(),
                                })
                                .await;
                        }
                    } else if let Some(failure) = LlmFailure::from_cordis(&err) {
                        // task-34①：LLM 失败走人话（一句原因+行动项；内部类别码/
                        // 重复措辞/原始报文不再外露）。机器链 err 仍是事实保留
                        // （会话落盘），仅展示层换装；非 llm 错误保留原链。
                        println!("[对话失败] {}", failure.user_message());
                        // 错误事件落盘（task-49）：会话侧诚实留痕——业主实证
                        // 「错误事件=0」的静默形态根除；复用 SystemMessage 词汇
                        //（不新增会话 schema），落盘失败不吞主错误（尽力留痕）。
                        // task-74 修法 a 下沉 handler 层：显式注入实例（与 arm
                        // 层同源）——上下文查找类失效一类根除。
                        if let Some(store) = &store {
                            let _ = store
                                .append(&yunyou_session::SessionEvent::SystemMessage {
                                    text: format!("对话失败：{err}"),
                                })
                                .await;
                        }
                        // task-36⑤：失败信令上抛（人话已在上方输出；单任务非零退出）
                        return Err(AppError::Turn(failure.user_message()));
                    } else {
                        println!("[对话失败] {err}");
                        // task-36⑤：失败信令上抛
                        return Err(AppError::Turn(err.to_string()));
                    }
                }
            }
            Ok(()) // task-36⑤：成功=Ok 信令
        })
    })
}

/// 提示符串（与 `Renderer::prompt` 同款形态：换行+云青 ❯+空格）。
///
/// render/mod.rs 系双人区（云绘施工中，纪律：非必要不进），此处用同一套
/// palette token 平行构造；形态漂移风险由「token 同源」压住（同一生成物）。
fn prompt_string(profile: Profile) -> String {
    use crate::render::terminal::{ascii, SGR_BRAND_PRIMARY_256, SYM_PROMPT};
    // task-27①：前置换行移除——回合收尾/横幅各行尾自带换行，再换行即生
    // 观感空行；首提示符间距由 run.rs 横幅后显式空行承担。
    format!(
        "{}{}{} ",
        profile.color(SGR_BRAND_PRIMARY_256),
        profile.symbol(SYM_PROMPT, ascii::PROMPT),
        profile.reset()
    )
}

/// 普通消息一轮对话（经会话槽 handler，/new 后自动用新装配）。
///
/// 返回 true=对话期 Ctrl-C（先 cancel 本轮、落定后请求退出循环，既有语义）。
async fn run_dialogue_turn(
    session: &Arc<ActiveSession>,
    store: Option<Arc<dyn SessionStore>>,
    line: &str,
) -> Result<bool, AppError> {
    let cancel = CancellationToken::new();
    let mut turn = (session.handler)(line, cancel.clone());
    // task-34：回合期 Esc 监视臂的数据源（进程级单例，与 lineedit 同源）
    let source = InputSource::global();
    // task-61：Ctrl-C 后给 turn 有限等待（5s）——cancel 已发，stream.rs 的
    // cancelled 分支会在下一次流块间隙或 EOF 时触发收束；如果 reqwest 卡在
    // TLS/TCP 层（服务端不发则 chunk() 永 Pending），超时后放弃等待——
    // inline select 结构保留（59a 验证过的形态，ctrl_c 注册不迁移），
    // 超时后 turn future 被 drop（reqwest 连接 drop 即关，泄漏面可接受）。
    // **不使用 tokio::spawn**——spawn 后 ctrl_c 信号处理路径出现回归（云探
    // 三场铁证：0c68e7f 的 Ctrl-C 臂完全静默，61 打回）。
    let exit_requested = tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => {
            println!();
            cancel.cancel();
            // task-49/74 落盘语义：用户中断也是会话事实——072afed 重写时丢失了
            // 此段（第 8 例漏带），74 定谳后补回。修法 a（lead 裁）：store handle
            // 显式注入——上下文查找类失效一类根除。
            match &store {
                Some(store) => {
                    let _ = store
                        .append(&yunyou_session::SessionEvent::SystemMessage {
                            text: "对话已取消（Ctrl-C）".to_string(),
                        })
                        .await;
                }
                None => {
                    eprintln!("[selfheal] 会话服务未装载，取消事件未落盘");
                }
            }
            // 有限等待：stream.rs 的 cancelled 分支应在流块间隙或 EOF 时触发；
            // 超时=放弃等待（turn future drop=连接关闭），不永久悬停。
            let bounded_wait = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                &mut turn,
            );
            match bounded_wait.await {
                Ok(_) => {} // turn 落定（正常或 Err）
                Err(_) => {
                    eprintln!("[selfheal] 回合收尾超时（5s），放弃等待——turn future 已 drop");
                }
            }
            true
        }
        // task-34：回合期 Esc 监视臂（inline 内联于 select，禁 tokio::spawn
        // ——spawn 后 ctrl_c 臂静默回归=task-61 三场铁证；biased 顺序
        // Ctrl-C>Esc>turn=既有信号语义优先级不变）。孤立 Esc（0x1B+50ms
        // 无后续）=打断当前回合回提示符；有后续字节（方向键/Alt 形态）回退
        // pushback 排队不吞字。取消落盘与 Ctrl-C 同款 SystemMessage（来源
        // 标注 Esc 供审计区分）；工具执行期同臂覆盖（监视=整回合，task-14
        // 组收割自动复用）。
        _ = esc_interrupt(source) => {
            cancel.cancel();
            // 取消落盘（对话态同款，task-74 修法 a store 注入形态）：来源
            // 标注 Esc 与 Ctrl-C 区分（审计可辨）。
            match &store {
                Some(store) => {
                    let _ = store
                        .append(&yunyou_session::SessionEvent::SystemMessage {
                            text: "对话已取消（Esc）".to_string(),
                        })
                        .await;
                }
                None => {
                    eprintln!("[selfheal] 会话服务未装载，取消事件未落盘");
                }
            }
            println!();
            // 有限等待（与 Ctrl-C 分支同款 5s 上限）：cancel 已发，stream.rs
            // cancelled 分支在流块间隙或 EOF 触发收束；超时放弃等待。
            let bounded_wait = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                &mut turn,
            );
            match bounded_wait.await {
                Ok(_) => {}
                Err(_) => {
                    eprintln!("[selfheal] 回合收尾超时（5s），放弃等待——turn future 已 drop");
                }
            }
            // 命门（云勘契约一）：esc 臂返回 **false=继续循环回提示符**——
            // 与 ctrl_c 臂的 true（请求退出循环 exit 130）语义分叉，丁案的
            // 「对话态 Esc=打断回提示符继续聊」断言即此；重构勿顺手统一。
            // 竞态备案：Esc 与回合完成竞态时 cancel 对已完成 token=no-op，
            // 本臂仍回 false 继续循环（落盘的「对话已取消（Esc）」=事实
            // 「用户请求了取消」，云勘验收备注口径）。
            false
        }
        // task-36⑤：失败信令=REPL 单点打印后继续循环（不退出；文案已含人话）
        result = &mut turn => {
            if let Err(err) = result {
                eprintln!("yunyou：{err}");
            }
            false
        },
    };
    Ok(exit_requested)
}

/// 回合期 Esc 打断监视（task-34）：读 InputSource——读到 0x1B 且 50ms 内
/// 无后续字节=孤立 Esc（完成=触发取消）；有后续（方向键/Alt/普通字节）=
/// 非目标形态，整段回退 pushback 排队（下一轮 lineedit 消费）并继续监视。
/// EOF（读线程死）不构成取消意图：监视臂挂起（pending），回合自然收束。
/// 50ms 与 lineedit read_csi **同值同因**——孤立 Esc 消歧口径，改动需两侧
/// 同步（grep「50ms 同值」双向可查；云勘契约 B）。
async fn esc_interrupt(source: &InputSource) {
    loop {
        let Some(b) = source.read_channel_byte().await.ok().flatten() else {
            // EOF/读源死亡：不取消，挂起等回合自然结束（select 其他分支收）
            std::future::pending::<()>().await;
            unreachable!();
        };
        if b != 0x1B {
            // 回合期预打字节：回退排队（禁吞字），继续等 Esc
            source.pushback(&[b]);
            continue;
        }
        // 0x1B：50ms 窗口等后续——无后续=孤立 Esc（完成）；有后续=非孤立
        match tokio::time::timeout(
            std::time::Duration::from_millis(50),
            source.read_channel_byte(),
        )
        .await
        {
            // 内层 Result<Option<u8>>：Some=后续字节（非孤立，整组回退）；
            // None=后续通道 EOF（终端流已死，按孤立 Esc 处理）；Err=io 错
            // （读源异常，同按孤立 Esc 保守取消）；外层 Err=50ms 无后续=孤立。
            Ok(Ok(Some(next))) => {
                // 方向键/Alt 形态（0x1B+X）：整段回退（含 0x1B 本体，原序），
                // 继续监视——只退后续不退引导符=方向键丢引导符（P2-2 同型
                // 事故前科，云勘契约补刀二明示整组回退）
                source.pushback(&[0x1B, next]);
                continue;
            }
            Ok(Ok(None)) => return,
            Ok(Err(_)) => return,
            Err(_) => return, // 50ms 无后续=孤立 Esc（触发取消）
        }
    }
}

/// REPL 循环（任务29 C2-repl）：Editor 行编辑（补全面板）+注册表分发。
///
/// Editor 每轮从会话槽重建：commands 服务随 /new 换装配，持快照会在重建后
/// 指旧注册表（旧 root dispose 时壳命令已除名）——按轮取当前会话才正确。
pub async fn run_repl(
    slot: Arc<SessionSlot>,
    shell: Arc<ShellCore>,
    render: RenderConfig,
) -> Result<(), AppError> {
    let mut pending_expand: Option<String> = None;
    // task-73 A1：会话级 cbreak（ICANON off+ISIG 保留——信号安全留内核，裁决
    // 采纳形态）——守卫持至 run_repl 返回（Drop 恢复含 panic unwind 路径）。
    // 逐行 Editor 内层 enable=幂等重入（tcsetattr 同值无害）。
    let _session_cbreak = crate::lineedit::RawMode::enable();
    // 输入历史与面板提示标志（面板交互票①③）：REPL 级共享，Editor 每轮重建时
    // 以 Arc 传入（跨轮连续）
    let history = Arc::new(crate::history::LineHistory::new());
    let hint_shown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    loop {
        if shell.exit.is_cancelled() {
            break; // /quit 的 token（shell_commands 已接），结算落盘后到此处退出
        }
        let session = slot.load();
        let commands = session
            .root
            .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
            .map_err(AppError::Kernel)?;
        let editor = Editor {
            prompt: prompt_string(render.profile),
            profile: render.profile,
            commands: commands.clone(),
            commands_snapshot: std::sync::Arc::new(commands.list()), // task-36③：每轮一次
            exit: shell.exit.clone(),
            history: history.clone(),
            hint_shown: hint_shown.clone(),
            // task-66 P1-2：宽度默认 80（TERMINAL_WIDTH 锚点）；实测宽注入
            // 候 run.rs 会话接线批收敛后一行接入（云界 hunk 同文件错峰）。
            width: crate::panel_text::TERMINAL_WIDTH,
        };
        let input = if render.interactive {
            editor.read_line(pending_expand.take()).await
        } else {
            // 非交互保持既有行为：逐行前打提示符（剥色档无 ANSI，符号保留）
            print!("{}", prompt_string(render.profile));
            use std::io::Write;
            let _ = stdout().flush();
            editor.read_line_plain().await
        };
        match input.map_err(AppError::Io)? {
            LineInput::Submitted(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                history.commit(trimmed); // 交互票①：提交即入史（含命令行，shell 惯例）
                if trimmed.starts_with('/') {
                    // 命令分发（含回填后用户显式以 / 开头的意图，交接 §六-4）
                    match execute_command(&slot, trimmed).await {
                        CommandSettled::Action => {}
                        CommandSettled::Expand(text) => {
                            if render.interactive {
                                pending_expand = Some(text); // 回填输入框待用户确认
                            } else {
                                // 非交互无输入框可回填：上屏展示、不自动发送
                                //（设计 §七「用户确认后发送」不因管道而破例）
                                println!("{}", text);
                            }
                        }
                    }
                } else {
                    // task-74 修法 a：store 显式注入（lookup 前移到回合外——
                    // 上下文查找类失效一类根除；None=服务未装载 fail-loud 留痕）
                    let store = session
                        .root
                        .get::<dyn SessionStore>(ServiceId::of::<dyn SessionStore>())
                        .ok();
                    // task-73 A1：回合期回显旗 on（cbreak 内核不回显，读线程
                    // 即时回写可打印字节）；回合结束/取消即 off（行编辑期编辑器自绘）。
                    crate::input_source::set_echo(true);
                    let turn = run_dialogue_turn(&session, store, trimmed).await;
                    crate::input_source::set_echo(false);
                    if turn? {
                        // task-59a：对话期 Ctrl-C=取消 → 130（消息已由 handler 单点打印）
                        return Err(AppError::Interrupted);
                    }
                }
                }
                LineInput::Interrupted => {
                    // task-59a：空闲 Ctrl-C=取消信号 → 130（POSIX 惯例；静默，
                    // 无在屏消息可拼接）。EOF/ExitRequested 仍走 0（主动退出）。
                    println!(); // 防退出态黏在提示符后（与 Eof 分支同款）
                    return Err(AppError::Interrupted);
                }
                LineInput::Eof | LineInput::ExitRequested => {
                    println!(); // EOF/退出：补换行防退出态黏在提示符后
                    break;
                }
        }
    }
    Ok(())
}

/// 单任务模式：处理一句话即返回（Ctrl-C 中断本轮后返回）。
pub async fn run_single_task(
    task: &str,
    handler: LineHandler,
    store: Option<Arc<dyn SessionStore>>,
) -> Result<(), AppError> {
    // task-59c：空任务前置守卫——人话直出不走对话失败面（loop/空输入 内部码
    // 不再可能抵达用户；agent-loop 侧消息已同步人话化作纵深防御）。
    if task.trim().is_empty() {
        println!("（空任务：没有可处理的内容）");
        return Ok(());
    }
    let cancel = CancellationToken::new();
    let turn = handler(task, cancel.clone());
    tokio::select! {
        // task-36⑤：失败信令上抛=非零退出码（脚本可判别；人话已由 handler 输出）
        outcome = turn => {
            outcome?;
            if cancel.is_cancelled() {
                Err(AppError::Interrupted)
            } else {
                Ok(())
            }
        }
        _ = tokio::signal::ctrl_c() => {
            cancel.cancel();
            cancel_persist(&store, task).await;
            println!("（已取消本轮对话）");
            Err(AppError::Interrupted)
        }
    }
}

/// 单任务 Ctrl-C 落盘对账（task-25①）：与对话态同语义（store 显式注入+
/// SystemMessage「对话已取消（Ctrl-C）」，task-74 修法 a 同族）；user-message
/// 对账补写：run_turn 落盘在回合开头，Ctrl-C 极早窗口可能未及——扫尾部缺
/// 则补，防单任务槽文件出现「有取消记录无任务记录」的孤儿形态。竞态权衡
/// 自证：扫描与补写之间 run_turn 若又落盘（cancel 已发窗口极窄），重复
/// user 消息可审计，优于不可恢复的孤儿。store 缺席=服务未装载 fail-visible
/// 留痕（对齐 run_dialogue_turn 同分支形态）。
async fn cancel_persist(store: &Option<Arc<dyn SessionStore>>, task: &str) {
    let Some(store) = store else {
        eprintln!("[selfheal] 会话服务未装载，取消事件未落盘");
        return;
    };
    let need_user = match store.load_all().await {
        Ok(events) => !events
            .iter()
            .any(|e| matches!(e, yunyou_session::SessionEvent::UserMessage { .. })),
        Err(_) => true, // 读不出=按缺补（重复可审计，孤儿不可恢复）
    };
    if need_user {
        let _ = store
            .append(&yunyou_session::SessionEvent::UserMessage { text: task.to_string() })
            .await;
    }
    let _ = store
        .append(&yunyou_session::SessionEvent::SystemMessage {
            text: "对话已取消（Ctrl-C）".to_string(),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// task-25①：单任务 Ctrl-C 落盘对账——首次调用补 user-message+落
    /// system-message「对话已取消（Ctrl-C）」；重复调用（对账场景）不重复
    /// 补 user（尾部已有则跳过），只追加一条取消 system。
    #[tokio::test]
    async fn cancel_persist落盘对账() {
        let dir = std::env::temp_dir().join(format!("yunyou-cancelpersist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("task-slot.jsonl");
        let store: Arc<dyn yunyou_session::SessionStore> =
            yunyou_session::JsonlStore::new(&file);
        let opt = Some(Arc::clone(&store));
        // 首次：空会话 → 补 user-message（任务文本）+取消 system
        cancel_persist(&opt, "写一篇散文").await;
        let events = store.load_all().await.unwrap();
        assert_eq!(events.len(), 2, "首次应恰两事件: {events:?}");
        assert!(
            matches!(&events[0], yunyou_session::SessionEvent::UserMessage { text } if text == "写一篇散文"),
            "user-message 对账补写在位: {events:?}"
        );
        assert!(
            matches!(&events[1], yunyou_session::SessionEvent::SystemMessage { text } if text == "对话已取消（Ctrl-C）"),
            "取消 system 在位: {events:?}"
        );
        // 二次（对账场景：user 已由 run_turn 落）→ 不重复补 user
        cancel_persist(&opt, "写一篇散文").await;
        let events = store.load_all().await.unwrap();
        let user_count = events
            .iter()
            .filter(|e| matches!(e, yunyou_session::SessionEvent::UserMessage { .. }))
            .count();
        assert_eq!(user_count, 1, "对账不重复补 user: {events:?}");
        assert_eq!(events.len(), 3, "二次仅追加一条取消 system");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// store 缺席（None）路径：fail-visible 留痕不 panic、不落盘。
    #[tokio::test]
    async fn cancel_persist_store缺席留痕() {
        cancel_persist(&None, "任意任务").await; // eprintln 留痕，不 panic
    }

    /// task-34：esc_interrupt 三态——孤立 Esc（完成=取消）；0x1B+后续字节
    /// 整组回退（含 0x1B 本体）后继续监视（不完成）；普通字节回退后继续等。
    #[tokio::test]
    async fn esc监视臂孤立与整组回退() {
        use std::time::Duration;
        // 场景 A：孤立 Esc（0x1B 无后续）→ esc_interrupt 快速完成（=取消）
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tx.send(0x1B).await.unwrap(); drop(tx);
        let src = crate::input_source::InputSource::from_receiver(rx);
        tokio::time::timeout(Duration::from_millis(500), esc_interrupt(&src))
            .await
            .expect("孤立 Esc 应在 50ms 窗口后完成");
        // 场景 B：方向键形态（0x1B 0x5B 0x41）→ 整组回退（含 0x1B 本体）+
        // esc_interrupt 不完成（继续监视）——timeout 超时即「未触发取消」
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        for b in [0x1Bu8, 0x5B, 0x41] { tx.send(b).await.unwrap(); }
        drop(tx);
        let src = crate::input_source::InputSource::from_receiver(rx);
        let r = tokio::time::timeout(Duration::from_millis(400), esc_interrupt(&src)).await;
        assert!(r.is_err(), "方向键形态不应触发取消（整组回退后继续监视）");
        // 回退队列原序可读回（禁吞字）：0x1B 0x5B 0x41 逐字节原序
        for expected in [0x1Bu8, 0x5B, 0x41] {
            assert_eq!(src.read_byte().await.unwrap(), Some(expected), "回退序保持");
        }
        // 场景 C：普通字节（预打字符）回退后继续等——不触发取消
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tx.send(b'x').await.unwrap(); drop(tx);
        let src = crate::input_source::InputSource::from_receiver(rx);
        let r = tokio::time::timeout(Duration::from_millis(400), esc_interrupt(&src)).await;
        assert!(r.is_err(), "普通字节不应触发取消");
        assert_eq!(src.read_byte().await.unwrap(), Some(b'x'), "预打字节回退可见");
    }

    /// 提示符形态：与 `Renderer::prompt` 三档字节一致（平行构造防漂移的真锁——
    /// NoColor 档保留 reset 是 render 层既有语义，terminal.rs reset 实现）。
    #[test]
    fn 提示符串与renderer_prompt字节一致() {
        for profile in [Profile::Full, Profile::NoColor, Profile::Plain] {
            let expected = {
                let mut buf = Vec::new();
                Renderer::new(&mut buf, profile, true).prompt().expect("写入内存缓冲");
                buf
            };
            let actual = prompt_string(profile);
            assert_eq!(actual.as_bytes(), expected.as_slice(), "profile {profile:?} 形态漂移");
            assert!(actual.ends_with(' '), "尾空格分隔输入缓冲");
        }
    }
}
