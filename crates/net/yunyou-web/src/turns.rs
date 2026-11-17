//! 回合执行器：web 侧的行 handler（对齐 repl.rs 的 loop_line_handler 职责，
//! 渲染端从终端换为广播通道；同一 agent-loop 服务、同一取消/收尾语义）。
//!
//! # 串行化裁决（REPL 行为等价）
//! REPL 天然一线程一回合；web 多连接并发提交会打破「会话唯一事实源」的单
//! 写者直觉——用回合闸门（try_lock）拒绝并发提交并人话报错，与 REPL 阻塞
//! 输入的行为等价。不排队：排队会让用户误以为已受理（连接层可立刻重发）。

use crate::observer::WebObserver;
use crate::protocol::ServerMessage;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;
use yunyou::{Context, ServiceId};
use yunyou_agent_loop::AgentLoop;

/// 回合执行器（进程级单例；多连接共享）。
pub struct TurnRunner {
    root_getter: Arc<dyn Fn() -> Context + Send + Sync>,
    model: String,
    /// 回合闸门：一回合未落定，后续提交拒绝（try_lock 即刻失败，不排队）。
    /// Arc 形态使守卫可 OwnedMutexGuard 化（'static）——v2 prompt 桥的
    /// 后台回合任务要持守卫入 spawn。
    gate: Arc<Mutex<()>>,
    /// 进行中回合的取消令牌（session/cancel 停止按钮；std 锁=临界区无 await）。
    active_cancel: std::sync::Mutex<Option<CancellationToken>>,
}

impl TurnRunner {
    /// 构造：root 获取回调（/new 换槽后取新装配）+ 模型名（落款用）。
    pub fn new(root_getter: Arc<dyn Fn() -> Context + Send + Sync>, model: String) -> Self {
        Self {
            root_getter,
            model,
            gate: Arc::new(Mutex::new(())),
            active_cancel: std::sync::Mutex::new(None),
        }
    }

    /// 当前会话装配 root（命令缝执行用；与回合同源——/new 换槽后取新装配）。
    pub(crate) fn root(&self) -> Context {
        (self.root_getter)()
    }

    /// 取回合闸门（命令执行同闸）：/new 重建装配不得与进行中回合并发——
    /// 旧 root dispose 会拆掉正在跑的回合的地基。命令与 REPL 等价排队
    /// （REPL 里回合中本就无法输命令），消息提交才是 try_lock 即刻拒绝。
    pub(crate) async fn lock_gate(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.gate.clone().lock_owned().await
    }

    /// 即刻探测回合闸门（v2 prompt 桥用）：占住=None（并发提交，RPC 层人话
    /// 拒绝）；拿到=Some(守卫)——守卫在手即持闸，交 [`Self::run_locked`]
    /// 执行回合，落定（含 Err）时释放。OwnedMutexGuard 无借用生命周期，
    /// 可随后台回合任务入 spawn。与 [`Self::run`] 同一闸门语义，区别仅在
    /// 受理时点：RPC 受理先应答，回合后台跑。
    pub(crate) fn try_lock_gate(&self) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.gate.clone().try_lock_owned().ok()
    }

    /// 跑一回合：七事件经广播通道发给全部连接。
    ///
    /// Err=壳层失败（并发拒绝 / agentLoop 缺失 / 回合失败），由连接层转
    /// Error 消息——agent-loop 契约：Err 路径不发 TurnComplete，页面靠
    /// Error 消息冻结未完块（渲染层收尾兜底，CLI 的 finish() 同职责）。
    pub async fn run(
        &self,
        text: &str,
        sender: broadcast::Sender<ServerMessage>,
    ) -> Result<(), String> {
        let Ok(_permit) = self.gate.clone().try_lock_owned() else {
            return Err("当前会话正在对话中，等本回合落定再发新消息".into());
        };
        self.run_locked(text, sender).await
    }

    /// 已持闸的回合执行体（[`Self::run`] 与 v2 prompt 桥共用）：调用方持有
    /// 闸门守卫，本方法不再 try_lock（持双守卫会死锁）。
    pub(crate) async fn run_locked(
        &self,
        text: &str,
        sender: broadcast::Sender<ServerMessage>,
    ) -> Result<(), String> {
        let root = (self.root_getter)();
        let Ok(agent_loop) = root.get::<dyn AgentLoop>(ServiceId::of::<dyn AgentLoop>()) else {
            return Err("agentLoop 服务未就绪（装配异常），本回合未执行".into());
        };
        let observer = Arc::new(WebObserver::new(sender, self.model.clone()));
        let cancel = CancellationToken::new();
        *self.active_cancel.lock().unwrap_or_else(|p| p.into_inner()) = Some(cancel.clone());
        let result = agent_loop
            .run_turn_observed(text, cancel, observer)
            .await
            .map(|_| ())
            .map_err(|err| format!("{err}"));
        *self.active_cancel.lock().unwrap_or_else(|p| p.into_inner()) = None;
        result
    }

    /// 停止按钮（session/cancel）：取消进行中回合——agent-loop 调用方令牌
    /// 贯穿，循环步首即断（Err loop/cancelled），闸门守卫随任务收尾释放。
    /// 无进行中回合=false（RPC 层转人话拒绝，不静默装成功）。
    pub(crate) fn cancel_active(&self) -> bool {
        let token = self
            .active_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 并发拒绝：闸门被占时第二回合即刻 Err（不排队不阻塞）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 并发提交闸门拒绝() {
        let runner = TurnRunner::new(Arc::new(Context::new), "test-model".into());
        let (sender, mut rx) = broadcast::channel(16);
        // 占住闸门：持锁期间第二路 run 应即刻被拒
        let _permit = runner.gate.lock().await;
        let err = runner.run("你好", sender.clone()).await.unwrap_err();
        assert!(err.contains("正在对话中"), "人话拒绝文案：{err}");
        drop(_permit);
        // 释放后可达 agentLoop 检查（测试 root 无装配 → 服务未就绪的失败路径）
        let err = runner.run("你好", sender).await.unwrap_err();
        assert!(err.contains("agentLoop"), "服务缺失人话文案：{err}");
        // 全程无事件泄漏（拒绝路径不广播）
        assert!(rx.try_recv().is_err(), "拒绝路径不应产生事件");
    }
}
