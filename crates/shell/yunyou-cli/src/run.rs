//! 总控流程：解析 home → 加载配置 → §2.5 装配 → 就绪标记 → 横幅 → REPL/单任务 → 优雅关闭。

use crate::render::Profile;
use crate::{assemble, cli, ready, repl, sessions_cmd, shell_commands, skills};
use yunyou_config::DshConfig;
use yunyou_session::SessionStore;
use std::fmt;
use std::io::{IsTerminal, Write, stdout};
use std::path::{Path, PathBuf};

/// 外壳层错误：配置 / 内核 / I/O / 装配四类，Display 全中文可直读。
#[derive(Debug)]
pub enum AppError {
    /// yunyou.toml 加载或校验失败（含内核 ValidationError 聚合）。
    Config(yunyou_config::ConfigError),
    /// 内核操作失败（dispose 等）。
    Kernel(yunyou::CordisError),
    /// 文件系统 I/O 失败。
    Io(std::io::Error),
    /// 组件装配失败（含组件名与内核错误链）。
    Assemble(String),
    /// 对话回合失败（人话消息已就绪，展示层直出；task-36⑤ 失败信令）。
    Turn(String),
    /// 用户主动中断（Ctrl-C；单任务被中断=任务未完成，退出码非零）。
    Interrupted,
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(err) => write!(f, "配置错误：{err}"),
            Self::Kernel(err) => write!(f, "内核错误：{err}"),
            Self::Io(err) => write!(f, "IO 错误：{err}"),
            Self::Assemble(msg) => write!(f, "启动失败：{msg}"),
            Self::Turn(msg) => write!(f, "对话失败：{msg}"),
            Self::Interrupted => write!(f, "本轮对话已取消（任务未完成）"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(err) => Some(err),
            Self::Kernel(err) => Some(err),
            Self::Io(err) => Some(err),
            // Turn/Interrupted：人话消息即全部语义，无底层错误链可挂
            Self::Turn(_) | Self::Interrupted => None,
            Self::Assemble(_) => None,
        }
    }
}

impl From<yunyou_config::ConfigError> for AppError {
    fn from(err: yunyou_config::ConfigError) -> Self {
        Self::Config(err)
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// 进程总控：成功返回 Ok；错误一律带中文上下文上抛（main 打印到 stderr）。
pub async fn run(command: crate::cli::Cli) -> Result<(), AppError> {
    // ① yunyou-home：--home > $YUNYOU_HOME > ~/.yunyou；先建目录（run/ 与 sessions 都在其下）
    let home = cli::resolve_home(command.home.clone());
    std::fs::create_dir_all(&home).map_err(AppError::Io)?;

    // ⓪' 自更新子命令（task-18）：纯网络+文件编排不进装配——早分派直返
    if let Some(cli::Command::Update { check, from_source }) = &command.command {
        let msg = crate::update::run_update(*check, *from_source).map_err(AppError::Assemble)?;
        println!("{msg}");
        return Ok(());
    }

    // ⓪ 插件管理子命令（task-10）：纯文件编排不进装配——早分派直返
    if let Some(cli::Command::Plugin { cmd }) = &command.command {
        let msg = match cmd {
            cli::PluginCmd::New { name, category, dir } => {
                crate::plugin_cmd::scaffold_new(name, category, dir.as_deref())
                    .map_err(AppError::Assemble)?
            }
            cli::PluginCmd::Install { path } => {
                crate::plugin_cmd::install(path, &home).map_err(AppError::Assemble)?
            }
            cli::PluginCmd::Check { path } => {
                println!("{}", crate::plugin_cmd::check(path));
                return Ok(());
            }
        };
        println!("{msg}");
        return Ok(());
    }

    // ② 配置：显式 --config 缺失即报错；隐式默认路径可回退全默认（联调一条命令启动）
    let config_path: PathBuf = command.config.clone().unwrap_or_else(|| home.join("yunyou.toml"));
    let config = load_config(&config_path, command.config.is_some())?;

    // ②-b 单任务模式独立会话槽（task-36②）：--task 是一次性任务，装配前即把
    // 会话路径换为 `task-<unix_ms>-<pid>.jsonl`——default.jsonl 不再被单任务
    // 读写，被打断残留的孤儿 user-message 不再进入后续任务上下文（压测实录：
    // 多次 --task 的孤儿消息全量累积进请求，模型自述「three identical messages」）。
    // 会话延续需求走 REPL；task 会话文件仍落盘（审计/回放语义不变）。
    let config = {
        let mut config = config;
        // task-71①：工作区根三段链解析（[workspace] root→git 根→启动目录兜底）——
        // 装配期一次性喂 ToolsConfig.root_dir（canonical 形态），会话稳定不重检。
        let ws_root = yunyou_tools::builtin::cwd::resolve_workspace_root(
            config.workspace.root.as_deref(),
            &std::env::current_dir().unwrap_or_else(|_| home.clone()),
        );
        config.tools.root_dir = ws_root.display().to_string();
        if command.task.is_some() {
            let task_path =
                home.join(format!("sessions/task-{}-{}.jsonl", unix_millis(), std::process::id()));
            config.session.path = Some(task_path.display().to_string());
        }
        config
    };

    // ④-a 启动会话选取（task-9 语义对齐 claude code）：
    // 默认=**全新会话**（时间戳+短 id 新文件，多开各写各文件、锁冲突自然化解）；
    // --resume 无参=交互选择历史会话（对齐 claude -r）、有参=按名/序号/路径恢复；
    // --continue=续最近一条（对齐 claude -c）；显式 session.path 配置与 --task
    // 独立槽语义不变（优先级最高=尊重显式指令）。flock 联动：恢复路径经装配期
    // session 插件抢锁（task-36①），真撞锁时错误信息给三条出路（lock.rs task-9）。
    let restored: Option<PathBuf> = if command.task.is_some() || config.session.path.is_some() {
        None
    } else if let Some(resume) = &command.resume {
        match resume.as_deref() {
            None | Some("") => sessions_cmd::pick_interactively(&home),
            Some(target) => Some(
                sessions_cmd::resolve_resume_target(target, &home).map_err(AppError::Turn)?,
            ),
        }
    } else if command.continue_recent {
        match sessions_cmd::pick_latest_session(&home) {
            Some(path) => Some(path),
            None => {
                eprintln!("尚无历史会话——本次启动全新会话。");
                None
            }
        }
    } else {
        None
    };
    let config = {
        let mut config = config;
        // task-71①：工作区根三段链解析（[workspace] root→git 根→启动目录兜底）——
        // 装配期一次性喂 ToolsConfig.root_dir（canonical 形态），会话稳定不重检。
        let ws_root = yunyou_tools::builtin::cwd::resolve_workspace_root(
            config.workspace.root.as_deref(),
            &std::env::current_dir().unwrap_or_else(|_| home.clone()),
        );
        config.tools.root_dir = ws_root.display().to_string();
        if let Some(path) = &restored {
            config.session.path = Some(path.display().to_string());
        } else if command.task.is_none() && config.session.path.is_none() {
            // task-9 默认新会话：全新时间戳文件（各实例各写各的，锁冲突自然化解）
            config.session.path = Some(new_session_path(&home).display().to_string());
        }
        config
    };

    // ④-a-2 撞锁预检+--fork 出路（task-9）：目标会话已被另一实例持有——
    // --fork=复制该会话为新会话再启动（不改别人正用的文件）；否则交装配期
    // 抢锁失败（lock.rs 人话错误已给三条出路），此处不再重复报错。
    let mut config = config;
    {
        let target = current_session_path(&config, &home);
        if command.task.is_none()
            && command.fork
            && yunyou_session::ExclusiveLock::is_locked(&target)
        {
            let copy_path = new_session_path(&home);
            std::fs::copy(&target, &copy_path).map_err(|err| {
                AppError::Io(std::io::Error::new(
                    err.kind(),
                    format!("复制会话 {} → {} 失败：{err}", target.display(), copy_path.display()),
                ))
            })?;
            eprintln!(
                "[fork] 会话 {} 正被另一实例使用，已复制为新会话 {}（原会话不受影响）",
                target.display(),
                copy_path.display()
            );
            config.session.path = Some(copy_path.display().to_string());
        }
    }

    // ③ §2.5 装配序列（失败路径内部已整树卸干净；home 供会话缺省路径补值）
    let mut assembled = assemble::assemble(&config, &home, &config_path).await?;

    // ③-b 插件状态表（方案 §2.4 第 5 步观测面；任务 30-⑤ 启动净化：--verbose 才出）
    if command.verbose {
        print!("{}", crate::status::render_status_table(&assembled.plugin_status));
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }

    // ④ 就绪标记：装配完成即创建（launcher/网关阶段的启动判定观测点）
    let mark = match ready::ReadyMark::create(&home) {
        Ok(mark) => mark,
        Err(err) => {
            let _ = assembled.root.dispose().await;
            return Err(err.into());
        }
    };

    // ⑤ 启动横幅（§4.1，装配完成后输出一次；会话目录即 session.path 解析值）
    let render_config = repl::RenderConfig {
        profile: Profile::detect(stdout().is_terminal()),
        interactive: stdout().is_terminal() && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true),
        model: config.llm.model.clone(),
        // 思考流模式（任务 30-②）：YUNYOU_THINKING=full 保留全文流式逃生口，默认收敛
        thinking: repl::ThinkingMode::from_env(),
    };
    // task-34② 冷启动预检：api_key 空=「端点不要求鉴权」语义直发必 401——横幅
    // 追加一条配置指路（不阻塞启动；引用型无值在装配期已 fail-loud 人话报错，
    // 走到横幅的空 key 只有「完全没配」一种，判空即可不探测端点）。
    let credential_notice = if config.llm.api_key.is_empty() {
        Some(format!(
            "API key 未配置：在 {} 写 api_key = \"$YUNYOU_LLM_API_KEY\" 并设置同名环境变量（详见 --help）",
            config_path.display()
        ))
    } else {
        None
    };
    let mut banner_renderer = repl_render(&render_config);
    banner_renderer.banner(&config.llm.model, &session_dir(&config, &home), credential_notice.as_deref(), command.task.is_none())?;
    // task-27①：横幅后显式空行——prompt_string 前置换行移除后，首提示符的
    // 间距由横幅侧承担（回合后提示符紧贴落款，首提示符与横幅仍空一行）。
    println!();
    // ④-b 恢复提示行+尾部回放（task-36④；task-19① 收编）：恢复行走 render 域
    // 单源 resume_header（☁ 品牌符+profile 降级体系，废除裸 \x1b[90m 与
    // 「（N 条记录）」漂移句式——挑刺报告 P1-2/P2-4，孤岛函数复活）；仅 REPL
    // 且确有恢复时出现。load_all 只读不抢锁（锁在装配期）。
    if let Some(path) = &restored {
        let events = yunyou_session::JsonlStore::new(path).load_all().await.unwrap_or_default();
        let count = events.len();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = banner_renderer.resume_header(&name, count);
        let mut out = banner_renderer.into_output();
        if let Err(err) = sessions_cmd::render_replay(&events, &mut out, render_config.profile) {
            eprintln!("会话回放渲染失败：{err}");
        }
        // task-74 v5：回放装载（只读）后续聊升级——写路径 fail-loud 预检
        //（升级失败当场报，对话写不进去不静默）。flock 在装配期已持（重开
        // 文件不重锁，锁在 store 生命周期）。
        let live_store = assembled.root.get::<dyn yunyou_session::SessionStore>(
            yunyou::ServiceId::of::<dyn yunyou_session::SessionStore>(),
        );
        if let Ok(store) = live_store {
            if let Err(err) = store.ensure_writable().await {
                let _ = assembled.root.dispose().await;
                return Err(AppError::Assemble(format!("续聊写模式升级失败：{err}")));
            }
        }
        let _ = stdout().flush();
    }
    let _ = stdout().flush();
    // 就绪标记路径：运维观测点走 stderr 且 --verbose 才输出（任务 30-⑤ 启动净化：
    // 启动输出最小化，观测信息让位于用户内容；mark 文件照建，观测点不受影响）
    if command.verbose {
        eprintln!("就绪标记：{}", mark.path().display());
    }

    // ⑥ REPL / 单任务模式（对话经 run_turn_observed 实时渲染；命令经注册表分发）
    // task-74 修法 a/v3：store handle 显式注入（装配点一次 lookup——上下文
    // 查找类失效一类根除；回放态 handler 层死实例的根除面）。
    let repl_store = assembled
        .root
        .get::<dyn yunyou_session::SessionStore>(yunyou::ServiceId::of::<dyn yunyou_session::SessionStore>())
        .ok();
    let handler = repl::loop_line_handler(assembled.root.clone(), render_config.clone(), repl_store);
    let exit = tokio_util::sync::CancellationToken::new();
    let slot = shell_commands::SessionSlot::new(shell_commands::ActiveSession {
        root: assembled.root.clone(),
        handler,
        plugin_status: assembled.plugin_status,
    });
    let shell = std::sync::Arc::new(shell_commands::ShellCore {
        config: config.clone(),
        home: home.clone(),
        config_path: config_path.clone(),
        session_dir: session_dir_path(&config, &home),
        slot: slot.clone(),
        render_config: render_config.clone(),
        current_session_path: std::sync::Mutex::new(current_session_path(&config, &home)),
        exit: exit.clone(),
    });
    // 壳三命令走 register 缝（装配序列零硬编码；重复注册 fail-loud）
    shell_commands::register_shell_commands(&assembled.root, shell.clone())
        .map_err(AppError::Kernel)?;
    // 技能→命令注册（任务29 C4，设计 §七）：项目根双槽扫描，壳侧职责；
    // 注册数按 --verbose 门控输出（任务 30-⑤ 启动净化纪律）
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
    let skill_count = skills::register_skill_commands(&assembled.root, &cwd);
    if command.verbose && skill_count > 0 {
        eprintln!("[技能] 已注册 {skill_count} 条技能命令（.yunyou/skills + .dsh/skills 双槽）");
    }
    // 技能渐进披露（池 task-13）：①目录段常驻（harness:skills，名+描述）+
    // ②skill_read 按需加载工具；两注册均观测返回值，--verbose 门控输出
    let catalog = skills::register_skill_catalog(&assembled.root, &cwd);
    let skill_read = crate::skill_read::register_skill_read_tool(&assembled.root, &cwd);
    if command.verbose && (catalog || skill_read) {
        eprintln!("[技能] 渐进披露：目录段={} skill_read={}", catalog, skill_read);
    }
    // memory.v1 记忆缝（池 task-14）：四操作工具 + MEMORY.md 活索引常驻注入
    //（复用 harness:skills 目录段机制；空索引不注入=零噪音）
    let memory_store = std::sync::Arc::new(crate::memory::MemoryStore::new(&home, &cwd));
    let memory_tools = crate::memory::register_memory_tools(&assembled.root, std::sync::Arc::clone(&memory_store));
    let memory_section = crate::memory::register_memory_section(&assembled.root, &memory_store);
    if command.verbose && (memory_tools > 0 || memory_section) {
        eprintln!("[记忆] memory.v1：工具 {} 个 索引段={}", memory_tools, memory_section);
    }
    let outcome = match command.task.as_deref() {
        Some(task) => {
            // task-25①：store 显式注入（task-74 修法 a 同族——上下文查找类
            // 失效一类根除；None=服务未装载 fail-visible 留痕）
            let store = assembled
                .root
                .get::<dyn SessionStore>(yunyou::ServiceId::of::<dyn SessionStore>())
                .ok();
            repl::run_single_task(task, slot.load().handler.clone(), store).await
        }
        // Plugin 子命令已在上游 ⓪ 早分派直返（不进装配）
        None => match &command.command {
            // web 启动形态（云幕，业主令）：并列前端——同一装配/会话/agent-loop。
            // root 经会话槽回调取（/new 换槽自动跟新装配）；服务收场后走统一 dispose（⑦）
            Some(cli::Command::Web { addr }) => {
                let params = yunyou_web::WebParams {
                    root_getter: {
                        let slot = slot.clone();
                        std::sync::Arc::new(move || slot.load().root.clone())
                    },
                    session_dir: session_dir_path(&config, &home),
                    model: config.llm.model.clone(),
                    addr: addr.clone(),
                };
                yunyou_web::serve(params).await.map_err(|err| AppError::Assemble(err.to_string()))
            }
            Some(cli::Command::Plugin { .. }) => unreachable!("plugin 子命令已于 ⓪ 早分派直返"),
            Some(cli::Command::Update { .. }) => unreachable!("update 子命令已于 ⓪' 早分派直返"),
            None => {
                // 定时注入接收端（proto v1.1 真投递）：market 插件到期→lineedit 注入臂
                let scheduled_rx = assembled.scheduled_rx.take();
                repl::run_repl(slot, shell, render_config.clone(), scheduled_rx).await
            }
        },
    };

    // ⑦ 优雅关闭：先摘就绪标记（观测者立刻感知不再接活），再整树逆序卸载
    //    （先消费者后提供者：REPL/agentLoop → tools → systemPrompt → sessions → llm）
    mark.remove();
    if let Err(err) = assembled.root.dispose().await {
        // 关闭阶段失败不吞主结果，仅告警（主结果可能是正常退出）
        eprintln!("yunyou 关闭警告：{err}");
    }
    outcome
}

/// 当前 UNIX 毫秒（task 会话文件名去重用；同毫秒双开由 pid 兜底唯一）。
fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// 新会话文件路径（task-9 默认新会话）：`sessions/<YYYYmmdd-HHMMSS>-<4位hex>.jsonl`
/// ——人名可读（/sessions 列表即时间序）+毫秒/pid 混合短 id 防同秒双开重名；
/// 极小概率重名再补 pid 后缀兜底（存在即重生成，循环有界）。
fn new_session_path(home: &Path) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    // 时间戳转本地格式：一期不做时区库，直接用 UTC 分解（文件名是唯一性载体，
    // 可读性需求由 mtime 列满足；注释自证不为伪装本地时间）
    let days = secs / 86400;
    let tod = secs % 86400;
    let (h, m, s) = (tod / 3600, tod % 3600 / 60, tod % 60);
    // 民用日期由 days 反推（1970-01-01 起；Civil-from-days 算法，纯整数无库）
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    let dir = home.join("sessions");
    let _ = std::fs::create_dir_all(&dir);
    let short_id = format!("{:04x}", (std::process::id() as u32 ^ (millis as u32).wrapping_mul(2654435761)) & 0xffff);
    let base = format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}-{short_id}");
    let mut path = dir.join(format!("{base}.jsonl"));
    let mut bump = 0u8;
    while path.exists() && bump < 64 {
        bump += 1;
        path = dir.join(format!("{base}-{bump}.jsonl"));
    }
    path
}

/// 当前活跃会话文件路径（task-36④：ShellCore.current_session_path 初始化值）。
/// 配置显式 path 优先；缺省补值逻辑与装配 with_default_paths 同口径。
fn current_session_path(config: &DshConfig, home: &Path) -> PathBuf {
    PathBuf::from(config.session.path.clone().unwrap_or_else(|| {
        home.join("sessions/default.jsonl").display().to_string()
    }))
}

/// 会话文件目录（/new 的新文件落点）：配置显式 path 的父目录或缺省补值后的目录。
fn session_dir_path(config: &DshConfig, home: &Path) -> PathBuf {
    let path = config.session.path.clone().unwrap_or_else(|| home.join("sessions/default.jsonl").display().to_string());
    std::path::Path::new(&path)
        .parent()
        .map(|dir| dir.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(&path))
}

/// 会话目录展示值（横幅用）：同 [`session_dir_path`] 的字符串形态。
fn session_dir(config: &DshConfig, home: &Path) -> String {
    session_dir_path(config, home).display().to_string()
}

/// 配置加载三态：显式路径缺失 fail-loud；隐式默认路径存在则加载、
/// 缺失则回退默认配置（+ env 覆盖，联调「一条命令启动」的兜底）。
fn load_config(path: &Path, explicit: bool) -> Result<DshConfig, AppError> {
    if explicit || path.exists() {
        yunyou_config::load(path).map_err(AppError::Config)
    } else {
        yunyou_config::load_optional(None).map_err(AppError::Config)
    }
}

/// 建横幅渲染器（stdout；横幅是一次性输出，与对话渲染器互不共享状态）。
fn repl_render(render_config: &repl::RenderConfig) -> crate::render::Renderer<std::io::Stdout> {
    crate::render::Renderer::new(stdout(), render_config.profile, render_config.interactive)
}
