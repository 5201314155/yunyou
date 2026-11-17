//! 活动行会话管理（自 observe.rs 按职责拆出，行数清偿批 task-22；纯搬移
//! 零行为变更）：spinner/等待指示/思考静默 debounce 三类暂态会话的起停。
//!
//! 单活动行纪律（原 Inner impl 块头注，搬移保真）：waiting 与思考活动行/
//! 工具 spinner/正文流式共用同一「当前活动行」，抢占优先级=正文流式 >
//! spinner > 思考活动行 > 等待指示；各事件分支首行先撤 waiting（ticker
//! 停止+槽清空），活动行的视觉替换由下一写者的 rewrite_active_line
//! （\r\e[K 前缀）天然完成，无双状态机各清各的。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::terminal::SPINNER_INTERVAL_MS;
use super::{Inner, TerminalObserver, QUIET_RESUME_MS};

/// 一个进行中工具的 spinner 会话（取消令牌 + 任务句柄）。
pub(super) struct SpinnerSession {
    pub(super) stop: CancellationToken,
    pub(super) task: JoinHandle<()>,
}

impl Inner {
    /// 开启等待指示（交互档且未在跑才起 1s ticker；耗时自回合起点续算不归零
    /// ——口径=用户等待总时长，与状态落款 started 同源）。非运行时上下文
    /// （单测）不起 ticker，零副作用。
    pub(super) fn start_waiting(self: &Arc<Self>) {
        if !self.renderer.lock().expect("渲染器锁").interactive() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let mut slot = self.waiting.lock().expect("等待槽锁");
        if slot.is_some() {
            return;
        }
        let inner = Arc::clone(self);
        let stop = CancellationToken::new();
        let stop_for_task = stop.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop_for_task.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        // 锁内只做同步写终端（微秒级），不违反「快速返回」契约
                        if let Ok(mut renderer) = inner.renderer.lock() {
                            let elapsed = inner.started.lock().expect("计时刻锁").elapsed();
                            let _ = renderer.waiting_frame(elapsed);
                            inner.waiting_painted.store(true, Ordering::Relaxed);
                            // task-48 接线：降级告知一次性取发（ticker 帧触发场景）
                            if let Some(notice) = renderer.take_selfheal_notice() {
                                eprintln!("[selfheal] {notice}");
                            }
                        }
                    }
                }
            }
        });
        *slot = Some(SpinnerSession { stop, task });
    }

    /// 停等待指示（幂等）。waiting 行作为「当前活动行」由下一写者原位替换
    /// （工具卡/思考行/正文流式各自带 rewrite 前缀；收尾路径 clear_active_line）。
    pub(super) fn stop_waiting(&self) {
        if let Some(session) = self.waiting.lock().expect("等待槽锁").take() {
            session.stop.cancel();
            session.task.abort();
        }
    }

    /// 停思考静默 debounce（幂等）。
    pub(super) fn stop_quiet(&self) {
        if let Some(session) = self.quiet.lock().expect("静默槽锁").take() {
            session.stop.cancel();
            session.task.abort();
        }
    }

    /// 思考静默 debounce（task-35a；守卫细化 task-51②，报告3 Bug-2）：
    /// ReasoningDelta 后静默超 [`QUIET_RESUME_MS`] 即恢复等待指示——恢复
    /// 与否按 renderer 流式缓冲现势判定（非回合级 seen 标记）：缓冲非空=
    /// 正文流式中（正文自己是活动行，不恢复）；缓冲空=段落间隙/mid-text
    /// 思考（照常恢复，38.99s 空窗的常见新形态）。
    pub(super) fn arm_quiet_resume(self: &Arc<Self>) {
        if !self.renderer.lock().expect("渲染器锁").interactive() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        self.stop_quiet();
        let inner = Arc::clone(self);
        let stop = CancellationToken::new();
        let stop_for_task = stop.clone();
        let task = tokio::spawn(async move {
            tokio::select! {
                _ = stop_for_task.cancelled() => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(QUIET_RESUME_MS)) => {
                    if !inner.renderer.lock().expect("渲染器锁").has_pending() {
                        inner.start_waiting();
                    }
                }
            }
        });
        *self.quiet.lock().expect("静默槽锁") = Some(SpinnerSession { stop, task });
    }
}

impl TerminalObserver {
    /// 开启 spinner 会话（仅交互终端有帧；Renderer 侧有静默判据）。
    pub(super) fn start_spinner(&self, name: String, summary: String) {
        if !self.lock().interactive() {
            return; // 非交互：无暂态，完成卡直接落定（快路径语义）
        }
        let mut slot = self.inner.spinner.lock().expect("spinner 槽锁");
        if slot.is_some() {
            return; // 已有进行中工具（防御：顺序执行下不应发生）
        }
        let inner = self.inner.clone(); // Arc<Inner> 整体克隆（Mutex 本身不可克隆）
        let stop = CancellationToken::new();
        let stop_for_task = stop.clone();
        let task = tokio::spawn(async move {
            let mut frame = 0usize;
            loop {
                tokio::select! {
                    _ = stop_for_task.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(SPINNER_INTERVAL_MS)) => {
                        frame += 1;
                        // 锁内只做同步写终端（微秒级），不违反「快速返回」契约
                        if let Ok(mut renderer) = inner.renderer.lock() {
                            // 耗时=帧数×帧间隔换算（秒级取整；任务起点=任务起点，
                            // 渲染层零时钟纪律——秒表只在驱动侧）
                            // task-65 P2 首秒 0s：不足 1 秒传 None（渲染层不显
                            // 耗时段）——「0s」假读数根除，诚实不估算。
                            let elapsed_secs = frame as u64 * SPINNER_INTERVAL_MS / 1000;
                            let elapsed = if elapsed_secs == 0 { None } else { Some(elapsed_secs) };
                            let _ = renderer.tool_call_frame(&name, &summary, frame, elapsed);
                        }
                    }
                }
            }
        });
        *slot = Some(SpinnerSession { stop, task });
    }

    /// 结束 spinner 会话（abort 定时任务，立即返回不拖慢事件流）。
    pub(super) fn stop_spinner(&self) {
        if let Some(session) = self.inner.spinner.lock().expect("spinner 槽锁").take() {
            session.stop.cancel();
            session.task.abort();
        }
    }
}
