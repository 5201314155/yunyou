//! 定时到期触发器（W4）：Scheduler 到期 → 注入会话回合（agent-loop 像收到
//! 用户消息一样自然接活，设计 scheduler.v1 §一）。真投递缝=回合闸门内的
//! 正常回合（事件流自然回显）；schedule/fired 事件随 proto v1.1 投递缝开闸
//! 后再发（websocket.md §4，未开闸不发——诚实）。
//!
//! # 闸门语义
//! 到期注入走 TurnRunner.run（与用户提交同闸门）：占用=本条目触发失败留痕
//! 顺延（SDK 错误隔离同律），不炸调度循环、不排队伪装受理。

use crate::protocol::ServerMessage;
use yunyou_plugin_sdk::scheduler::{ScheduleEntry, SchedulerTrigger};
use std::sync::Arc;
use tokio::sync::broadcast;

/// 复制句柄包裹（Handle 不可 Clone 到结构体字段？——可 Clone；Arc 包一层即可）。
type TokioHandle = tokio::runtime::Handle;

/// 到期→回合注入触发器（instance.rs 装配；turns=独立执行器句柄）。
pub(crate) struct TurnFireTrigger {
    /// 回合执行器（与 WebState.turns 同款构造；注入与用户提交同闸门串行）。
    pub(crate) turns: Arc<crate::turns::TurnRunner>,
    /// 服务事件流真源（注入回合的事件并入服务广播，前端 follow 流自然可见）。
    pub(crate) broadcast: broadcast::Sender<crate::protocol::ServerMessage>,
    /// 装配期捕获的 tokio 句柄（D5，云尺 E2E 实测）：on_fire 运行在 SDK 调度
    /// std 线程——Handle::try_current() 必败（生产调度线程同样无运行时上下文，
    /// 注释原只虑测试环境=漏）。serve 装配时 Handle::current() 存入，on_fire
    /// 用该 handle.spawn（不再 try_current 猜环境）。
    pub(crate) handle: TokioHandle,
}

impl SchedulerTrigger for TurnFireTrigger {
    fn name(&self) -> &str {
        "web-turn-fire"
    }

    fn on_fire(&self, entry: &ScheduleEntry, _fired_at_ms: u64) -> Result<(), String> {
        let turns = self.turns.clone();
        let message = entry.message.clone();
        let entry_id = entry.id.clone();
        // D5 修复：用装配期捕获的 handle spawn（调度线程无运行时上下文——
        // try_current 在生产路径必败，原实现只虑测试环境=漏）
        let sender = self.broadcast.clone();
        self.handle.spawn(async move {
            if let Err(err) = turns.run(&message, sender.clone()).await {
                // 失败双通道：stderr 留痕（顺延语义）+人话入事件流（规范 §2.4
                // 「错误可行动」——只写日志用户不可见）
                eprintln!("[定时 {entry_id}] 注入回合失败（顺延）: {err}");
                let _ = sender.send(ServerMessage::Error {
                    text: format!("[定时 {entry_id}] 到期注入失败（已顺延）：{err}"),
                });
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ServerMessage;
    use crate::turns::TurnRunner;
    use std::sync::Arc;
    use yunyou::Context;

    /// D5 回归锚（云尺 E2E 实测生产调度线程注入必败）：句柄注入后 on_fire 在
    /// 「无运行时上下文的线程模拟」下仍能 spawn 回合（注入事件回流广播）。
    /// 模拟=从当前 runtime 里拿 handle 存入触发器，再在 runtime 外的裸线程调
    /// on_fire（=SDK 调度线程同构）。
    #[test]
    fn 调度线程语义_句柄注入后可spawn() {
        let handle = tokio::runtime::Runtime::new().expect("runtime");
        let (tx, mut rx) = tokio::sync::broadcast::channel(16);
        let trigger = {
            let handle = handle.handle().clone();
            TurnFireTrigger {
                turns: Arc::new(TurnRunner::new(Arc::new(Context::new), "m".into())),
                broadcast: tx.clone(),
                handle,
            }
        };
        let entry = ScheduleEntry {
            id: "sched-d5".into(),
            session_id: None,
            spec: yunyou_plugin_sdk::scheduler::ScheduleSpec::At { at_ms: 1 },
            message: "到期注入".into(),
            durable: false,
            created_at_ms: 1,
            paused: false,
        };
        // 裸线程（无运行时上下文=SDK 调度线程同构）：原 try_current 必败路径
        let result = std::thread::spawn(move || trigger.on_fire(&entry, 1)).join().expect("线程在");
        assert!(result.is_ok(), "句柄注入后调度线程可 spawn：{result:?}");
        // 注入回合失败（无装配 root）→ 人话 Error 事件回流广播
        let event = handle.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .expect("事件在")
                .expect("流在")
        });
        assert!(matches!(event, ServerMessage::Error { ref text } if text.contains("agentLoop")), "{event:?}");
    }
}
