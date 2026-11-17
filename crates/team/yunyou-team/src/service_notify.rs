//! 服务层通知挂接（K-17 ②③）：submit→复核通知、approve→解锁通知。
//!
//! 从 service.rs 拆出（单一职责：业务逻辑与通知发送分文件，lead ① 裁）。
//! 通知正文合成复用 auto.rs 纯函数；hub 发送经 ctx 身份通道（团队 fiber="lead"）。

use crate::auto::{review_notify_body, unlock_notify_body, unblocked_notify_body};
use crate::service::TeamCore;
use yunyou::hub::{Address, OutgoingMessage};
use yunyou::WeakContext;
use std::sync::Arc;

/// submit 后通知复核席（②自动送审）：读标题→合成正文→hub 发送（from=团队 fiber）。
/// 通知失败不吞错也不阻塞 submit 本身（尽力而为通知面）。
pub(crate) fn notify_review(
    core: &Arc<TeamCore>,
    ctx: &WeakContext,
    task_id: u64,
) {
    let title = core
        .board
        .lock()
        .expect("任务板锁")
        .task_title(task_id)
        .unwrap_or_default();
    let body = review_notify_body(task_id, &title, "成员已提交，候审");
    if let Some(team_ctx) = ctx.upgrade() {
        let _ = team_ctx.hub_send(
            Address::Member("lead".into()),
            OutgoingMessage { kind: yunyou::hub::MessageKind::System, body },
        );
    }
    // K-18②：解锁事件广播——空闲成员 worker 经 board_wakeup 唤醒→首查可领
    // 任务（K-17 衔接）。notify_one（非 notify_waiters）：permit 持久存储——
    // worker 不在等待位（run_one 中/循环间隙）时通知不丢，下次 .notified()
    // 立即返回（notify_waiters 只唤醒当下等待者=竞态窗口丢通知，实测 B 延迟
    // 到 5s 兜底才领取即此根因）。多成员广播语义由 5s 兜底兜底+permit 逐个消费。
    core.board_wakeup.notify_one();
}

/// approve 后扫描阻塞链通知 owner（③自动解锁）：扫 blocked_by 含本任务的
/// Pending 任务→逐个通知其 owner「前置已通过」。rejected 不触发（v1 边界）。
pub(crate) fn notify_unlock(
    core: &Arc<TeamCore>,
    ctx: &WeakContext,
    task_id: u64,
    approve: bool,
) {
    if !approve {
        return; // rejected 不触发解锁通知（v1 边界，卡面注记）
    }
    // 一次持锁取齐（扫描结果+标题+各被阻塞任务的 owner），锁外发送——
    // T-1 修复：扫描结果此前赋 `_blocked` 后弃用，被阻塞任务 owner 永远收不到
    // 「前置已通过」（自动流转③半失效）。现逐个接回通知链路：owner 已认领才发
    // （未认领任务由 board_wakeup 广播兜底，无需点名）。
    let (blocked, title) = {
        let board = core.board.lock().expect("任务板锁");
        (board.tasks_blocked_by(task_id), board.task_title(task_id).unwrap_or_default())
    };
    let owners: Vec<(u64, String)> = blocked
        .into_iter()
        .filter_map(|blocked_id| {
            core.board
                .lock()
                .expect("任务板锁")
                .task_owner(blocked_id)
                .map(|owner| (blocked_id, owner))
        })
        .collect();
    let title_for_body = title.clone();
    if let Some(team_ctx) = ctx.upgrade() {
        let body = unlock_notify_body(task_id, &title_for_body);
        let _ = team_ctx.hub_send(
            Address::Member("lead".into()),
            OutgoingMessage { kind: yunyou::hub::MessageKind::System, body },
        );
        // ③自动解锁核心面：被阻塞任务逐个通知其 owner「前置已通过」（通知
        // 失败不阻塞 approve 本身，尽力而为通知面——与 notify_review 同纪律）
        for (blocked_id, owner) in &owners {
            let body = unblocked_notify_body(task_id, &title, *blocked_id);
            let _ = team_ctx.hub_send(
                Address::Member(owner.clone()),
                OutgoingMessage { kind: yunyou::hub::MessageKind::System, body },
            );
        }
    }
    // K-18②：解锁事件广播——空闲成员 worker 经 board_wakeup 唤醒→首查可领
    // 任务（K-17 衔接）。notify_one（非 notify_waiters）：permit 持久存储——
    // worker 不在等待位（run_one 中/循环间隙）时通知不丢，下次 .notified()
    // 立即返回（notify_waiters 只唤醒当下等待者=竞态窗口丢通知，实测 B 延迟
    // 到 5s 兜底才领取即此根因）。多成员广播语义由 5s 兜底兜底+permit 逐个消费。
    core.board_wakeup.notify_one();
}
