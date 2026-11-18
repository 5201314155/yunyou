//! web 服务实例生命周期（任务40 期间自 server.rs 拆出——行数纪律：server.rs
//! 本就超限不得再加新功能；职责分家：本文件管「起停」，server.rs 管「路由」）。
//!
//! 铸造 token → 建态（广播+wire 桥+回合执行器）→ 绑定 → 打印访问地址 →
//! 后台 serve。serve 与测试共用的底座（测试拿真实 addr/token 打真请求，
//! 不 mock 通道）。

use crate::bridge::WireBridge;
use crate::error::WebError;
use crate::server::{router, WebState};
use crate::token;
use crate::turns::TurnRunner;
use axum::Router;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use yunyou::Context;

/// web 服务参数（run.rs 接线面；字段均为值/句柄，可廉价构造）。
pub struct WebParams {
    /// 当前会话装配 root 的获取口：/new 换槽后取新 root（run.rs 从会话槽取；
    /// 经回调注入是依赖方向裁决——本 crate 不依赖 yunyou-cli，防循环）。
    pub root_getter: Arc<dyn Fn() -> Context + Send + Sync>,
    /// 会话目录（sessions/*.jsonl 列表与历史回放，P4 消费）。
    pub session_dir: PathBuf,
    /// 模型名（横幅与状态落款，P2 消费）。
    pub model: String,
    /// 监听地址 host:port（None = 127.0.0.1:0 随机端口，冲突免疫；LAN 暴露需显式指定）。
    pub addr: Option<String>,
    /// 宿主定时注入缝（assemble scheduled_rx；W4 消费臂——None=无宿主调度）。
    pub scheduled_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    /// 配置文件路径（W5 settings 修订写落点；run.rs 传实际 yunyou.toml 路径）。
    pub config_path: std::path::PathBuf,
}

/// 运行中的 web 服务实例（start_instance 产物；测试与嵌入方持有真实地址与令牌）。
pub struct WebInstance {
    /// 实际绑定地址（端口 0 时由内核分配后回填）。
    pub addr: SocketAddr,
    /// 本进程访问令牌（与启动打印一致）。
    pub token: String,
    /// 停止令牌（cancel 即优雅收场）。
    pub stop: CancellationToken,
    /// serve 任务句柄（join 得 IO 结果）。
    pub task: tokio::task::JoinHandle<std::io::Result<()>>,
}

/// 进程级入口：启动 → Ctrl-C 收场。
///
/// web 语义里 /quit = 断开连接（WS 层处置，不落此函数）——杀服务是终端
/// Ctrl-C 的职权；P3 起如需并听壳退出令牌在此扩展。
pub async fn serve(mut params: WebParams) -> Result<(), WebError> {
    let instance = start_instance(&mut params).await?;
    tokio::signal::ctrl_c().await?;
    instance.stop.cancel();
    // 收场阶段的 IO 错误不再上抛（进程退出在即，报了也没人接）
    let _ = instance.task.await;
    Ok(())
}

/// 启动实例：铸造 token → 建态 → 绑定 → 打印访问地址 → 后台 serve。
pub async fn start_instance(params: &mut WebParams) -> Result<WebInstance, WebError> {
    let token = token::mint();
    let addr = resolve_addr(params.addr.as_deref())?;
    // 广播容量：Delta 帧高频，256 条缓冲吸收瞬时积压（滞后由连接层人话告警）
    let (broadcast_tx, _) = broadcast::channel(256);
    // 定时调度器（W4）：宿主持有+触发器挂接（到期→回合注入）+1s tick 线程
    //（进程退出即焚；durable 持久化未闭环=会话内有效，诚实投影）
    let scheduler = Arc::new(yunyou_plugin_sdk::scheduler::Scheduler::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }));
    let state = Arc::new(WebState {
        token,
        model: params.model.clone(),
        turns: Arc::new(TurnRunner::new(params.root_getter.clone(), params.model.clone())),
        session_dir: params.session_dir.clone(),
        wire: Arc::new(WireBridge::new(params.model.clone())),
        config_store: Arc::new(crate::config_store::ConfigStore::new(params.config_path.clone())),
        loopback: addr.ip().is_loopback(),
        scheduler: {
            scheduler.set_trigger(Arc::new(crate::schedule_fire::TurnFireTrigger {
                turns: Arc::new(TurnRunner::new(params.root_getter.clone(), params.model.clone())),
                broadcast: broadcast_tx.clone(),
            }));
            scheduler.spawn_thread();
            scheduler
        },
        broadcast: broadcast_tx,
    });
    // 定时注入消费臂（W4，lead 批）：宿主缝 scheduled_rx → 回合闸门内注入
    // 活跃会话（事件流自然回显）。闸门占用=人话日志+5s 退避重试（不排队伪装、
    // 防忙会话期打转刷屏）；其他失败=留痕放弃本条（miss 顺延语义）。
    if let Some(mut scheduled_rx) = params.scheduled_rx.take() {
        let turns = state.turns.clone();
        let broadcast = state.broadcast.clone();
        tokio::spawn(async move {
            while let Some(text) = scheduled_rx.recv().await {
                loop {
                    match turns.run(&text, broadcast.clone()).await {
                        Ok(()) => break,
                        Err(err) if err.contains("正在对话中") => {
                            eprintln!("[定时注入] 回合占用，5s 后重试：{text}");
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                        Err(err) => {
                            eprintln!("[定时注入] 注入失败（miss 顺延放弃本条）: {err}");
                            break;
                        }
                    }
                }
            }
        });
    }
    start_with_state(addr, state).await
}

/// 持态启动（测试与 start_instance 共用底座）：传入的 state 即服务态——
/// 测试经同一 broadcast/wire 桥驱动事件流，不 mock 通道。
pub(crate) async fn start_with_state(
    addr: SocketAddr,
    state: Arc<WebState>,
) -> Result<WebInstance, WebError> {
    crate::bridge::spawn_pump(state.clone());
    let app: Router = router(state.clone());
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(WebError::Io)?;
    let bound = listener.local_addr().map_err(WebError::Io)?;
    let stop = CancellationToken::new();
    let stop_for_serve = stop.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { stop_for_serve.cancelled().await })
            .await
    });
    print_launch_line(bound, &state.token);
    Ok(WebInstance { addr: bound, token: state.token.clone(), stop, task })
}

/// 启动行（业主实测入口）：完整 URL 含 token；token 恒开是安全线不是可选项。
fn print_launch_line(addr: SocketAddr, token_str: &str) {
    let scope = if addr.ip().is_loopback() {
        String::new()
    } else {
        "（已绑定非回环地址：token 鉴权恒开，请勿外传完整地址）".to_string()
    };
    println!("☁ 云悠 web 已启动：http://{addr}/?token={token_str}{scope}（Ctrl-C 退出）");
}

/// 地址解析：None = 127.0.0.1:0（随机端口）；显式 --addr 需 host:port 字面形态。
fn resolve_addr(addr: Option<&str>) -> Result<SocketAddr, WebError> {
    let raw = addr.unwrap_or("127.0.0.1:0");
    raw.parse::<SocketAddr>()
        .map_err(|_| WebError::Addr(format!("需 host:port 形态（IPv6 写 [::1]:端口），得到：{raw}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 地址解析：缺省回环随机端口；坏形态中文报错点名原文。
    #[test]
    fn 地址解析缺省与坏形态() {
        assert_eq!(resolve_addr(None).unwrap().to_string(), "127.0.0.1:0");
        let err = resolve_addr(Some("localhost:8080")).unwrap_err();
        assert!(err.to_string().contains("localhost:8080"), "应点名原文：{err}");
        assert!(resolve_addr(Some("[::1]:0")).is_ok(), "IPv6 带方括号应可解析");
    }
}
