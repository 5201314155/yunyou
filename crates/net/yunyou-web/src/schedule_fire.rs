//! 定时到期触发器（W4）：Scheduler 到期 → 注入会话回合（agent-loop 像收到
//! 用户消息一样自然接活，设计 scheduler.v1 §一）。真投递缝=回合闸门内的
//! 正常回合（事件流自然回显）；schedule/fired 事件随 proto v1.1 投递缝开闸
//! 后再发（websocket.md §4，未开闸不发——诚实）。
//!
//! # 闸门语义
//! 到期注入走 TurnRunner.run（与用户提交同闸门）：占用=本条目触发失败留痕
//! 顺延（SDK 错误隔离同律），不炸调度循环、不排队伪装受理。

use yunyou_plugin_sdk::scheduler::{ScheduleEntry, SchedulerTrigger};
use std::sync::Arc;
use tokio::sync::broadcast;

/// 到期→回合注入触发器（instance.rs 装配；turns=独立执行器句柄）。
pub(crate) struct TurnFireTrigger {
    /// 回合执行器（与 WebState.turns 同款构造；注入与用户提交同闸门串行）。
    pub(crate) turns: Arc<crate::turns::TurnRunner>,
    /// 服务事件流真源（注入回合的事件并入服务广播，前端 follow 流自然可见）。
    pub(crate) broadcast: broadcast::Sender<crate::protocol::ServerMessage>,
}

impl SchedulerTrigger for TurnFireTrigger {
    fn name(&self) -> &str {
        "web-turn-fire"
    }

    fn on_fire(&self, entry: &ScheduleEntry, _fired_at_ms: u64) -> Result<(), String> {
        let turns = self.turns.clone();
        let message = entry.message.clone();
        let entry_id = entry.id.clone();
        // broadcast：本触发器运行在调度线程（spawn_thread 的 std thread）——
        // 回合是 async，桥到 tokio 运行时执行；无运行时=测试环境，留痕即返。
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let sender = self.broadcast.clone();
                handle.spawn(async move {
                    if let Err(err) = turns.run(&message, sender).await {
                        eprintln!("[定时 {entry_id}] 注入回合失败（顺延）: {err}");
                    }
                });
                Ok(())
            }
            Err(_) => {
                eprintln!("[定时 {entry_id}] 无 tokio 运行时，注入跳过（顺延）");
                Err("无运行时".into())
            }
        }
    }
}
