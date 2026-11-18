//! market 插件代理装载体（L2 设计 §5.1 ProxyPlugin；K-19 刀2 批2/3/4）。

use crate::market_manifest::{self, MarketManifestV2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use yunyou::registry::{FiberOutput, Plugin};
use yunyou::{ConfigValue, Context, ServiceId};
use yunyou_tools::contract::{ToolDefinition, ToolError, ToolExec, ToolOutcome};
use yunyou_tools::RegistryService;
use yunyou_plugin_proto::{
    decode_frame, encode_frame, Hello, HelloErr, HelloOk, PluginProtoFrame, ToolDescriptor,
    ToolInvoke, ToolOutcome as PToolOutcome, ToolResult, ToolResultErr, ToolsList, ToolsListOk,
    PROTOCOL_VERSION,
};
use yunyou_util::BoxFut;


/// 装卸超时收割（P0 落刀）：按 pid=pgid 强杀子进程组（process_group(0) 前提）。
fn kill_stale_child(pid: &Option<u32>) {
    if let Some(pid) = pid {
        unsafe {
            libc::kill(-(*pid as i32), libc::SIGKILL);
        }
        eprintln!("[proxy] 超时收割子进程组 pid={pid}");
    }
}

/// spawn 形态（§4.1 沙箱衔接两路：Sandboxed=生产默认；Direct=测试缝——
/// 集成测试二进制无哨兵分派面，OS 级沙箱链由 task-24 forked 探针独立验证）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SpawnForm {
    #[default]
    Sandboxed,
    Direct,
}

/// 代理工具执行句柄（子进程 stdio 双通道共享；invoke_id 关联分发）。
/// child 归属=看护任务独占（waitpid 检测）；Drop 兜底=kill_on_drop（spawn
/// 设定）+process_group(0)=killpg 前提（批4 shutdown/killpg 扩展落点）。
pub struct ProxyHandle {
    /// 出站（宿主→插件帧）。
    pub tx: mpsc::Sender<PluginProtoFrame>,
    /// 入站分发注册表：invoke_id→oneshot（tool_result 等待方）。
    pub pending: Arc<Mutex<HashMap<String, oneshot::Sender<PluginProtoFrame>>>>,
    /// 子进程 pid（process_group(0)=pid 即 pgid；killpg 目标——看护任务登记）。
    pub child_pid: Arc<std::sync::Mutex<Option<u32>>>,
}

impl ProxyHandle {
    /// 发一帧并等待其 tool_result（invoke_id 关联；超时=软失败）。
    pub async fn invoke(
        &self,
        name: &str,
        args: &str,
        timeout_ms: u64,
    ) -> Result<ToolOutcome, String> {
        let invoke_id = format!(
            "inv-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let (rtx, rrx) = oneshot::channel();
        self.pending.lock().await.insert(invoke_id.clone(), rtx);
        self.tx
            .send(PluginProtoFrame::ToolInvoke(ToolInvoke {
                invoke_id: invoke_id.clone(),
                name: name.to_string(),
                args: args.to_string(),
            }))
            .await
            .map_err(|_| "插件进程已死亡（发送失败）".to_string())?;
        let frame = tokio::time::timeout(Duration::from_millis(timeout_ms), rrx)
            .await
            .map_err(|_| format!("插件调用超时（{timeout_ms}ms）——软失败计 fail-loud"))?
            .map_err(|_| "读泵已退出（插件进程死亡）".to_string())?;
        self.pending.lock().await.remove(&invoke_id);
        match frame {
            PluginProtoFrame::ToolResult(ToolResult { invoke_id: rid, outcome }) if rid == invoke_id => {
                Ok(match outcome {
                    PToolOutcome::Ok(ok) => {
                        // canonical 值：插件返回 JSON 文本→Value（解析失败=原文字符串值兜底）
                        let value = serde_json::from_str(&ok.value)
                            .unwrap_or(serde_json::Value::String(ok.value));
                        ToolOutcome::Ok(value)
                    }
                    PToolOutcome::Err(err) => ToolOutcome::Err(ToolError {
                        code: err.code,
                        message: err.message,
                        hint: err.hint,
                    }),
                })
            }
            other => Err(format!("非预期帧形态: {other:?}")),
        }
    }
}

/// 代理工具（tools_list_ok 描述符→ToolsRegistry 注册条目；execute=转发 invoke）。
struct ProxyTool {
    descriptor_name: String,
    description: String,
    parameters: serde_json::Value,
    output_schema: Option<serde_json::Value>,
    timeout_ms: u64,
    is_concurrency_safe: bool,
    handle: Arc<ProxyHandle>,
}

impl ToolDefinition for ProxyTool {
    fn name(&self) -> &str { &self.descriptor_name }
    fn description(&self) -> &str { &self.description }
    fn parameters(&self) -> serde_json::Value { self.parameters.clone() }
    fn output_schema(&self) -> Option<serde_json::Value> { self.output_schema.clone() }
    fn timeout_ms(&self) -> Option<u64> { Some(self.timeout_ms) }
    fn is_concurrency_safe(&self) -> bool { self.is_concurrency_safe }
    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let handle = Arc::clone(&self.handle);
        let name = self.descriptor_name.clone();
        let timeout = self.timeout_ms;
        Box::pin(async move {
            match handle.invoke(&name, &args.to_string(), timeout).await {
                Ok(outcome) => outcome,
                Err(reason) => ToolOutcome::Err(ToolError::new("tool/process-crashed", reason)),
            }
        })
    }
}

/// market 插件代理插件（§5.1：manifest=文件投影、setup=spawn+握手+tools_list+
/// 代理注册、子进程即执行体）。
pub struct ProxyPlugin {
    /// manifest v2（解析+校验过的 Ok 形态）。
    pub manifest_v2: MarketManifestV2,
    /// 插件目录（entry 相对锚+$SELF 展开）。
    pub plugin_dir: PathBuf,
    /// 沙箱 enforcer（Sandboxed 形态必填；None+Direct=测试缝）。
    pub enforcer: Option<Arc<dyn yunyou_tools::sandbox::SandboxEnforcer>>,
    /// spawn 形态（默认 Sandbled；生产恒 Sandbled——Direct 为测试缝显式声明）。
    pub spawn_form: SpawnForm,
    /// 会话消息投递回调（v1.1 真投递，task-14 续）：DeliverMessage→注入会话。
    /// 装配层注入（接 REPL 定时注入队列）；None=降级日志留痕+Ack ok（不悬挂插件）。
    pub deliver: Option<Arc<dyn Fn(Option<String>, String) -> Result<String, String> + Send + Sync>>,
}

impl Plugin for ProxyPlugin {
    fn name(&self) -> &str { &self.manifest_v2.id }

    fn manifest(&self) -> yunyou::manifest::PluginManifest {
        yunyou::manifest::PluginManifest {
            id: self.manifest_v2.id.clone(),
            version: self.manifest_v2.version.clone(),
            level: yunyou::manifest::PluginLevel::Market,
            contracts: self.manifest_v2.contracts.clone(),
            provides: vec![format!("market:{}", self.manifest_v2.id)],
            inject: vec![],
            ui: vec![],
            auto_enable: self.manifest_v2.auto_enable,
        }
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![ServiceId::of::<dyn RegistryService>()]
    }

    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, yunyou::Result<FiberOutput>> {
        Box::pin(async move {
            // ① spawn 形态分叉（§4.1 两路）：Direct=测试缝（entry 直 spawn，
            // 无哨兵——OS 级沙箱链由 task-24 forked 探针独立验证）；Sandboxed=
            // enforcer prepare 链（可写根=$SELF+声明项+tmp 并入；缺席=拒装
            // fail-loud——bash 降级语义不适用插件）
            let entry: Vec<String> = self.manifest_v2.runtime.entry.clone();
            use std::os::unix::process::CommandExt;
            let mut command = match self.spawn_form {
                SpawnForm::Direct => {
                    // 裸名入口（不含 /）相对插件目录解析：std 对含路径分隔符的
                    // program 才按 child cwd 解析，裸名会走 PATH 搜索=找不到
                    // 产物（task-10 自举实测锚）。判据=插件目录内存在同名文件
                    // 才补 ./（PATH 命令名如 python3 不误伤——p6_6 实测锚）。
                    let program = if entry[0].contains('/') {
                        entry[0].clone()
                    } else if self.plugin_dir.join(&entry[0]).is_file() {
                        format!("./{}", entry[0])
                    } else {
                        entry[0].clone()
                    };
                    let mut c = tokio::process::Command::new(&program);
                    c.args(&entry[1..]);
                    // §4.1 command_cwd 语义：entry 相对路径=相对插件目录——
                    // 子进程 cwd=插件目录（main.py 可解析的锚）
                    c.current_dir(&self.plugin_dir);
                    c
                }
                SpawnForm::Sandboxed => {
                    let Some(enforcer) = self.enforcer.as_ref() else {
                        return Err(yunyou::CordisError::new(
                            yunyou::ErrorKind::FiberFailed,
                            "沙箱禁用态不允许装载进程外插件（fail-loud；bash 降级语义不适用插件）",
                        ));
                    };
                    let policy = yunyou_tools::sandbox::WorkspacePolicy::v1(market_manifest::writable_roots(&self.manifest_v2, &self.plugin_dir));
                    let prepared = enforcer.prepare(&policy).map_err(|err| {
                        yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("沙箱 prepare 失败：{err}"))
                    })?;
                    let (program, argv) = prepared.helper_argv(&entry);
                    let mut c = tokio::process::Command::new(&program);
                    c.as_std_mut().arg0(&argv[0]);
                    c.args(&argv[1..]);
                    c
                }
            };
            command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = command.spawn().map_err(|err| {
                yunyou::CordisError::new(
                    yunyou::ErrorKind::FiberFailed,
                    format!("插件进程 spawn 失败（entry={entry:?}）：{err}"),
                )
            })?;
            // 装卸超时收割锚：pid=pgid（process_group(0)），超时按组强杀防残留
            let child_pid_for_timeout = child.id();
            let mut stdin = child.stdin.take().expect("stdin piped");
            let stdout = child.stdout.take().expect("stdout piped");
            eprintln!("[p6-diag] spawn ok: id={:?} stdin/stdout taken", child.id());

            // ④ 握手（hello→hello_ok/hello_err；版本不符=Failed 不静默降级）
            let hello = PluginProtoFrame::Hello(Hello {
                protocol_version: PROTOCOL_VERSION,
                capabilities: vec![],
            });
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            // 装卸超时（P0 协查落刀：握手/tools_list 两处 read_line 原无超时=
            // 「子进程活着不回帧」装配永挂；包 manifest 既有 request_timeout_ms，
            // 超时=Failed 人话+killpg 收割防残留子进程）
            let io_timeout = std::time::Duration::from_millis(self.manifest_v2.runtime.request_timeout_ms);
            stdin
                .write_all(encode_frame(&hello).map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("hello 编码：{e}")))?.as_bytes())
                .await
                .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("hello 写入：{e}")))?;
            stdin.write_all(b"\n").await.map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("hello 换行：{e}")))?;
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            eprintln!("[p6-diag] awaiting handshake line...");
            tokio::time::timeout(io_timeout, reader.read_line(&mut line))
                .await
                .map_err(|_| {
                    kill_stale_child(&child_pid_for_timeout);
                    yunyou::CordisError::new(
                        yunyou::ErrorKind::FiberFailed,
                        format!("插件握手超时（{}ms）——子进程活着但不回帧，已收割", io_timeout.as_millis()),
                    )
                })?
                .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("握手读取：{e}")))?;
            eprintln!("[p6-diag] handshake line: {}", line.trim());
            match decode_frame(line.trim()).map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("握手帧解码：{e}")))? {
                PluginProtoFrame::HelloOk(HelloOk { protocol_version, .. }) if protocol_version == PROTOCOL_VERSION => {}
                PluginProtoFrame::HelloErr(HelloErr { reason }) => {
                    return Err(yunyou::CordisError::new(
                        yunyou::ErrorKind::FiberFailed,
                        format!("插件握手拒绝：{reason}"),
                    ));
                }
                other => {
                    return Err(yunyou::CordisError::new(
                        yunyou::ErrorKind::FiberFailed,
                        format!("握手期非应答帧: {other:?}"),
                    ));
                }
            }

            // ⑤ tools_list→tools_list_ok（插件上报清单）
            stdin.write_all(encode_frame(&PluginProtoFrame::ToolsList(ToolsList)).unwrap().as_bytes()).await
                .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("tools_list 写入：{e}")))?;
            stdin.write_all(b"\n").await.map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("tools_list 换行：{e}")))?;
            line.clear();
            tokio::time::timeout(io_timeout, reader.read_line(&mut line))
                .await
                .map_err(|_| {
                    kill_stale_child(&child_pid_for_timeout);
                    yunyou::CordisError::new(
                        yunyou::ErrorKind::FiberFailed,
                        format!("插件工具清单超时（{}ms）——子进程活着但不回帧，已收割", io_timeout.as_millis()),
                    )
                })?
                .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("tools_list 读取：{e}")))?;
            let tools: Vec<ToolDescriptor> = match decode_frame(line.trim()).map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("tools_list 解码：{e}")))? {
                PluginProtoFrame::ToolsListOk(ToolsListOk { tools }) => tools,
                other => {
                    return Err(yunyou::CordisError::new(
                        yunyou::ErrorKind::FiberFailed,
                        format!("tools_list 期非应答帧: {other:?}"),
                    ));
                }
            };

            // ⑥ 读泵（行→帧→invoke_id 分发；空行/坏行跳过）+stderr drain（坑2）
            let pending: Arc<Mutex<HashMap<String, oneshot::Sender<PluginProtoFrame>>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let (tx, mut rx) = mpsc::channel::<PluginProtoFrame>(64);
            let pending_reader = Arc::clone(&pending);
            // v1.1 真投递捕获：回调（装配注入）+写通道 clone（回 Ack）+诊断 id
            let deliver_handler = self.deliver.clone();
            let pump_tx = tx.clone();
            let plugin_id_for_pump = self.manifest_v2.id.clone();
            tokio::spawn(async move {
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    if let Ok(frame) = decode_frame(line.trim()) {
                        match &frame {
                            PluginProtoFrame::ToolResult(r) => {
                                if let Some(sender) =
                                    pending_reader.lock().await.remove(&r.invoke_id)
                                {
                                    let _ = sender.send(frame);
                                    continue;
                                }
                            }
                            // v1.1 真投递（task-14 续）：DeliverMessage→回调注入会话
                            PluginProtoFrame::DeliverMessage(m) => {
                                let deliver_id = m.deliver_id.clone();
                                let result = match deliver_handler.as_ref() {
                                    Some(handler) => {
                                        let session = m.session_id.clone();
                                        let text = m.text.clone();
                                        handler(session, text)
                                    }
                                    None => Ok("投递面未接线（日志留痕）".into()),
                                };
                                let (ok, detail) = match result {
                                    Ok(route) => (true, route),
                                    Err(err) => (false, err),
                                };
                                eprintln!(
                                    "[plugin:{plugin_id_for_pump}] 投递 {deliver_id}: ok={ok} {detail}"
                                );
                                let _ = pump_tx
                                    .send(PluginProtoFrame::DeliverAck(
                                        yunyou_plugin_proto::DeliverAck {
                                            deliver_id,
                                            ok,
                                            detail,
                                        },
                                    ))
                                    .await;
                            }
                            _ => {}
                        }
                    }
                }
            });
            tokio::spawn(async move {
                while let Some(frame) = rx.recv().await {
                    if let Ok(encoded) = encode_frame(&frame) {
                        if stdin.write_all(encoded.as_bytes()).await.is_err()
                            || stdin.write_all(b"\n").await.is_err()
                        {
                            break;
                        }
                    }
                }
            });

            // ⑦ stderr drain（坑2：独立任务逐行前缀透传主板 stderr，防 64KB 管道满死锁）
            let mut stderr = child.stderr.take().expect("stderr piped");
            let plugin_id = self.manifest_v2.id.clone();
            let plugin_id_for_watch = plugin_id.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = vec![0u8; 4096];
                loop {
                    match stderr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let text = String::from_utf8_lossy(&buf[..n]);
                            for l in text.lines() {
                                eprintln!("[plugin:{plugin_id_for_watch}] {l}");
                            }
                        }
                    }
                }
            });

            // ⑦' child 死亡看护（§5.2 MVP 看护：wait 检测→进程死亡→pending 全清
            // +各等待方收 tool/process-crashed 软失败——等待方不悬挂；代理 fiber
            // 的 Failed 态由下一次 invoke 的 Err 路径自然触发=Failed 可 restart()
            // 复活（懒恢复语义 §5.1）
            let child_pid_slot: Arc<std::sync::Mutex<Option<u32>>> =
                Arc::new(std::sync::Mutex::new(child.id()));
            let handle = Arc::new(ProxyHandle { tx, pending, child_pid: Arc::clone(&child_pid_slot) });
            // ⑧ 卸载 effect（契约①注册即 effect 逆序撤销）：shutdown 帧（尽力
            // 而为——不回执也杀，不悬挂）+killpg 进程组强杀（§4.3 进程层三层
            // 清单第一层）。effect 契约=同步 FnOnce 返回 Result<Option<Disposer>>
            // ——shutdown 帧的异步投递转独立任务（不等待回执=不悬挂语义），
            // killpg 同步直杀（进程组收割即时完成）。
            let disposer_handle = Arc::clone(&handle);
            let disposer_id = plugin_id.clone();
            // 修复（task-10 自举实测锚）：shutdown+killpg 必须是 **Disposer**（卸载
            // 时执行），此前误作 effect 注册体=注册瞬间立即 shutdown+killpg（插件
            // 刚握手完即被杀，工具全废）。effect 注册体只登记清理器本体。
            ctx.effect("插件卸载 shutdown+killpg", move || {
                let handle = Arc::clone(&disposer_handle);
                let disposer = yunyou::Disposer::new(move || {
                    let handle = Arc::clone(&handle);
                    let disposer_id = disposer_id.clone();
                    Box::pin(async move {
                        // shutdown 帧：异步投递（独立任务，不等回执=不悬挂）
                        let shutdown_tx = handle.tx.clone();
                        tokio::spawn(async move {
                            let _ = shutdown_tx
                                .send(PluginProtoFrame::Shutdown(yunyou_plugin_proto::Shutdown))
                                .await;
                        });
                        // killpg 进程组强杀（同步；pid=pgid 由 process_group(0) 设定）
                        #[cfg(unix)]
                        {
                            let pid = handle.child_pid.lock().map(|slot| *slot).unwrap_or(None);
                            if let Some(pid) = pid {
                                unsafe {
                                    libc::kill(-(pid as i32), libc::SIGKILL);
                                }
                            }
                        }
                        eprintln!("[plugin:{disposer_id}] 卸载：shutdown 帧已投递+进程组已收割");
                    })
                });
                Ok(Some(disposer))
            })?;

            // ⑨ 代理工具注册（ToolsRegistry；disposer=契约①注册即 effect 逆序撤销）
            let registry = ctx
                .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
                .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("registry 获取：{e}")))?;
                        let _timeout = self.manifest_v2.runtime.request_timeout_ms;
            for descriptor in tools {
                let params_value = serde_json::from_str(&descriptor.parameters)
                    .unwrap_or(serde_json::json!({"type":"object"}));
                let out_value: Option<serde_json::Value> =
                    descriptor.output_schema.as_ref().and_then(|s| serde_json::from_str(s).ok());
                let proxy = ProxyTool {
                    descriptor_name: descriptor.name.clone(),
                    description: descriptor.description.clone(),
                    parameters: params_value,
                    output_schema: out_value,
                    timeout_ms: descriptor.timeout_ms.unwrap_or(_timeout),
                    is_concurrency_safe: descriptor.is_concurrency_safe,
                    handle: Arc::clone(&handle),
                };
                let tool_disposer = Arc::clone(&registry)
                    .register_tool(Arc::new(proxy))
                    .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::FiberFailed, format!("代理工具注册：{e}")))?;
                // 契约①「注册即 effect，逆序撤销」：Disposer 挂 ctx.effect 存活
                // ——丢弃即注销（回归债实证：Disposer 丢弃→工具即撤→tx 链断→
                // 泵退→stdin EOF→SDK 退出→死亡帧误发）。effect 存续=工具存续
                // =插件 fiber 生命周期。
                ctx.effect(
                    format!("market 工具 {} 注销", descriptor.name),
                    move || {
                        drop(tool_disposer);
                        Ok(None)
                    },
                )?;
            }

            let death_pending = Arc::clone(&handle.pending);
            let plugin_id_for_death = plugin_id.clone();
            let child_for_watch = child;
            tokio::spawn(async move {
                let mut child = child_for_watch;
                let status = child.wait().await;
                eprintln!("[plugin:{plugin_id_for_death}] 进程退出: {status:?}——pending 全清（等待方收 process-crashed）");
                let mut map = death_pending.lock().await;
                for (_, sender) in map.drain() {
                    let _ = sender.send(PluginProtoFrame::ToolResult(ToolResult {
                        invoke_id: String::new(),
                        outcome: PToolOutcome::Err(ToolResultErr {
                            code: "tool/process-crashed".into(),
                            message: "插件进程死亡（调用中死亡归一，§5.2）".into(),
                            hint: Some("restart() 复活后重试".into()),
                        }),
                    }));
                }
            });

            Ok(FiberOutput)
        })
    }
}
