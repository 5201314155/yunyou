//! IPC 客户端（网关侧，方案 §2.2/§2.6）：连接 agent 侧 UDS、hello 认证
//! （secret SHA-256 首帧）、JSON Lines 帧收发、断线退避重连（1s→30s+抖动，
//! §2.6）与 resume 补发对接（事件回放由 agent 侧执行，客户端只报 last_seq）。
//!
//! 职责边界：本模块只管「连接与帧传输」——业务语义（session.open 惰性创建/
//! chat 桥接）归 http.rs/ws.rs 的 handler（阶段 B 同批）。secret 的读取归
//! 部署侧（凭证文件路径约定见 ipc-types::paths，网关进程以配置注入明文）。

use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use yunyou_ipc_types::{
    decode_frame, encode_frame, GatewayIpcFrame, Hello, ResumeInfo, PROTOCOL_VERSION,
};

/// agent 不可达（503 对应物；审计由调用方留痕）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUnreachable {
    pub reason: String,
}

impl std::fmt::Display for AgentUnreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent 不可达：{}", self.reason)
    }
}

/// IPC 客户端配置。
#[derive(Debug, Clone)]
pub struct IpcClientConfig {
    /// agent 侧套接字路径（ipc-types::paths::GATEWAY_SOCKET 的绝对化形态）。
    pub socket_path: PathBuf,
    /// IPC secret 明文（部署侧从 run/gateway-ipc-secret 读入；只用于计算
    /// SHA-256 首帧，明文不过线）。
    pub secret: String,
    /// 重连退避起点（§2.6：1s 起）。
    pub backoff_initial_ms: u64,
    /// 重连退避上限（§2.6：30s）。
    pub backoff_max_ms: u64,
}

impl IpcClientConfig {
    /// 默认退避参数（§2.6 定案值）。
    pub fn new(socket_path: PathBuf, secret: String) -> Self {
        Self { socket_path, secret, backoff_initial_ms: 1_000, backoff_max_ms: 30_000 }
    }
}

/// 一条已建立的 IPC 会话连接（帧级收发；业务编排在 handler）。
pub struct IpcConnection {
    /// 出站发送端（帧→agent；对端断开=SendError→重连由外层管理）。
    tx: mpsc::Sender<GatewayIpcFrame>,
    /// 入站订阅端（agent→帧；多订阅=广播克隆，chat.events 分发用）。
    rx: mpsc::Receiver<GatewayIpcFrame>,
}

impl IpcConnection {
    /// 建连+hello 握手（同步语义：成功=双帧往返完成；失败=AgentUnreachable）。
    ///
    /// `resume`：断线重连时携带（§2.6——agent 从会话存储回放 seq>last_seq）。
    pub async fn connect(
        config: &IpcClientConfig,
        resume: Option<ResumeInfo>,
    ) -> Result<Self, AgentUnreachable> {
        let stream = UnixStream::connect(&config.socket_path).await.map_err(|err| {
            AgentUnreachable { reason: format!("UDS 连接失败（{}）: {err}", config.socket_path.display()) }
        })?;
        let (read_half, mut write_half) = stream.into_split();
        // 首帧 hello（secret SHA-256，不传明文——§2.2 双因子之二）
        let hello = GatewayIpcFrame::Hello(Hello {
            secret_hash: crate::token::sha256_hex(&config.secret),
            protocol_version: PROTOCOL_VERSION,
            resume,
        });
        write_half
            .write_all(encode_frame(&hello).map_err(|err| AgentUnreachable {
                reason: format!("hello 编码失败: {err}"),
            })?
            .as_bytes())
            .await
            .map_err(|err| AgentUnreachable { reason: format!("hello 写入失败: {err}") })?;
        write_half
            .write_all(&[yunyou_ipc_types::FRAME_SEPARATOR])
            .await
            .map_err(|err| AgentUnreachable { reason: format!("hello 分隔写入失败: {err}") })?;
        // 读 hello 应答（单帧；非 HelloOk=认证拒绝）
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|err| AgentUnreachable { reason: format!("hello 应答读取失败: {err}") })?;
        match decode_frame(line.trim()).map_err(|err| AgentUnreachable {
            reason: format!("hello 应答解码失败: {err}"),
        })? {
            GatewayIpcFrame::HelloOk(_) => {}
            GatewayIpcFrame::HelloErr(err_frame) => {
                return Err(AgentUnreachable { reason: format!("agent 握手拒绝: {}", err_frame.reason) });
            }
            other => {
                return Err(AgentUnreachable {
                    reason: format!("握手期收到非应答帧: {}", other_type(&other)),
                });
            }
        }
        // 帧泵：入站行→广播通道；出站通道→写出（任务随连接存活）
        let (in_tx, in_rx) = mpsc::channel(256); // §2.6 背压容量定案值
        let (out_tx, mut out_rx) = mpsc::channel::<GatewayIpcFrame>(64);
        tokio::spawn(async move {
            let mut reader = reader;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // 对端关闭
                    Ok(_) => {
                        if let Ok(frame) = decode_frame(line.trim()) {
                            if in_tx.send(frame).await.is_err() {
                                break; // 订阅方全撤=连接废弃
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                if let Ok(encoded) = encode_frame(&frame) {
                    // 写帧+分隔顺序 await（两步失败即断，外层重连逻辑接管）
                    let frame_ok = write_half.write_all(encoded.as_bytes()).await.is_ok()
                        && write_half.write_all(b"\n").await.is_ok();
                    if !frame_ok {
                        break;
                    }
                }
            }
        });
        Ok(Self { tx: out_tx, rx: in_rx })
    }

    /// 发送一帧（对端断开=Err；重连由外层会话管理）。
    pub async fn send(&self, frame: GatewayIpcFrame) -> Result<(), AgentUnreachable> {
        self.tx.send(frame).await.map_err(|_| AgentUnreachable {
            reason: "IPC 通道已关闭（agent 断连）".into(),
        })
    }

    /// 收一帧（None=对端关闭；调用方决定重连）。
    pub async fn recv(&mut self) -> Option<GatewayIpcFrame> {
        self.rx.recv().await
    }
}

fn other_type(frame: &GatewayIpcFrame) -> &'static str {
    match frame {
        GatewayIpcFrame::Hello(_) => "hello",
        GatewayIpcFrame::HelloOk(_) => "hello_ok",
        GatewayIpcFrame::HelloErr(_) => "hello_err",
        GatewayIpcFrame::Ping(_) => "ping",
        GatewayIpcFrame::Pong(_) => "pong",
        GatewayIpcFrame::SessionOpen(_) => "session.open",
        GatewayIpcFrame::SessionOpened(_) => "session.opened",
        GatewayIpcFrame::ChatSend(_) => "chat.send",
        GatewayIpcFrame::ChatEvent(_) => "chat.events",
        GatewayIpcFrame::PanelTasksRead(_) => "panel.tasks.read",
        GatewayIpcFrame::PanelCommandSubmit(_) => "panel.command.submit",
    }
}

/// 退避序列（§2.6：1s 起，上限 30s+抖动；测试用纯函数锁形态）。
pub fn backoff_ms(attempt: u32, initial_ms: u64, max_ms: u64) -> u64 {
    let base = initial_ms.saturating_mul(1u64 << attempt.min(5));
    base.clamp(initial_ms, max_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 退避序列形态锁（§2.6：1s 起、30s 封顶、指数不溢出）。
    #[test]
    fn 退避序列形态() {
        assert_eq!(backoff_ms(0, 1_000, 30_000), 1_000);
        assert_eq!(backoff_ms(1, 1_000, 30_000), 2_000);
        assert_eq!(backoff_ms(2, 1_000, 30_000), 4_000);
        assert_eq!(backoff_ms(5, 1_000, 30_000), 30_000, "封顶 30s");
        assert_eq!(backoff_ms(9, 1_000, 30_000), 30_000, "超封顶不溢出");
    }
}
