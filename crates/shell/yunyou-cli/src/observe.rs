//! 渲染观察者（task-10 B 线接线层）：实现 A 线 [`TurnObserver`]，
//! 把 agent-loop 事件映射为渲染视图并驱动终端。
//!
//! # 契约对齐（yunyou-agent-loop/src/observer.rs）
//! - `on_event` 被同步 await：本实现全部操作为轻量写终端 + spawn/abort，
//!   不 sleep 不阻塞事件流（spinner 循环归独立 tokio 任务，80ms 一帧）；
//! - 落盘类事件必然已持久化后才到达，渲染所见即所存；
//! - UserMessage 跳过（REPL stdin 回显天然承担）；AssistantMessage 跳过
//!   （内容已被 AssistantDelta 流覆盖；未满行由回合收尾（TurnComplete 或
//!   宿主 finish()）落定）。
//!
//! # spinner 会话
//! ToolCall 到达：非交互直接静默（Renderer 侧判据）；交互则 spawn 80ms
//! 定时任务（取消令牌即止）。ToolResult 到达：abort 定时任务 → 完成卡落定。
//! 一期 agent-loop 顺序执行，单槽足够；并行批次（树字段启用）扩多槽。
//!
//! # 任务 30 接线（③人话摘要 / ④树状分支）
//! - **单槽 last call**：ToolCall 的 `arguments_summary` 记入单槽，ToolResult
//!   到达时取走传给适配层生成人话摘要（bash= Ran…/读/改/搜，见 adapter）；
//! - **树归集器**：ToolCall 记 `call_id → (parent, depth)` 与 parent 在途子数；
//!   ToolResult 按 depth>0 拼 [`tree::indent_prefix`] 前缀渲染子调用完成卡，
//!   `is_last` = 完成时刻该 parent 已无在途兄弟（顺序/并行均语义成立：
//!   并行批里后完成者收尾 └─）。一期 A 线树字段恒 None（turn.rs 生成端未填），
//!   归集器就绪等值，平铺行为不变。回合收尾清空归集器（call_id 不跨轮残留）。

mod activity;
mod tree_tracker;

use activity::SpinnerSession;
use tree_tracker::TreeTracker;

use crate::adapter::tool_result_event;
use crate::render::event::TurnEvent as RenderEvent;
use crate::render::terminal;
use crate::render::tool;
use crate::render::tree;
use crate::render::Renderer;
use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use yunyou_agent_loop::{TurnObserver, TurnEvent as AgentEvent};
use yunyou_util::BoxFut;

/// 思考静默 debounce 时长（task-35a）：ReasoningDelta 后静默超过该值即恢复
/// 等待指示——思考活动行只在事件到达时刷新，静默期正是 R1b 实测 38.99s 空窗。
const QUIET_RESUME_MS: u64 = 2000;

/// 终端渲染观察者（Arc 共享进 agent-loop 的事件回调）。
pub struct TerminalObserver {
    inner: Arc<Inner>,
    /// 模型名（状态落款用；来自 llm 配置）。
    model: String,
}

struct Inner {
    renderer: Mutex<Renderer<Stdout>>,
    /// 进行中工具的 spinner 会话（一期单槽：顺序执行）。
    /// 【树字段启用前置条件清单（task-59 单槽清单）】并行批次落地时本槽扩多槽：
    /// spinner 槽→按 call_id 分槽；last_call 单槽→改 per-call 映射；waiting/quiet
    /// 双槽→树批次下仍单槽合法（回合级非 per-call）。前置=树字段启用（A 线
    /// turn.rs 生成端填 parent_tool_call_id，挂 task-26/K-7 团队插件接入）。
    spinner: Mutex<Option<SpinnerSession>>,
    /// 本轮起始时刻（状态落款耗时；CLI 侧计时）。
    started: Mutex<Instant>,
    /// 单槽 last call（任务 30-③）：(工具名, 参数摘要)；ToolResult 到达取走。
    last_call: Mutex<Option<(String, String)>>,
    /// 树归集器（任务 30-④）。
    tree: Mutex<TreeTracker>,
    /// 本回合取消令牌（task-38④：下游管道关闭时提前收场用）。REPL 装配层经
    /// [`TerminalObserver::set_cancel`] 注入；未注入时 BrokenPipe 只消音不取消。
    cancel: Mutex<Option<CancellationToken>>,
    /// 回合等待指示会话（task-35a：首字节前/段间空窗的活体信号；交互档专用，
    /// 1s ticker 逐秒渲染「… 等待回复 (Xs · Ctrl-C 中断)」）。
    waiting: Mutex<Option<SpinnerSession>>,
    /// 思考静默 debounce 会话（task-35a）：ReasoningDelta 后静默超
    /// [`QUIET_RESUME_MS`] 即恢复等待指示（单槽：新 delta 到达即重 arm）。
    quiet: Mutex<Option<SpinnerSession>>,
    /// waiting 行是否在屏（task-51① A 案）：ticker 首帧置位、他者接管清零——
    /// TurnComplete 收尾据此在落款前补 clear_active_line（防落款拼接等待行尾，
    /// 复验测试「快回合_落款不拼接等待行尾」字节级坐实；清行在 turn_done 之前，
    /// 保住落款行自身零重绘契约 render_width.rs）。
    waiting_painted: AtomicBool,
}

impl TerminalObserver {
    /// 构造：渲染器 + 模型名（模型名进状态落款）。
    pub fn new(renderer: Renderer<Stdout>, model: String) -> Self {
        let interactive = renderer.interactive();
        let inner = Arc::new(Inner {
            renderer: Mutex::new(renderer),
            spinner: Mutex::new(None),
            started: Mutex::new(Instant::now()),
            last_call: Mutex::new(None),
            tree: Mutex::new(TreeTracker::default()),
            cancel: Mutex::new(None),
            waiting: Mutex::new(None),
            quiet: Mutex::new(None),
            waiting_painted: AtomicBool::new(false),
        });
        // task-35a：回合级等待指示起表——交互档 1s ticker（首跳在 1s 后，快回合
        // 零噪音）；非交互/Plain 落一行静态提示（waiting_notice 交互档零输出）。
        if interactive {
            inner.start_waiting();
        } else {
            let _ = inner.renderer.lock().expect("渲染器锁").waiting_notice();
        }
        Self { inner, model }
    }
}

impl TerminalObserver {
    /// 注入本回合取消令牌（task-38④：下游管道关闭时提前收场）。装配层在构造
    /// 后立即调用；未注入时 BrokenPipe 仅消音不取消（功能降级不破行为）。
    pub fn set_cancel(&self, token: CancellationToken) {
        *self.inner.cancel.lock().expect("cancel 槽锁") = Some(token);
    }

    /// 渲染失败上报（task-38④）：管道关闭（`--task | head -c N` 等下游截断）
    /// → 提前收场（取消本回合，停止白烧 token）+ 一次性提示；其余渲染失败保持
    /// 旧行为（留一行提示，不打断对话）。cancel 未注入时只消音——逐块刷
    /// 「Broken pipe」警告比静默更糟（基线实证）。
    fn report_render_failure(&self, err: std::io::Error) {
        if err.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("（渲染警告：{err}）");
            return;
        }
        let token = self.inner.cancel.lock().expect("cancel 槽锁").clone();
        if let Some(token) = token {
            if !token.is_cancelled() {
                eprintln!("（输出管道已关闭，提前收场）");
                token.cancel();
            }
        }
    }

    /// 短锁取渲染器（锁内仅同步写终端，无 await）。
    fn lock(&self) -> std::sync::MutexGuard<'_, Renderer<Stdout>> {
        self.inner.renderer.lock().expect("渲染器锁")
    }

    /// 事件分发（同步轻量；渲染失败不影响对话主流程）。
    /// 各事件分支抽私有方法（行数清偿批 task-22：handle 原 103 行超函数 80
    /// 硬顶，拆分发+五分支小函数，零行为变更）。
    fn handle(&self, event: AgentEvent) {
        let result: io::Result<()> = match event {
            AgentEvent::UserMessage { .. } => Ok(()), // REPL stdin 回显已承担
            AgentEvent::AssistantDelta { text } => self.ev_assistant_delta(&text),
            AgentEvent::ReasoningDelta { text } => self.ev_reasoning_delta(&text),
            AgentEvent::ToolCall { call_id, name, arguments_summary, parent_tool_call_id, .. } => {
                self.ev_tool_call(&call_id, name, arguments_summary, parent_tool_call_id)
            }
            AgentEvent::ToolResult { call_id, name, summary, content, exit_code, is_error, .. } => {
                self.ev_tool_result(&call_id, &name, &summary, &content, exit_code, is_error)
            }
            AgentEvent::AssistantMessage { .. } => Ok(()), // Delta 流已覆盖
            // task-23 软着陆暂停：文案经本事件渲染（SystemMessage 仅落盘不进
            // 事件流——暂停指引「回复『继续』」必须上屏），dim 落定行直写
            //（profile 降级单源；结构化字段供程序化消费方，渲染取 message）。
            AgentEvent::TurnPaused { message, .. } => self.ev_turn_paused(&message),
            // task-27 压缩完成：透明性落定行（压缩改变后续记忆边界必须可见）。
            AgentEvent::Compacted { pre_tokens, .. } => {
                self.ev_compacted(pre_tokens)
            }
            AgentEvent::TurnComplete {
                prompt_tokens,
                completion_tokens,
                context_tokens,
                context_window,
                ..
            } => {
                self.ev_turn_complete(prompt_tokens, completion_tokens, context_tokens, context_window)
            }
        };
        if let Err(err) = result {
            // 渲染失败（如终端写断）不打断对话：管道关闭走 38④ 提前收场，其余留提示
            self.report_render_failure(err);
        }
    }

    /// 正文增量（task-35a：正文接管——撤等待与静默 debounce，正文流式自带活动行管理）。
    fn ev_assistant_delta(&self, text: &str) -> io::Result<()> {
        self.inner.stop_waiting();
        self.inner.stop_quiet();
        self.inner.waiting_painted.store(false, Ordering::Relaxed);
        let result = self.lock().text_delta(text);
        // task-48 接线：自愈降级一次性告知（观察者层发，stderr 侧通道）
        if let Some(notice) = self.lock().take_selfheal_notice() {
            eprintln!("[selfheal] {notice}");
        }
        result
    }

    /// 思考增量（task-35a：思考活动行接管前先撤等待指示；思考静默 2s 由 debounce 恢复）。
    fn ev_reasoning_delta(&self, text: &str) -> io::Result<()> {
        self.inner.stop_waiting();
        self.inner.stop_quiet();
        self.inner.waiting_painted.store(false, Ordering::Relaxed);
        let result = self.lock().reasoning_delta(text);
        self.inner.arm_quiet_resume();
        if let Some(notice) = self.lock().take_selfheal_notice() {
            eprintln!("[selfheal] {notice}");
        }
        result
    }

    /// 工具开始（task-35a：spinner 接管前先撤等待指示；任务 30-③④ 单槽+树归集登记）。
    fn ev_tool_call(
        &self,
        call_id: &str,
        name: String,
        arguments_summary: String,
        parent_tool_call_id: Option<String>,
    ) -> io::Result<()> {
        self.inner.stop_waiting();
        self.inner.stop_quiet();
        self.inner.waiting_painted.store(false, Ordering::Relaxed);
        // 任务 30-③：单槽记录参数摘要（人话摘要的命令/path/pattern 来源）
        *self.inner.last_call.lock().expect("last call 锁") =
            Some((name.clone(), arguments_summary.clone()));
        // 任务 30-④：树归集登记（一期 A 线 parent 恒 None，行为=平铺）
        self.inner.tree.lock().expect("树归集锁").on_call(call_id, parent_tool_call_id.as_deref());
        self.start_spinner(name, arguments_summary);
        Ok(())
    }

    /// 工具完成（停会话→取走单槽摘要→人话卡；depth>0 挂树前缀；落定后恢复等待指示）。
    fn ev_tool_result(
        &self,
        call_id: &str,
        name: &str,
        summary: &str,
        content: &serde_json::Value,
        exit_code: Option<i64>,
        is_error: bool,
    ) -> io::Result<()> {
        self.inner.stop_waiting();
        self.inner.stop_quiet();
        self.inner.waiting_painted.store(false, Ordering::Relaxed);
        self.stop_spinner();
        // 任务 30-③：取走单槽参数摘要 → 适配层人话摘要
        let args_summary = self
            .inner
            .last_call
            .lock()
            .expect("last call 锁")
            .take()
            .map(|(_, args)| args);
        let done = tool_result_event(name, summary, content, exit_code, is_error, args_summary.as_deref());
        // 任务 30-④：depth>0 的子调用挂树（indent_prefix 前缀 + 收尾符号）
        let (depth, is_last) = self.inner.tree.lock().expect("树归集锁").on_result(call_id);
        let rendered = if depth == 0 {
            self.lock().tool_done(&done)
        } else {
            let prefix = {
                let (_, profile) = self.lock().parts();
                tree::indent_prefix(profile, depth, is_last)
            };
            self.lock().tool_done_with_prefix(&done, &prefix)
        };
        // task-35a：工具段落落定→恢复等待指示（工具后到下一段思考/回复
        // 的空窗实测 27.5s——B5 计时第二坑）；渲染失败仍走统一上报
        rendered.map(|_| self.inner.start_waiting())
    }

    /// 回合完成收尾（task-51① A 案：先清在屏等待行再落款，保住落款零重绘契约）。
    fn ev_turn_complete(
        &self,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        context_tokens: Option<u64>,
        context_window: Option<u64>,
    ) -> io::Result<()> {
        // painted=true 蕴含交互态（ticker 仅交互档起），clear 恒安全。
        let waiting_on_screen = self.inner.waiting_painted.load(Ordering::Relaxed);
        self.inner.stop_waiting();
        self.inner.waiting_painted.store(false, Ordering::Relaxed);
        if waiting_on_screen {
            let mut guard = self.lock();
            let (out, _) = guard.parts();
            let _ = terminal::clear_active_line(out, true);
        }
        self.inner.stop_quiet();
        self.stop_spinner();
        let elapsed = self.inner.started.lock().expect("计时刻锁").elapsed();
        let done = RenderEvent::TurnDone {
            elapsed,
            prompt_tokens,
            completion_tokens,
            context_tokens,
            context_window,
            model: self.model.clone(),
        };
        let result = self.lock().turn_done(&done);
        // task-48 接线：回合界——清窗重探+连续健康恢复（renderer 自还原）
        self.lock().selfheal_new_turn();
        self.inner.tree.lock().expect("树归集锁").clear();
        result
    }

    /// 压缩完成单行（task-27，ev_* 家族同款 dim 落定行）。
    fn ev_compacted(&self, pre_tokens: u64) -> io::Result<()> {
        let mut guard = self.lock();
        let (out, profile) = guard.parts();
        writeln!(
            out,
            "{}〔已自动压缩：前文 {} tok 已摘要，后续对话以摘要为前文记忆〕{}",
            profile.dim(),
            pre_tokens,
            profile.reset()
        )?;
        out.flush()
    }

    /// 软着陆暂停单行（task-23，ba50f38 拆分后新分发位）：
    /// 「已到达本轮 X 上限，回复『继续』即可无缝延续」dim 落定行——
    /// profile 降级单源（Plain 档 dim=空串纯文本）。
    fn ev_turn_paused(&self, message: &str) -> io::Result<()> {
        let mut guard = self.lock();
        let (out, profile) = guard.parts();
        writeln!(out, "{}{message}{}", profile.dim(), profile.reset())?;
        out.flush()
    }
}

impl TurnObserver for TerminalObserver {
    fn on_event(&self, event: AgentEvent) -> BoxFut<'_, ()> {
        Box::pin(async move { self.handle(event) })
    }
}

impl TerminalObserver {
    /// 回合收尾（REPL 在 run_turn 返回后 Ok/Err 两路统一调用，审查 P1-2）：
    /// 停 spinner 会话 → 落定未满行（非交互先补输出）→ 清活动行 → 清树归集器。
    ///
    /// Err 路径 A 线不发 TurnComplete（yunyou-agent-loop turn.rs 事件时序契约），
    /// 取消/失败路径的渲染收尾由本方法兜底——否则 Ctrl-C 后流式半行滞留活动行
    /// 且 spinner 任务持续刷屏（ToolResult/TurnComplete 均不到达，stop 无人调）。
    /// Ok 路径调用为幂等（TurnComplete 已停 spinner 并落定落款行）。
    ///
    /// task-37① 三态之三：槽内尚有 spinner 会话=ToolResult 永不到达（回合被
    /// 取消/失败）——落一张中性取消卡（– 已取消），不留「无结果」空白；工具名
    /// 取 last_call（取消路径无人取走，单槽语义下即在途者）。
    pub fn finish(&self) {
        // task-35a：等待指示与静默 debounce 与 spinner 同点收编（云探交接预警
        // 采纳）——先停 ticker 再走既有收尾：waiting 行作为当前活动行，被下方
        // 取消卡的 clear_active_line 或 renderer.finish() 原位清掉，取消路径
        // 「waiting 行未清+取消卡落定」的叠加不存在。
        self.inner.stop_quiet();
        self.inner.stop_waiting();
        let in_flight = self.inner.spinner.lock().expect("spinner 槽锁").take();
        let last_call = self.inner.last_call.lock().expect("last call 锁").take();
        if let Some(session) = in_flight {
            session.stop.cancel();
            session.task.abort();
            if let Some((name, _args)) = last_call {
                let mut guard = self.lock();
                let (out, profile) = guard.parts();
                let _ = terminal::clear_active_line(out, true);
                let _ = tool::tool_cancelled(out, profile, &name);
            }
        }
        if let Err(err) = self.lock().finish() {
            self.report_render_failure(err);
        }
        self.inner.tree.lock().expect("树归集锁").clear();
    }
}

impl Drop for TerminalObserver {
    /// task-25②：观察者收尾 RAII 化——finish 幂等（Ok 路径显式调用在先，
    /// 此处为兜底第二击，重复调用零副作用）。正常路径 Arc 归零时多跑一次
    /// 幂等 finish；异常路径（run_dialogue_turn 5s 超时 drop turn→handler
    /// future 死→observer 引用归零）由本 Drop 补 finish——waiting ticker
    /// 与活动行不再刷屏残留至进程退出。
    fn drop(&mut self) {
        self.finish();
    }
}
