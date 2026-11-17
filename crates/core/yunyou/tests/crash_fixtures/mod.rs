//! 崩溃隔离混沌夹具·公共设施（task-11/S-ISO-4 拆分件）。
//!
//! 按家规四拆分：本模块=插件夹具与测试惯用法（wait_state/status_logger/探针服务/
//! 毒化插件五件），被 crash_isolation.rs（C1-C9）与 crash_memory.rs（C12/C13）
//! 共用；行数纪律：测试文件主文件≤400，夹具独立成模。

// 多靶共享夹具：单靶视角（crash_memory 不用 HealthyPlugin 族）unused 是
// 结构现象非死代码（crash_isolation 5 处使用实证）——头注 allow 收口。
#![allow(dead_code)]

use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yunyou::{
    ConfigValue, Context, CordisError, Disposer, FiberHandle, FiberOutput, FiberState, Listener,
    Plugin, Result, ServiceId,
};
use yunyou_util::BoxFut;

pub(crate) type Log = Arc<Mutex<Vec<String>>>;

pub(crate) fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

pub(crate) fn push(log: &Log, entry: impl Into<String>) {
    log.lock().unwrap().push(entry.into());
}

/// 轮询等待 fiber 到达目标状态（装卸任务异步推进；照搬 lifecycle.rs 惯用法）。
pub(crate) async fn wait_state(handle: &FiberHandle, want: FiberState) {
    for _ in 0..400 {
        if handle.state() == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等待状态 {want:?} 超时，当前 {:?}", handle.state());
}

/// status 事件采集器（root 侧收集 `旧态→新态` 序列，设计 §A.1 事件序列断言载体）。
pub(crate) fn status_logger(log: Log, tag: String) -> Listener {
    Arc::new(move |args: &[Value]| {
        let log = Arc::clone(&log);
        let tag = tag.clone();
        let old = args[1].as_str().unwrap_or("?").to_string();
        let new = args[2].as_str().unwrap_or("?").to_string();
        Box::pin(async move {
            push(&log, format!("[{tag}]{old}→{new}"));
            Ok(yunyou::ListenerOutcome::Continue) as Result<_>
        })
    })
}

// ── 探针服务与健康插件（主板存活探针 + 强计数观测点） ───────────────

pub(crate) trait Probe: Send + Sync {
    fn ping(&self) -> u8;
    /// ContextInner 强计数观测（服务内藏 WeakContext，lifecycle.rs:398 模式同源）。
    fn strong_count(&self) -> usize;
}

/// 健康插件：provide 计数探针（提供方）。
pub(crate) struct HealthyPlugin {
    tag: &'static str,
    log: Log,
}

impl HealthyPlugin {
    pub(crate) fn new(tag: &'static str) -> Self {
        Self { tag, log: new_log() }
    }
}

impl Plugin for HealthyPlugin {
    fn name(&self) -> &str {
        self.tag
    }
    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let probe: Arc<dyn Probe> = Arc::new(ProbeImpl);
            ctx.provide(ServiceId::of::<dyn Probe>(), probe)?;
            push(&self.log, format!("load:{}", self.tag));
            Ok(FiberOutput)
        })
    }
}

pub(crate) struct ProbeImpl;
impl Probe for ProbeImpl {
    fn ping(&self) -> u8 {
        7
    }
    fn strong_count(&self) -> usize {
        0 // 非计数档（仅 C2/C9 ping 用）；计数档见 CountedProbe
    }
}

/// 计数档探针：setup 时抓 WeakContext，暴露强计数（C12 断言载体）。
pub(crate) struct CountingHealthy {
    tag: &'static str,
}

impl CountingHealthy {
    pub(crate) fn new(tag: &'static str) -> Self {
        Self { tag }
    }
}

impl Plugin for CountingHealthy {
    fn name(&self) -> &str {
        self.tag
    }
    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let probe: Arc<dyn Probe> = Arc::new(CountedProbe { weak: ctx.downgrade() });
            ctx.provide(ServiceId::of::<dyn Probe>(), probe)?;
            Ok(FiberOutput)
        })
    }
}

struct CountedProbe {
    weak: yunyou::WeakContext,
}
impl Probe for CountedProbe {
    fn ping(&self) -> u8 {
        7
    }
    fn strong_count(&self) -> usize {
        self.weak.strong_count()
    }
}

/// 空插件（C3/C12 扫描活性探针：装卸动作真实发生但不占服务槽——
/// 同作用域重复 provide 同一 ServiceId=ServiceDuplicate，扫描探针必须零 provide）。
pub(crate) struct BarePlugin {
    tag: &'static str,
}

impl BarePlugin {
    pub(crate) fn new(tag: &'static str) -> Self {
        Self { tag }
    }
}

impl Plugin for BarePlugin {
    fn name(&self) -> &str {
        self.tag
    }
    fn setup(self: Arc<Self>, _ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move { Ok(FiberOutput) })
    }
}

// ── 毒化插件（注入点按 mode 分派，设计 §A.1） ──────────────────────

/// 注入点：`Setup`=setup future 内 panic（凶点一）；`Effect`=ctx.effect 注册体
/// panic（凶点二，决策锁内）；`EffectWithHeal`=同 Effect 但 healed 置位后正常
/// （C4 可重启用）；`ConfigGate`=按 config["panic"] 开关决定（C9 二次装载）。
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum PoisonMode {
    Setup,
    Effect,
    EffectWithHeal,
    ConfigGate,
}

pub(crate) struct PoisonPlugin {
    tag: String,
    mode: PoisonMode,
    /// C4 痊愈开关（Arc 三方共享：测试侧 / fiber 内插件克隆）。
    healed: Arc<AtomicBool>,
    /// C1：effect 注册体返回的 Err 原样暂存——PluginPanicked kind 的插件侧断言点
    /// （setup 的 `?` 传播链上，插件是唯一能拿到该 Err 的位置）。
    effect_error: Arc<Mutex<Option<CordisError>>>,
    log: Log,
}

impl PoisonPlugin {
    pub(crate) fn new(tag: impl Into<String>, mode: PoisonMode) -> (Self, Arc<Mutex<Option<CordisError>>>, Arc<AtomicBool>) {
        let slot = Arc::new(Mutex::new(None));
        let healed = Arc::new(AtomicBool::new(false));
        (
            Self {
                tag: tag.into(),
                mode,
                healed: Arc::clone(&healed),
                effect_error: Arc::clone(&slot),
                log: new_log(),
            },
            slot,
            healed,
        )
    }
}

impl Plugin for PoisonPlugin {
    fn name(&self) -> &str {
        &self.tag
    }
    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            match self.mode {
                PoisonMode::Setup => {
                    panic!("setup 故意爆炸（凶点一注入）");
                }
                PoisonMode::Effect | PoisonMode::EffectWithHeal => {
                    let healed = self.healed.clone();
                    let outcome = ctx.effect("毒化注册体", move || -> Result<Option<Disposer>> {
                        if healed.load(Ordering::SeqCst) {
                            return Ok(None); // C4：痊愈后注册体正常
                        }
                        panic!("注册体故意爆炸（凶点二注入，决策锁内）");
                    });
                    if let Err(err) = outcome {
                        *self.effect_error.lock().unwrap() = Some(err.clone());
                        return Err(err); // 传播 → finish_load_error → Failed
                    }
                    let _ = outcome;
                    push(&self.log, format!("load:{}", self.tag));
                    Ok(FiberOutput)
                }
                PoisonMode::ConfigGate => {
                    if config.get("panic").and_then(Value::as_bool).unwrap_or(false) {
                        panic!("二次装载故意爆炸（C9 注入）");
                    }
                    let probe: Arc<dyn Probe> = Arc::new(ProbeImpl);
                    ctx.provide(ServiceId::of::<dyn Probe>(), probe)?;
                    push(&self.log, format!("load:{}", self.tag));
                    Ok(FiberOutput)
                }
            }
        })
    }
}

