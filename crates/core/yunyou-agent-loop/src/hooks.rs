//! 组装前钩子缝（task-11，设计草图 lead 批准 2026-11-18）。
//!
//! # 定位与触发时机
//! 缝=请求出口：`compose_request` 产物之后、发送之前，按序调用已注册钩子。
//! 钩子**只读会话事实源**（SessionEvent 落盘不动），只裁剪本次请求的消息清单——
//! 裁剪是投影级，不是改写历史（与 L-10 纪律同源：落盘记录原样，回发面可归一）。
//! 触发点在重试包装之外：重试重发同一 request 不重组装，钩子不重复触发。
//!
//! # 声明式纪律（lead 批复加条，保缓存与安全边界）
//! [`ComposeEdit`] 只允许「删整条 / 置换 tool_result 为占位」两种声明式指令，
//! **禁任意重写请求体**——钩子拿不到 `&mut Message`，不可能注入伪造内容或改写
//! 用户输入；引擎按声明应用并做配对完整性守卫（拆散 tool_calls/tool_result
//! 配对的指令丢弃，L-3/L-10 同族风险面不重开）。
//!
//! # 错误隔离（对齐插件崩溃隔离语义）
//! 钩子 Err / panic / 超时（默认 200ms）→ 跳过该钩子本轮，请求照发（裸清单）；
//! 同一钩子连续失败 [`HOOK_FAILURE_TRIP`] 次 → 自动摘除（进程内软态，重装即恢复），
//! 对齐 compaction 连败熔断先例。钩子是增强缝不是承重墙，任何故障不阻塞轮次。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use yunyou::Result;
use yunyou_llm::Message;
use yunyou_util::BoxFut;

/// 单钩子执行超时（v1 常量；配置旋钮候 [agent_loop] hook_timeout_ms 后续卡）。
pub(crate) const HOOK_TIMEOUT: Duration = Duration::from_millis(200);
/// 同一钩子连续失败摘除阈值（对齐 compaction_failure_limit=3 先例）。
pub(crate) const HOOK_FAILURE_TRIP: u32 = 3;

/// 组装前钩子输入（只读投影；消息以清单下标线性定位，system 首位约定不变）。
pub struct PreComposeRequest<'a> {
    /// 本次将发送的消息清单（compose_request 产物，未发送）。
    pub messages: &'a [Message],
    /// 本回合累计已用步数（策略常按步数预算裁剪）。
    pub steps_used: usize,
    /// 上下文水位快照（最后一步 usage.prompt；None=端点未报，策略自行降级）。
    pub context_tokens: Option<u64>,
    /// 模型上下文窗口（None=未配置）。
    pub context_window: Option<u64>,
}

/// 裁剪指令（声明式两型，v1 只做删与置换，不做增；理由见模块头声明式纪律）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeEdit {
    /// 整条删除：按消息下标（compose 产物清单的下标）。
    Remove {
        /// 目标消息下标。
        message_index: usize,
    },
    /// tool_result 置换为占位 stub（保 tool 消息壳=配对完整，剥内容省 token）。
    StubToolResult {
        /// 目标消息下标（必须是 role=Tool 的消息，否则指令被守卫丢弃）。
        message_index: usize,
        /// 占位正文（人话，如「[工具输出已被策略折叠：原 N 字]」）。
        stub_text: String,
    },
}

/// 组装前钩子（task-11 缝，v1）。
///
/// 注册面=内核 Context 服务表：钩子提供方插件 inject
/// [`HooksRegistryService`](crate::hooks::HooksRegistryService) 并在 setup 期
/// [`register`](HooksRegistry::register)；运行期注册先后无关（runner 每轮快照）。
/// 卸载纪律：注册表由 agent-loop 插件持有，钩子提供方 dispose 时应自行 unregister
/// （v1 未强制——摘除兜底=失败熔断；强化候 SDK 二期 hook 生命周期）。
pub trait PreComposeHook: Send + Sync {
    /// 钩子显示名（诊断/摘除点名用；同名注册 fail-loud）。
    fn name(&self) -> &str;

    /// 执行序（同序按注册先后稳定排序；小序先跑——「先删大块再摘要」类策略
    /// 依赖顺序确定性）。
    fn order(&self) -> u32 {
        100
    }

    /// 组装前裁剪：返回声明式指令清单（引擎单点合并应用，防 N 个钩子互相覆写）。
    fn on_pre_compose(
        &self,
        request: &PreComposeRequest<'_>,
    ) -> BoxFut<'_, Result<Vec<ComposeEdit>>>;
}

/// 钩子注册表服务契约（钩子提供方插件 inject 面）。
pub trait HooksRegistryService: Send + Sync {
    /// 注册钩子；同名 fail-loud（重复注册=装配期 bug，不静默覆盖）。
    fn register(&self, hook: Arc<dyn PreComposeHook>) -> Result<()>;
    /// 注销钩子（按名；返回是否确有注销）。
    fn unregister(&self, name: &str) -> bool;
    /// 在册钩子名清单（观测面；--verbose 状态表用）。
    fn list(&self) -> Vec<String>;
}

struct HookEntry {
    hook: Arc<dyn PreComposeHook>,
    /// 注册序（同 order 稳定排序键）。
    seq: usize,
}

/// 钩子注册表（线程安全；runner 与服务面共享同一实例）。
#[derive(Default)]
pub struct HooksRegistry {
    entries: RwLock<Vec<HookEntry>>,
    seq_next: AtomicUsize,
    /// 连败计数（按钩子名；成功清零，达阈值摘除）。
    failures: Mutex<HashMap<String, u32>>,
}

impl HooksRegistry {
    /// 注册钩子；同名 fail-loud。
    pub fn register(&self, hook: Arc<dyn PreComposeHook>) -> Result<()> {
        let mut entries = self.entries.write().expect("钩子表锁");
        if entries.iter().any(|e| e.hook.name() == hook.name()) {
            return Err(yunyou::CordisError::new(
                yunyou::ErrorKind::Internal,
                format!("钩子「{}」重复注册（同名 fail-loud）", hook.name()),
            ));
        }
        let seq = self.seq_next.fetch_add(1, Ordering::Relaxed);
        entries.push(HookEntry { hook, seq });
        Ok(())
    }

    /// 注销钩子（按名）；返回是否确有注销。
    pub fn unregister(&self, name: &str) -> bool {
        let mut entries = self.entries.write().expect("钩子表锁");
        let before = entries.len();
        entries.retain(|e| e.hook.name() != name);
        entries.len() != before
    }

    /// 在册钩子名清单（观测面）。
    pub fn list(&self) -> Vec<String> {
        self.entries
            .read()
            .expect("钩子表锁")
            .iter()
            .map(|e| e.hook.name().to_string())
            .collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.read().expect("钩子表锁").is_empty()
    }

    /// 按序执行全部钩子（错误隔离三态：Err/panic/超时→跳过+连败计数），
    /// 返回合并后的声明式指令清单。应用（含配对守卫）由 [`apply_edits`] 单点收口。
    pub(crate) async fn run(&self, request: PreComposeRequest<'_>) -> Vec<ComposeEdit> {
        if self.is_empty() {
            return Vec::new(); // 空缝零开销：无注册=行为与无钩子逐字节一致
        }
        let mut snapshot: Vec<Arc<dyn PreComposeHook>> = {
            let entries = self.entries.read().expect("钩子表锁");
            let mut hooks: Vec<Arc<dyn PreComposeHook>> =
                entries.iter().map(|e| e.hook.clone()).collect();
            hooks.sort_by_key(|h| h.order()); // sort 稳定：同序保持注册先后（push 序）
            hooks
        };
        let mut edits = Vec::new();
        for hook in snapshot.drain(..) {
            let name = hook.name().to_string();
            let fut = catch_panic(hook.on_pre_compose(&request));
            match tokio::time::timeout(HOOK_TIMEOUT, fut).await {
                Ok(Some(Ok(hook_edits))) => {
                    self.failures.lock().expect("连败表锁").insert(name.clone(), 0);
                    edits.extend(hook_edits);
                }
                Ok(Some(Err(err))) => {
                    self.record_failure(&name, format!("返回错误: {err}"));
                }
                Ok(None) => {
                    self.record_failure(&name, "panic（已捕获，跳过本轮）".to_string());
                }
                Err(_elapsed) => {
                    self.record_failure(&name, format!("超时（>{:?}）", HOOK_TIMEOUT));
                }
            }
        }
        edits
    }

    /// 连败计数：达 [`HOOK_FAILURE_TRIP`] 摘除（软态，重装即恢复）。
    fn record_failure(&self, name: &str, cause: String) {
        let mut failures = self.failures.lock().expect("连败表锁");
        let count = failures.entry(name.to_string()).or_insert(0);
        *count += 1;
        let count = *count;
        let tripped = count >= HOOK_FAILURE_TRIP;
        drop(failures);
        if tripped {
            self.unregister(name);
            tracing::warn!("[hooks] 钩子「{name}」连续失败 {count} 次已自动摘除（{cause}；重装即恢复）");
        } else {
            tracing::warn!("[hooks] 钩子「{name}」本轮跳过（{cause}；{count}/{}）", HOOK_FAILURE_TRIP);
        }
    }
}

impl HooksRegistryService for HooksRegistry {
    fn register(&self, hook: Arc<dyn PreComposeHook>) -> Result<()> {
        HooksRegistry::register(self, hook)
    }
    fn unregister(&self, name: &str) -> bool {
        HooksRegistry::unregister(self, name)
    }
    fn list(&self) -> Vec<String> {
        HooksRegistry::list(self)
    }
}

/// panic 隔离 future 包装（零新依赖）：poll 内 catch_unwind，panic→Ready(None)。
struct CatchPanic<F>(F);

impl<F: Future> Future for CatchPanic<F> {
    type Output = Option<F::Output>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // 投影到内部 future：结构体仅一层包装，字段移动安全
        let this = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| this.poll(cx))) {
            Ok(poll) => poll.map(Some),
            Err(_) => Poll::Ready(None),
        }
    }
}

fn catch_panic<F: Future>(fut: F) -> impl Future<Output = Option<F::Output>> {
    CatchPanic(fut)
}

/// 指令应用结果（观测/测试用）。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// 实际应用的指令数。
    pub applied: usize,
    /// 被配对守卫丢弃的指令数（丢弃原因走 tracing warn）。
    pub dropped: usize,
}

/// 引擎单点应用（多钩子合并语义收口；草图纸面裁定原样实现）：
/// ①同 message_index 冲突时**后到覆盖**（edits 顺序=钩子执行序，order 大者后到）；
/// ②index 降序应用（防删移位）；③**配对完整性守卫**：删 assistant 工具声明或其
/// tool 结果=拆散配对（端点拒收同族风险），StubToolResult 恒保壳安全——违例指令
/// 丢弃并 warn 点名，请求照发。
pub fn apply_edits(messages: &mut Vec<Message>, edits: &[ComposeEdit]) -> ApplyReport {
    if edits.is_empty() {
        return ApplyReport::default();
    }
    // ①同位后到覆盖（保留每下标最后一条指令）
    let mut final_edits: Vec<(usize, &ComposeEdit)> = Vec::new();
    for edit in edits {
        let index = match edit {
            ComposeEdit::Remove { message_index } => *message_index,
            ComposeEdit::StubToolResult { message_index, .. } => *message_index,
        };
        final_edits.retain(|(idx, _)| *idx != index);
        final_edits.push((index, edit));
    }
    // ②降序应用
    final_edits.sort_by_key(|(idx, _)| std::cmp::Reverse(*idx));
    let mut report = ApplyReport::default();
    for (index, edit) in final_edits {
        let Some(message) = messages.get(index) else {
            tracing::warn!("[hooks] 指令目标下标 {index} 越界（清单 {} 条），丢弃", messages.len());
            report.dropped += 1;
            continue;
        };
        // ③配对完整性守卫
        let removes_declaration = !message.tool_calls.is_empty();
        let removes_tool_result = message.tool_call_id.is_some();
        if matches!(edit, ComposeEdit::Remove { .. }) && (removes_declaration || removes_tool_result) {
            tracing::warn!(
                "[hooks] Remove 指令命中工具配对面（下标 {index}：{}），丢弃以保配对完整",
                if removes_declaration { "assistant 工具声明" } else { "tool 结果" }
            );
            report.dropped += 1;
            continue;
        }
        match edit {
            ComposeEdit::Remove { .. } => {
                messages.remove(index);
            }
            ComposeEdit::StubToolResult { stub_text, .. } => {
                if !removes_tool_result {
                    tracing::warn!("[hooks] StubToolResult 指令目标下标 {index} 非 tool 结果消息，丢弃");
                    report.dropped += 1;
                    continue;
                }
                if let Some(target) = messages.get_mut(index) {
                    target.content = stub_text.clone();
                }
            }
        }
        report.applied += 1;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_llm::Role;

    fn tool_pair() -> Vec<Message> {
        let mut call = Message::text(Role::Assistant, "");
        call.tool_calls = vec![yunyou_llm::ToolCallRequest {
            id: "c1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        }];
        vec![
            Message::text(Role::User, "跑一下"),
            call,
            Message::tool_result("c1", "很长很长的输出"),
            Message::text(Role::Assistant, "好了"),
        ]
    }

    /// 声明式纪律守卫①：Remove 命中工具声明/tool 结果=丢弃（配对完整性）。
    #[test]
    fn remove命中工具配对面被丢弃() {
        let mut messages = tool_pair();
        let report = apply_edits(
            &mut messages,
            &[ComposeEdit::Remove { message_index: 1 }],
        );
        assert_eq!(report.applied, 0);
        assert_eq!(report.dropped, 1);
        assert_eq!(messages.len(), 4, "配对面指令不得生效");
        // tool 结果同面
        let report = apply_edits(&mut messages, &[ComposeEdit::Remove { message_index: 2 }]);
        assert_eq!(report.dropped, 1);
        assert_eq!(messages.len(), 4);
    }

    /// StubToolResult：剥内容保壳（配对完好，端点可收）。
    #[test]
    fn stub剥内容保壳() {
        let mut messages = tool_pair();
        let report = apply_edits(
            &mut messages,
            &[ComposeEdit::StubToolResult {
                message_index: 2,
                stub_text: "[已折叠]".into(),
            }],
        );
        assert_eq!(report.applied, 1);
        assert_eq!(messages[2].content, "[已折叠]");
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("c1"), "tool 壳保留");
    }

    /// 普通文本消息 Remove 放行（守卫只拦配对面）。
    #[test]
    fn remove普通文本放行() {
        let mut messages = tool_pair();
        let report = apply_edits(&mut messages, &[ComposeEdit::Remove { message_index: 3 }]);
        assert_eq!(report.applied, 1);
        assert_eq!(messages.len(), 3);
    }

    /// 同 index 后到覆盖：两条指令同位，后到者生效。
    #[test]
    fn 同位后到覆盖() {
        let mut messages = tool_pair();
        let report = apply_edits(
            &mut messages,
            &[
                ComposeEdit::Remove { message_index: 2 },
                ComposeEdit::StubToolResult { message_index: 2, stub_text: "后者".into() },
            ],
        );
        assert_eq!(report.applied, 1, "合并后仅一条指令生效");
        assert_eq!(messages.len(), 4, "Remove 被后到的 Stub 覆盖");
        assert_eq!(messages[2].content, "后者");
    }

    /// 越界下标丢弃不炸。
    #[test]
    fn 越界下标丢弃() {
        let mut messages = tool_pair();
        let report = apply_edits(&mut messages, &[ComposeEdit::Remove { message_index: 99 }]);
        assert_eq!(report.dropped, 1);
        assert_eq!(messages.len(), 4);
    }

    /// 空指令清单=零操作。
    #[test]
    fn 空指令零操作() {
        let mut messages = tool_pair();
        let report = apply_edits(&mut messages, &[]);
        assert_eq!(report, ApplyReport::default());
    }

    /// 注册表面：同名 fail-loud/注销/清单。
    #[test]
    fn 注册表同名fail_loud与注销() {
        struct Nop;
        impl PreComposeHook for Nop {
            fn name(&self) -> &str {
                "nop"
            }
            fn on_pre_compose(
                &self,
                _request: &PreComposeRequest<'_>,
            ) -> BoxFut<'_, Result<Vec<ComposeEdit>>> {
                Box::pin(async { Ok(Vec::new()) })
            }
        }
        let registry = HooksRegistry::default();
        registry.register(Arc::new(Nop)).expect("首注应成功");
        let err = registry.register(Arc::new(Nop)).expect_err("同名应 fail-loud");
        assert!(err.to_string().contains("重复注册"));
        assert_eq!(registry.list(), vec!["nop"]);
        assert!(registry.unregister("nop"));
        assert!(!registry.unregister("nop"), "重复注销=false");
        assert!(registry.list().is_empty());
    }

    /// 隔离三态：Err/panic/超时钩子均跳过、正常钩子指令仍生效；连败 3 次摘除。
    #[tokio::test(start_paused = true)]
    async fn 隔离三态与连败摘除() {
        struct Bad;
        impl PreComposeHook for Bad {
            fn name(&self) -> &str {
                "bad"
            }
            fn on_pre_compose(
                &self,
                _request: &PreComposeRequest<'_>,
            ) -> BoxFut<'_, Result<Vec<ComposeEdit>>> {
                Box::pin(async {
                    Err(yunyou::CordisError::new(yunyou::ErrorKind::Internal, "钩子故障"))
                })
            }
        }
        struct Panics;
        impl PreComposeHook for Panics {
            fn name(&self) -> &str {
                "panics"
            }
            fn on_pre_compose(
                &self,
                _request: &PreComposeRequest<'_>,
            ) -> BoxFut<'_, Result<Vec<ComposeEdit>>> {
                Box::pin(async { panic!("钩子爆了") })
            }
        }
        struct Slow;
        impl PreComposeHook for Slow {
            fn name(&self) -> &str {
                "slow"
            }
            fn on_pre_compose(
                &self,
                _request: &PreComposeRequest<'_>,
            ) -> BoxFut<'_, Result<Vec<ComposeEdit>>> {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok(Vec::new())
                })
            }
        }
        struct Good;
        impl PreComposeHook for Good {
            fn name(&self) -> &str {
                "good"
            }
            fn on_pre_compose(
                &self,
                request: &PreComposeRequest<'_>,
            ) -> BoxFut<'_, Result<Vec<ComposeEdit>>> {
                // 借用数据在装箱前取出：async move 块零捕获，双入参生命周期
                // 歧义（'self vs 'request）即消失
                let last_index = request.messages.len().saturating_sub(1);
                Box::pin(async move {
                    Ok(vec![ComposeEdit::StubToolResult {
                        message_index: last_index,
                        stub_text: "置尾".into(),
                    }])
                })
            }
        }
        let registry = HooksRegistry::default();
        registry.register(Arc::new(Bad)).unwrap();
        registry.register(Arc::new(Panics)).unwrap();
        registry.register(Arc::new(Slow)).unwrap();
        registry.register(Arc::new(Good)).unwrap();
        let messages = vec![Message::text(Role::User, "hi"), Message::text(Role::Assistant, "ok")];
        let req = PreComposeRequest { messages: &messages, steps_used: 0, context_tokens: None, context_window: None };
        let edits = registry.run(req).await;
        assert_eq!(edits.len(), 1, "三个故障钩子跳过，正常钩子指令应到达");
        // 连败 3 次摘除（good 成功清零不影响 bad 计数）
        for _ in 0..2 {
            let req = PreComposeRequest { messages: &messages, steps_used: 0, context_tokens: None, context_window: None };
            registry.run(req).await;
        }
        assert!(!registry.list().contains(&"bad".to_string()), "bad 连败 3 次应被摘除");
        assert!(registry.list().contains(&"good".to_string()), "成功钩子不受连败影响");
    }
}
