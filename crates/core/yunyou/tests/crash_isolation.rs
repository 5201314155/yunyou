//! 崩溃隔离混沌夹具·核心注入面（task-11 / S-ISO-4，《插件崩溃隔离设计》§6+附录 A）。
//!
//! 覆盖：C1 持锁注册体 panic / C2+C3 主板存活与扫描不击穿 / C4 可重启用 /
//! C7 setup panic（幽灵态回归）/ C8 busy 复位 / C6 waterfall 环 panic（批甲语义，
//! 与 veto-Ok 分立）/ C9 二次装载 panic 消费方不陪炸。
//! C12/C13（插拔 N 次内存回归）见 crash_memory.rs；emit/parallel/serial 三分发
//! 既有覆盖见 lifecycle.rs `listener_panic_is_contained`。C10（R1 在途收流）随
//! S7-R1 另批占位。断言风格：状态+事件序列+强计数，不依赖多线程时序（§A.4-2）。

mod crash_fixtures;

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use yunyou::keys;
use yunyou::{
    ConfigValue, Context, ErrorKind, FiberOutput, FiberState, Plugin, Result, ServiceId,
    WaterfallBuiltin, WaterfallNext,
};
use yunyou_util::BoxFut;
use crash_fixtures::{
    new_log, status_logger, wait_state, BarePlugin, HealthyPlugin, PoisonMode, PoisonPlugin, Probe,
};

// ── C1：持锁注册体 panic → Failed + PluginPanicked + 幽灵态回归断言 ──

/// C1：ctx.effect 注册体 panic（三凶点唯一锁内路径）。
/// 断言：setup 经 `?` 传播 → fiber Failed（非 Loading，幽灵态回归）；错误
/// kind=PluginPanicked 且消息含 panic 原文；status 事件序列含 loading→failed。
#[tokio::test(flavor = "multi_thread")]
async fn c1_effect注册体panic归一failed() {
    let root = Context::new();
    let statuses = new_log();
    root.on(keys::STATUS, status_logger(Arc::clone(&statuses), "甲".to_string())).unwrap();

    let (poison, slot, _healed) = PoisonPlugin::new("毒化甲", PoisonMode::Effect);
    let handle = root.plugin(poison, json!({}));
    let err = handle.await_ready().await.expect_err("注册体 panic 应致装载失败");
    wait_state(&handle, FiberState::Failed).await;
    // 幽灵态回归断言（凶点一验收面）：失败落 Failed，绝非常驻 Loading
    assert_eq!(handle.state(), FiberState::Failed);
    // panic 归一：插件侧暂存与 await_ready 抛出双点同 kind 同消息
    let stored_kind = {
        let slot_guard = slot.lock().unwrap();
        slot_guard.as_ref().map(|e| e.kind().clone())
    };
    let _stored = stored_kind.as_ref().ok_or_else(|| panic!("panic 归一存储应在"));
    assert!(err.message().contains("注册体故意爆炸"), "{}", err.message());
    assert_eq!(err.kind(), &ErrorKind::PluginPanicked, "await_ready 抛存储错误");
    assert!(err.message().contains("注册体故意爆炸"), "{err}");
    // 状态事件序列含 loading→failed（装卸引擎收尾链走通）。注：status 事件经
    // set_state 的 spawn 异步发出，与 watch 状态更新不同步——有界轮询等它落账。
    let mut seq = String::new();
    for _ in 0..400 {
        seq = statuses.lock().unwrap().join(",");
        if seq.contains("[甲]loading→failed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(seq.contains("[甲]loading→failed"), "状态序列：{seq}");
}

// ── C2+C3：崩溃后主板存活 + 全表扫描不击穿 ─────────────────────────

/// C2+C3：承 C1 崩溃后——主板存活（健康插件服务可达/事件分发正常）、
/// 全表扫描路径无 panic（第三插件装卸 + notify_services 驱动崩 fiber 重算 epoch）。
/// 「同锁不中毒」由 C4 的 restart 成功直接证明（restart 首步即抢同一把决策锁——
/// 若 C1 的锁内 panic 中毒了锁，此处 `.expect("fiber 决策锁")` 连环炸）。
#[tokio::test(flavor = "multi_thread")]
async fn c2_c3_崩溃后主板存活与扫描不击穿() {
    let root = Context::new();
    // ① 先放一个健康插件（崩溃后它必须活着）
    let healthy = root.plugin(
        HealthyPlugin::new("健康乙"),
        json!({}),
    );
    healthy.await_ready().await.expect("健康插件装载");

    // ② 毒化甲崩溃
    let (poison, _slot, _healed) = PoisonPlugin::new("毒化甲", PoisonMode::Effect);
    let poison_handle = root.plugin(poison, json!({}));
    let _ = poison_handle.await_ready().await;
    wait_state(&poison_handle, FiberState::Failed).await;

    // ③ 主板存活：健康插件服务 get 正常 + 事件分发正常
    let probe = root.get::<dyn Probe>(ServiceId::of::<dyn Probe>()).expect("服务应可达");
    assert_eq!(probe.ping(), 7);
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen2 = Arc::clone(&seen);
    root.on("c2/ping", Arc::new(move |_args: &[Value]| {
        let seen = Arc::clone(&seen2);
        Box::pin(async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(yunyou::ListenerOutcome::Continue) as Result<_>
        })
    }))
    .unwrap();
    root.emit("c2/ping", &[]);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1, "事件分发应正常");

    // ④ 扫描不击穿：第三插件装卸触发 notify_services 全表扫描（含崩 fiber drive）
    let third = root.plugin(BarePlugin::new("空丙"), json!({}));
    third.await_ready().await.expect("第三插件应正常装载（扫描路径锁可用）");
    third.dispose().await.expect("第三插件应正常卸载");
    assert_eq!(healthy.state(), FiberState::Active, "健康插件不受牵连");
}

// ── C4：可重启用（restart 复活 = 同锁不中毒的直接证据） ─────────────

/// C4：healed 置位后 restart 复活（Failed 唯一复位手段）；
/// restart 首步抢崩 fiber 的决策锁——若 C1 锁内 panic 中毒了锁，此处即连环炸
/// （云枢验收补充①：std 锁方案的存亡条款）。
#[tokio::test(flavor = "multi_thread")]
async fn c4_崩溃后restart复活() {
    let root = Context::new();
    let (poison, slot, healed) = PoisonPlugin::new("毒化甲", PoisonMode::EffectWithHeal);
    let handle = root.plugin(poison, json!({}));
    let _ = handle.await_ready().await;
    wait_state(&handle, FiberState::Failed).await;
    assert_eq!(*slot.lock().unwrap().as_ref().unwrap().kind(), ErrorKind::PluginPanicked);

    // 痊愈开关（与 fiber 内插件克隆共享同一 Arc）+ restart
    healed.store(true, Ordering::SeqCst);
    handle.restart().await.expect("restart 应成功（决策锁未中毒=同锁不毒证据）");
    wait_state(&handle, FiberState::Active).await;
    assert_eq!(handle.state(), FiberState::Active);
}

// ── C7+C8：setup panic → Failed（幽灵态回归）+ busy 复位可重复 ───────

/// C7：setup future 内 panic（凶点一）——spawn 边界截获，fiber 落 Failed 而非
/// 幽灵 Loading；await_ready 抛 PluginPanicked（载荷含 panic 原文）。
#[tokio::test(flavor = "multi_thread")]
async fn c7_setup_panic落failed非幽灵loading() {
    let root = Context::new();
    let (poison, _slot, _healed) = PoisonPlugin::new("毒化丙", PoisonMode::Setup);
    let handle = root.plugin(poison, json!({}));
    let err = handle
        .await_ready()
        .await
        .expect_err("setup panic 应致装载失败（装卸任务存活=无幽灵 Loading）");
    assert_eq!(handle.state(), FiberState::Failed, "幽灵态回归主断言：Failed 而非 Loading");
    assert_eq!(err.kind(), &ErrorKind::PluginPanicked);
    assert!(err.message().contains("setup 故意爆炸"), "{err}");
}

/// C8：承 C7——busy 复位的可重复性：连续 restart 各返回 PluginPanicked、状态
/// 恒回 Failed、不悬挂不幽灵；主板依旧可装载健康插件。
#[tokio::test(flavor = "multi_thread")]
async fn c8_setup崩溃后restart复位busy() {
    let root = Context::new();
    let (poison, _slot, _healed) = PoisonPlugin::new("毒化丙", PoisonMode::Setup);
    let handle = root.plugin(poison, json!({}));
    let _ = handle.await_ready().await;
    wait_state(&handle, FiberState::Failed).await;
    for _ in 0..3 {
        let err = handle.restart().await.expect_err("Setup 模式 restart 仍应炸");
        assert_eq!(err.kind(), &ErrorKind::PluginPanicked, "{err}");
        wait_state(&handle, FiberState::Failed).await;
    }
    let healthy = root.plugin(HealthyPlugin::new("健康乙"), json!({}));
    healthy.await_ready().await.expect("反复崩溃后主板仍应可装载健康插件");
}

// ── C6：waterfall 监听器 panic（批甲语义）——与 veto-Ok 链停分立 ─────────

/// C6-a：监听器环 panic → Err(PluginPanicked)、链停、builtin 不执行（fail-closed）。
/// C6-b（同用例分立断言）：veto（不调 next、返回 Ok）→ 链停但 **Ok**、builtin 不执行
/// ——两者 Outcome 可区分（Err vs Ok），防「panic 被当合法否决」的语义坍缩（云枢批甲细则）。
#[tokio::test(flavor = "multi_thread")]
async fn c6_waterfall_panic链停与veto分立() {
    let root = Context::new();
    // C6-a：panic 环
    root.on_waterfall(
        "c6/panic",
        Arc::new(|_args: &[Value], _next: WaterfallNext| -> BoxFut<'static, Result<Value>> {
            Box::pin(async { panic!("瀑布监听器故意爆炸") })
        }),
    )
    .unwrap();
    let builtin_ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&builtin_ran);
    let builtin: WaterfallBuiltin = Arc::new(move |_args| {
        let flag = Arc::clone(&flag);
        Box::pin(async move {
            flag.store(true, Ordering::SeqCst);
            Ok(json!("builtin"))
        })
    });
    let err = root.waterfall("c6/panic", &[], builtin).await.expect_err("panic 环应归一 Err");
    assert_eq!(err.kind(), &ErrorKind::PluginPanicked, "{err}");
    assert!(err.message().contains("瀑布监听器故意爆炸"), "{err}");
    assert!(!builtin_ran.load(Ordering::SeqCst), "panic 链停：builtin 不得执行");

    // C6-b：veto（合法否决）——同为链停，但 Outcome 是 Ok（无 builtin）
    root.on_waterfall(
        "c6/veto",
        Arc::new(|_args: &[Value], _next: WaterfallNext| -> BoxFut<'static, Result<Value>> {
            Box::pin(async { Ok(json!("vetoed")) })
        }),
    )
    .unwrap();
    let builtin_ran2 = Arc::new(AtomicBool::new(false));
    let flag2 = Arc::clone(&builtin_ran2);
    let builtin2: WaterfallBuiltin = Arc::new(move |_args| {
        let flag = Arc::clone(&flag2);
        Box::pin(async move {
            flag.store(true, Ordering::SeqCst);
            Ok(json!("builtin"))
        })
    });
    let value = root.waterfall("c6/veto", &[], builtin2).await.expect("veto 应 Ok");
    assert_eq!(value, json!("vetoed"));
    assert!(!builtin_ran2.load(Ordering::SeqCst), "veto 链停：builtin 同样不执行");
    // 主板存活
    let healthy = root.plugin(HealthyPlugin::new("健康乙"), json!({}));
    healthy.await_ready().await.expect("waterfall panic 后主板仍可装载");
}

// ── C9：二次装载 panic 链——消费方「依赖未就绪」不陪炸 + epoch 复活 ────────

/// C9：提供方首次装载 Active+provide → 消费方 Active；update 打开 panic 开关 →
/// 提供方 Failed；消费方被驱动卸载 → Pending（依赖未就绪）；update 关开关 →
/// 提供方复活 Active；消费方 epoch 重算自动重装 Active。
#[tokio::test(flavor = "multi_thread")]
async fn c9_二次装载panic消费方不陪炸() {
    let root = Context::new();
    let (poison, _slot, _healed) = PoisonPlugin::new("毒化丁", PoisonMode::ConfigGate);
    let provider = root.plugin(poison, json!({}));
    provider.await_ready().await.expect("首次装载应成功");
    wait_state(&provider, FiberState::Active).await;

    let consumer = root.plugin(ConsumerOfProbe { tag: "消费戊" }, json!({}));
    consumer.await_ready().await.expect("消费方应装载");
    wait_state(&consumer, FiberState::Active).await;

    // 打开 panic 开关：update → internal/update waterfall（无监听者 → builtin 重启）
    // → 二次 setup panic → spawn 边界 → Failed
    provider.update(json!({"panic": true})).await.expect_err("二次装载应失败");
    wait_state(&provider, FiberState::Failed).await;
    // 消费方被驱动卸载 → Pending（strict 语义：提供者非 Active 其服务不可见）
    wait_state(&consumer, FiberState::Pending).await;

    // 关开关：Failed fiber 的 update 走 restart_inner → 复活
    provider.update(json!({})).await.expect("关开关后 update（=restart）应成功");
    wait_state(&provider, FiberState::Active).await;
    // 消费方 epoch 重算自动重装（epoch 机制验收）
    wait_state(&consumer, FiberState::Active).await;
    let probe = root.get::<dyn Probe>(ServiceId::of::<dyn Probe>()).expect("服务应重新可见");
    assert_eq!(probe.ping(), 7);
}

/// 消费方插件（C9 专用）：inject Probe。
struct ConsumerOfProbe {
    tag: &'static str,
}

impl Plugin for ConsumerOfProbe {
    fn name(&self) -> &str {
        self.tag
    }
    fn inject(&self) -> Vec<ServiceId> {
        vec![ServiceId::of::<dyn Probe>()]
    }
    fn setup(self: Arc<Self>, _ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move { Ok(FiberOutput) })
    }
}

