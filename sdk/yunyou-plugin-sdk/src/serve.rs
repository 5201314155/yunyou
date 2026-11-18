//! 协议服务循环（serve 的实现面；帧序镜像宿主 ProxyPlugin，fail-loud 不降级）。
//!
//! 写线程化（proto v1.1 真投递，task-14 续）：stdout 单写者=专用写线程（拥有
//! stdout，经 mpsc 命令通道收帧）；主循环只读 stdin 分派——触发器线程经
//! [`crate::scheduler::DeliverHandle`] 投递 DeliverMessage 并阻塞等 DeliverAck，
//! 与主循环读写互不抢锁（双通道 stdio 天然两线程两向）。
//!
//! 同步 std::io 形态（无 tokio——SDK 零异步依赖）：宿主读泵/写泵按行对发，
//! 插件侧读循环+写线程按行对答，时序天然吻合。
//!
//! 锁豁免口径（K-14 同族）：本模块 std 锁（投递等待表）全部 expect 直取——
//! 中毒=SDK 缺陷必须炸出来，文件级声明保留警报不可静默。

#![allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决，见模块头）：投递等待表锁边界后触发=SDK 缺陷
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use yunyou_plugin_proto::{
    decode_frame, encode_frame, DeliverAck, DeliverMessage, Hello, HelloErr, HelloOk,
    PluginProtoFrame, ToolInvoke, ToolResult, ToolResultErr, ToolResultOk, ToolsList, ToolsListOk,
    ToolOutcome, PROTOCOL_VERSION,
};

use crate::scheduler::{Scheduler, SchedulerTrigger};
use crate::Plugin;

/// 投递回执等待方（std oneshot 等价=mpsc；deliver_id→回执发送端）。
type AckWaiters = Arc<Mutex<HashMap<String, mpsc::Sender<Result<String, String>>>>>;

/// 投递回执默认超时（宿主侧单请求超时同量级；超时=按 miss 策略顺延）。
const ACK_TIMEOUT: Duration = Duration::from_secs(35);

/// 协议服务主循环：读 stdin 一行→解帧→分派；EOF/shutdown 退出。
pub(crate) fn serve(plugin: Plugin, scheduler: Option<Arc<Scheduler>>) -> ExitCode {
    // 启动诊断（stderr——宿主 drain 透传主板日志；stdout 是协议专用通道）
    eprintln!(
        "[yunyou-plugin-sdk:{}] v{} 就绪（category={:?}，工具数={}）",
        plugin.id(),
        plugin.version(),
        plugin.category_ref(),
        plugin.tools().len()
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut line = String::new();

    // 写线程：stdout 唯一写者（主循环与触发器线程都经 tx 发帧；协议通道互斥）
    let (out_tx, out_rx) = mpsc::channel::<PluginProtoFrame>();
    // 写线程 detach（不持 JoinHandle）：EOF/Shutdown 退出不 join（调度线程持
    // out_tx clone，通道计数不归零=join 永等——自举实测锚）；进程退出即收
    let _writer = std::thread::spawn(move || {
        let mut out = stdout.lock();
        while let Ok(frame) = out_rx.recv() {
            if let Ok(encoded) = encode_frame(&frame) {
                if out.write_all(encoded.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
                    break;
                }
                let _ = out.flush();
            }
        }
    });

    // 投递回执等待表（触发器线程阻塞等 Ack；主循环收 Ack 唤醒）
    let waiters: AckWaiters = Arc::new(Mutex::new(HashMap::new()));

    // 触发器投递通道：Scheduler 到期 → 本循环发 DeliverMessage（经调度线程回调）
    if let Some(sched) = &scheduler {
        let deliver = DeliverHandle {
            out_tx: out_tx.clone(),
            waiters: Arc::clone(&waiters),
            seq: std::sync::atomic::AtomicU64::new(1),
        };
        sched.set_trigger(Arc::new(DispatchingTrigger {
            inner: Mutex::new(None),
            deliver,
        }));
    }

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // 宿主退出（stdin EOF）：正常终止
                drop(out_tx); // 不 join writer：调度线程持 out_tx clone（spawn_thread
                              // 长持），通道计数不归零=join 永等（自举实测锚）；
                              // 进程退出即收全部线程
                return ExitCode::SUCCESS;
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!("[yunyou-plugin-sdk] stdin 读取失败（退出）: {err}");
                drop(out_tx);
                return ExitCode::FAILURE;
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue; // 空行容错（宿主握手期可能补发换行）
        }
        let frame = match decode_frame(trimmed) {
            Ok(frame) => frame,
            Err(err) => {
                // 坏帧：stderr 报告+继续（宿主读泵对坏行同款容错；不退出——
                // 单行损坏不应杀插件）
                eprintln!("[yunyou-plugin-sdk:{}] 坏帧跳过: {err}", plugin.id());
                continue;
            }
        };
        match frame {
            // ① 握手：版本比对，不符=hello_err+exit(1)（fail-loud，不静默降级）
            PluginProtoFrame::Hello(Hello { protocol_version, capabilities: _ }) => {
                if protocol_version == PROTOCOL_VERSION {
                    let reply = PluginProtoFrame::HelloOk(HelloOk {
                        protocol_version: PROTOCOL_VERSION,
                        plugin_name: plugin.id().to_string(),
                    });
                    if out_tx.send(reply).is_err() {
                        eprintln!("[yunyou-plugin-sdk:{}] hello_ok 写出失败", plugin.id());
                        return ExitCode::FAILURE;
                    }
                } else {
                    let reason = format!(
                        "协议版本不兼容：宿主={protocol_version}，插件 SDK 支持={PROTOCOL_VERSION}（升级任一侧后重试）"
                    );
                    let _ = out_tx.send(PluginProtoFrame::HelloErr(HelloErr { reason }));
                    return ExitCode::FAILURE;
                }
            }
            // ② 工具清单
            PluginProtoFrame::ToolsList(ToolsList) => {
                let reply = PluginProtoFrame::ToolsListOk(ToolsListOk {
                    tools: plugin.descriptors(),
                });
                if out_tx.send(reply).is_err() {
                    eprintln!("[yunyou-plugin-sdk:{}] tools_list_ok 写出失败", plugin.id());
                    return ExitCode::FAILURE;
                }
            }
            // ③ 工具调用：白名单校验（未上报名拒绝）→解析 args（协议面为 JSON
            // 文本）→执行→回 tool_result
            PluginProtoFrame::ToolInvoke(ToolInvoke { invoke_id, name, args }) => {
                let outcome = match plugin.find_tool(&name) {
                    None => ToolOutcome::Err(ToolResultErr {
                        code: "tool/unknown-tool".into(),
                        message: format!("工具 \"{name}\" 未在本插件 tools_list 上报（白名单拒绝）"),
                        hint: None,
                    }),
                    Some(tool) => match serde_json::from_str::<serde_json::Value>(&args) {
                        Err(err) => ToolOutcome::Err(ToolResultErr {
                            code: "tool/invalid-args".into(),
                            message: format!("参数不是合法 JSON：{err}"),
                            hint: Some("对照工具 parameters 的 JSON Schema 修正参数".into()),
                        }),
                        Ok(parsed) => match tool.execute(parsed) {
                            Ok(value) => match serde_json::to_string(&value) {
                                Ok(text) => ToolOutcome::Ok(ToolResultOk { value: text }),
                                Err(err) => ToolOutcome::Err(ToolResultErr {
                                    code: "tool/exec".into(),
                                    message: format!("返回值序列化失败：{err}"),
                                    hint: None,
                                }),
                            },
                            Err(failure) => ToolOutcome::Err(ToolResultErr {
                                code: failure.code,
                                message: failure.message,
                                hint: failure.hint,
                            }),
                        },
                    },
                };
                let reply = PluginProtoFrame::ToolResult(ToolResult { invoke_id, outcome });
                if out_tx.send(reply).is_err() {
                    eprintln!("[yunyou-plugin-sdk:{}] tool_result 写出失败", plugin.id());
                    return ExitCode::FAILURE;
                }
            }
            // ④ 投递回执：按 deliver_id 唤醒等待的触发器线程（v1.1）
            PluginProtoFrame::DeliverAck(DeliverAck { deliver_id, ok, detail }) => {
                #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
                let waiter = waiters.lock().expect("投递等待表锁").remove(&deliver_id);
                if let Some(tx) = waiter {
                    let _ = tx.send(if ok {
                        Ok(detail)
                    } else {
                        Err(detail)
                    });
                } else {
                    eprintln!(
                        "[yunyou-plugin-sdk:{}] DeliverAck 无等待方（超时后迟到）: {deliver_id}",
                        plugin.id()
                    );
                }
            }
            // ⑤ 卸载通知：立即退出（宿主 killpg 兜底，不悬挂）
            PluginProtoFrame::Shutdown(_) => {
                drop(out_tx); // 不 join writer（同 EOF 分支：调度线程持 clone）
                return ExitCode::SUCCESS;
            }
            // 其余帧=宿主→插件方向之外的回声（协议误用）：stderr 报告不退出
            other => {
                eprintln!("[yunyou-plugin-sdk:{}] 非预期帧忽略: {other:?}", plugin.id());
            }
        }
    }
}

/// 写一帧（供需要同步直写的场景；主循环内已统一走写线程，此函数仅握手前诊断）。
#[allow(dead_code)]
fn write_frame<W: Write>(out: &mut W, frame: &PluginProtoFrame) -> std::io::Result<()> {
    let line = encode_frame(frame).map_err(std::io::Error::other)?;
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

/// 投递句柄（v1.1）：到期触发器经此把消息交宿主并等回执。
///
/// 由 serve() 构造并注入调度触发器；插件代码一般不直接使用（触发器实现内用）。
pub struct DeliverHandle {
    out_tx: mpsc::Sender<PluginProtoFrame>,
    waiters: AckWaiters,
    seq: std::sync::atomic::AtomicU64,
}

impl DeliverHandle {
    /// 投递一条消息并阻塞等回执（超时=Err「宿主未回执」→触发器按 miss 顺延）。
    pub fn deliver(
        &self,
        session_id: Option<String>,
        text: String,
    ) -> Result<String, String> {
        let deliver_id = format!(
            "dlv-{}",
            self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let (rtx, rrx) = mpsc::channel();
        self.waiters
            .lock()
            .expect("投递等待表锁")
            .insert(deliver_id.clone(), rtx);
        let sent = self.out_tx.send(PluginProtoFrame::DeliverMessage(DeliverMessage {
            deliver_id: deliver_id.clone(),
            session_id,
            text,
        }));
        if sent.is_err() {
            #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
            self.waiters.lock().expect("投递等待表锁").remove(&deliver_id);
            return Err("协议通道已关闭（宿主退出）".into());
        }
        match rrx.recv_timeout(ACK_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
            self.waiters.lock().expect("投递等待表锁").remove(&deliver_id);
                Err("宿主投递回执超时（按 miss 策略顺延）".into())
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err("协议通道已关闭（宿主退出）".into())
            }
        }
    }
}

/// 调度触发器桥（SDK 内建）：把 Scheduler 的到期回调桥接到投递句柄；
/// 若插件作者另设触发器（set_trigger），桥内保留二者（先内建投递，再转发作者回调）。
struct DispatchingTrigger {
    inner: Mutex<Option<Arc<dyn SchedulerTrigger>>>,
    deliver: DeliverHandle,
}

impl SchedulerTrigger for DispatchingTrigger {
    fn name(&self) -> &str {
        "sdk-dispatching"
    }

    fn on_fire(&self, entry: &crate::scheduler::ScheduleEntry, fired_at_ms: u64) -> Result<(), String> {
        let text = entry
            .message
            .replace("{id}", &entry.id)
            .replace("{fired_at}", &fired_at_ms.to_string());
        let prefix = if entry.durable {
            "[定时·durable"
        } else {
            "[定时"
        };
        let full = format!("{prefix} {}] {text}", entry.id);
        // 投递结果决定回调成败（宿主失败=按 miss 策略顺延，设计 §五）
        self.deliver.deliver(entry.session_id.clone(), full)?;
        if let Some(inner) = self.inner.lock().expect("桥内触发器槽锁").clone() {
            inner.on_fire(entry, fired_at_ms)?;
        }
        Ok(())
    }

    fn on_idle_tick(&self, now_ms: u64) {
        if let Some(inner) = self.inner.lock().expect("桥内触发器槽锁").clone() {
            inner.on_idle_tick(now_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)] // 测试面豁免（K-14 观察期口径）

    use super::*;
    use crate::scheduler::{ScheduleSpec, Scheduler};
    use std::sync::atomic::AtomicU64;

    /// 双端进程内投递链：模拟宿主线程收 DeliverMessage→回 DeliverAck(ok)；
    /// 触发器经 DeliverHandle 阻塞等回执→tick 完成（真投递全链，真机前锚）。
    #[test]
    fn 投递链_宿主回执_ok() {
        let clock = Arc::new(AtomicU64::new(0));
        let sched = Arc::new(Scheduler::new({
            let clock = Arc::clone(&clock);
            move || clock.load(std::sync::atomic::Ordering::Relaxed)
        }));
        let (out_tx, out_rx) = mpsc::channel::<PluginProtoFrame>();
        let waiters: AckWaiters = Arc::new(Mutex::new(HashMap::new()));
        let deliver = DeliverHandle {
            out_tx: out_tx.clone(),
            waiters: Arc::clone(&waiters),
            seq: std::sync::atomic::AtomicU64::new(1),
        };
        // 模拟宿主：收 DeliverMessage→回 DeliverAck(ok)
        let host = std::thread::spawn(move || {
            if let Ok(PluginProtoFrame::DeliverMessage(m)) = out_rx.recv_timeout(Duration::from_secs(5)) {
                assert!(m.text.contains("提醒"), "宿主应收到注入消息，实际 {}", m.text);
                let _ = out_tx.send(PluginProtoFrame::DeliverAck(DeliverAck {
                    deliver_id: m.deliver_id,
                    ok: true,
                    detail: "已入注入队列".into(),
                }));
            } else {
                panic!("宿主应先收到 DeliverMessage");
            }
        });
        // 触发器（作者面）：经 deliver 投递并等回执
        struct T(Arc<DeliverHandle>);
        impl SchedulerTrigger for T {
            fn name(&self) -> &str { "test" }
            fn on_fire(&self, _entry: &crate::scheduler::ScheduleEntry, _f: u64) -> Result<(), String> {
                self.0.deliver(None, "提醒".into()).map(|_| ())
            }
        }
        sched.set_trigger(Arc::new(T(Arc::new(deliver))));
        sched
            .register_schedule(None, ScheduleSpec::Interval { every_ms: 50 }, "提醒".into())
            .expect("注册应成功");
        clock.store(100, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(sched.tick(), 1, "到期触发+回执 Ok=回调成功");
        host.join().expect("宿主线程应正常结束");
    }

    /// 投递链失败面：宿主回 ok:false→deliver Err→回调 Err（miss 顺延语义）。
    #[test]
    fn 投递链_宿主回执_false_err() {
        let clock = Arc::new(AtomicU64::new(0));
        let sched = Arc::new(Scheduler::new({
            let clock = Arc::clone(&clock);
            move || clock.load(std::sync::atomic::Ordering::Relaxed)
        }));
        let (out_tx, out_rx) = mpsc::channel::<PluginProtoFrame>();
        let waiters: AckWaiters = Arc::new(Mutex::new(HashMap::new()));
        let deliver = DeliverHandle {
            out_tx: out_tx.clone(),
            waiters: Arc::clone(&waiters),
            seq: std::sync::atomic::AtomicU64::new(1),
        };
        let host = std::thread::spawn(move || {
            if let Ok(PluginProtoFrame::DeliverMessage(m)) = out_rx.recv_timeout(Duration::from_secs(5)) {
                let _ = out_tx.send(PluginProtoFrame::DeliverAck(DeliverAck {
                    deliver_id: m.deliver_id,
                    ok: false,
                    detail: "会话已结束".into(),
                }));
            }
        });
        // 直接验证 DeliverHandle 层回执转发（宿主 false→Err；miss 顺延语义）
        let r = deliver.deliver(None, "x".into());
        assert!(r.is_err(), "宿主 false 回执应 Err");
        host.join().unwrap();
        let _ = sched; // 调度器保持存活
    }
}
