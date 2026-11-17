//! lifecycle 回归：共享任务池 task-2 验收标准第 2 条的全部场景。
//!
//! 覆盖：依赖就绪自动装载 / 依赖消失自动卸载、先消费者后提供者的卸载顺序、
//! effect 逆序清理、update waterfall（保留实例 / 重启 / 重入抛错 / 否决短路）、
//! Weak 依赖者存续时服务实例存活、setup 落定后 Context 强句柄释放（方案 §2.1.2 修订）。
//!
//! 配置/作用域/错误路径类场景见 tests/config.rs。

use yunyou::{
    ConfigValue, Context, CordisError, Disposer, ErrorKind, FiberHandle, FiberOutput, FiberState,
    Listener, ListenerOutcome, Plugin, Result, ServiceId, WaterfallBuiltin,
};
use yunyou_util::BoxFut;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// 测试共享的顺序日志。
type Log = Arc<Mutex<Vec<String>>>;

fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn push(log: &Log, entry: impl Into<String>) {
    log.lock().unwrap().push(entry.into());
}

fn snapshot(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// 轮询等待 fiber 到达目标状态（装卸任务异步推进）。
async fn wait_state(handle: &FiberHandle, want: FiberState) {
    for _ in 0..400 {
        if handle.state() == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等待状态 {want:?} 超时，当前 {:?}", handle.state());
}

/// 契约服务（object-safe：无泛型方法、无 Self 返回值，方案 §2.1.3）。
trait Greeter: Send + Sync {
    fn greet(&self) -> String;
}

/// 服务实现：析构时记录（验证 Weak 存续语义的析构时机）。
struct Greeting {
    tag: &'static str,
    log: Log,
}

impl Greeter for Greeting {
    fn greet(&self) -> String {
        format!("hello from {}", self.tag)
    }
}

impl Drop for Greeting {
    fn drop(&mut self) {
        push(&self.log, format!("drop:{}", self.tag));
    }
}

/// 提供者插件：setup 里 provide `Greeter`。
struct ProviderPlugin {
    tag: &'static str,
    log: Log,
}

impl Plugin for ProviderPlugin {
    fn name(&self) -> &str {
        self.tag
    }

    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let greeter: Arc<dyn Greeter> = Arc::new(Greeting { tag: self.tag, log: self.log.clone() });
            ctx.provide(ServiceId::of::<dyn Greeter>(), greeter)?;
            push(&self.log, format!("load:{}", self.tag));
            Ok(FiberOutput)
        })
    }
}

/// 消费者插件：依赖 `Greeter`；卸载 probe 在 teardown 期间回访实例（Weak 升级验证）。
struct ConsumerPlugin {
    tag: &'static str,
    log: Log,
    /// setup 里降级出的弱引用（不持强引用，避免测试自身救活实例）。
    probe_slot: Arc<Mutex<Option<Weak<dyn Greeter>>>>,
}

impl ConsumerPlugin {
    fn new(tag: &'static str, log: Log) -> Self {
        Self { tag, log, probe_slot: Arc::new(Mutex::new(None)) }
    }
}

impl Plugin for ConsumerPlugin {
    fn name(&self) -> &str {
        self.tag
    }

    fn inject(&self) -> Vec<ServiceId> {
        vec![ServiceId::of::<dyn Greeter>()]
    }

    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let greeter = ctx.get::<dyn Greeter>(ServiceId::of::<dyn Greeter>())?;
            *self.probe_slot.lock().unwrap() = Some(Arc::downgrade(&greeter));
            push(&self.log, format!("load:{}", self.tag));
            let slot = self.probe_slot.clone();
            let log = self.log.clone();
            ctx.effect("teardown-probe", move || {
                Ok(Some(Disposer::new(move || {
                    let slot = slot.clone();
                    let log = log.clone();
                    Box::pin(async move {
                        // 卸载期间实例必须仍可用（服务表强引用已撤，靠依赖方弱引用维持）
                        if let Some(greeter) = slot.lock().unwrap().as_ref().and_then(Weak::upgrade) {
                            push(&log, format!("teardown-call:{}", greeter.greet()));
                        }
                    })
                })))
            })?;
            Ok(FiberOutput)
        })
    }
}

/// 计数插件：记录 setup 次数（重启 ⇔ setup 重跑）。
struct CountingPlugin {
    tag: &'static str,
    loads: Arc<std::sync::atomic::AtomicUsize>,
}

impl CountingPlugin {
    fn new(tag: &'static str) -> (Self, Arc<std::sync::atomic::AtomicUsize>) {
        let loads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (Self { tag, loads: loads.clone() }, loads)
    }
}

impl Plugin for CountingPlugin {
    fn name(&self) -> &str {
        self.tag
    }

    fn setup(self: Arc<Self>, _ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            self.loads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(FiberOutput)
        })
    }
}

/// 验收 1：插件注册 → 装载 → Active，服务可用。
#[tokio::test(flavor = "multi_thread")]
async fn basic_load_and_active() {
    let root = Context::new();
    let log = new_log();
    let provider = root.plugin(ProviderPlugin { tag: "provider", log: log.clone() }, json!({}));
    provider.await_ready().await.expect("应装载成功");
    assert_eq!(provider.state(), FiberState::Active);
    let greeter = root
        .get::<dyn Greeter>(ServiceId::of::<dyn Greeter>())
        .expect("服务应可见");
    assert_eq!(greeter.greet(), "hello from provider");
    assert_eq!(snapshot(&log), vec!["load:provider"]);
}

/// 验收 2：依赖就绪自动装载——缺依赖时 Pending，提供者出现后自动 Active。
#[tokio::test(flavor = "multi_thread")]
async fn dependency_ready_triggers_load() {
    let root = Context::new();
    let log = new_log();
    let consumer = root.plugin(ConsumerPlugin::new("consumer", log.clone()), json!({}));
    assert_eq!(consumer.state(), FiberState::Pending, "缺依赖应保持 Pending");
    let _provider = root.plugin(ProviderPlugin { tag: "provider", log: log.clone() }, json!({}));
    // 依赖就绪 → 自动装载（轮询等待，装卸是异步任务）
    wait_state(&consumer, FiberState::Active).await;
    assert_eq!(
        snapshot(&log),
        vec!["load:provider", "load:consumer"],
        "装载顺序：提供者先就绪，消费者后装载"
    );
}

/// 验收 2：依赖消失自动卸载——提供者卸载后消费者回 Pending。
#[tokio::test(flavor = "multi_thread")]
async fn dependency_lost_triggers_unload() {
    let root = Context::new();
    let log = new_log();
    let consumer = root.plugin(ConsumerPlugin::new("consumer", log.clone()), json!({}));
    let provider = root.plugin(ProviderPlugin { tag: "provider", log: log.clone() }, json!({}));
    provider.await_ready().await.expect("提供者应装载");
    wait_state(&consumer, FiberState::Active).await;
    provider.dispose().await.expect("提供者应卸载");
    wait_state(&consumer, FiberState::Pending).await;
    assert_eq!(consumer.state(), FiberState::Pending);
    assert_eq!(
        snapshot(&log),
        vec!["load:provider", "load:consumer", "teardown-call:hello from provider", "drop:provider"],
        "消费者先卸载（teardown 期间实例仍可用），服务实例随后析构"
    );
}

/// 验收 3：先消费者后提供者的卸载顺序（dispose 提供者，断言清理顺序）。
#[tokio::test(flavor = "multi_thread")]
async fn unload_order_consumer_before_provider() {
    let root = Context::new();
    let log = new_log();
    let consumer = root.plugin(ConsumerPlugin::new("consumer", log.clone()), json!({}));
    let provider = root.plugin(ProviderPlugin { tag: "provider", log: log.clone() }, json!({}));
    provider.await_ready().await.expect("提供者应装载");
    wait_state(&consumer, FiberState::Active).await;
    provider.dispose().await.expect("应卸载");
    wait_state(&consumer, FiberState::Pending).await;
    let order: Vec<String> = snapshot(&log)
        .into_iter()
        .filter(|entry| entry.starts_with("teardown-call"))
        .collect();
    assert_eq!(
        order.len(),
        1,
        "消费者的 teardown 探针应恰好执行一次（先于提供者清理完成）"
    );
    assert!(order[0].contains("hello from provider"), "卸载期间实例仍可调用");
}

/// 验收 4：effect 逆序清理（后注册的先清理）。
#[tokio::test(flavor = "multi_thread")]
async fn effects_disposed_in_reverse() {
    let root = Context::new();
    let log = new_log();
    struct EffectsPlugin {
        log: Log,
    }
    impl Plugin for EffectsPlugin {
        fn name(&self) -> &str {
            "effects"
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                for tag in ["a", "b", "c"] {
                    let log = self.log.clone();
                    ctx.effect(format!("effect-{tag}"), move || {
                        Ok(Some(Disposer::new(move || {
                            let log = log.clone();
                            let tag = tag;
                            Box::pin(async move { push(&log, format!("dispose:{tag}")) })
                        })))
                    })?;
                }
                Ok(FiberOutput)
            })
        }
    }
    let handle = root.plugin(EffectsPlugin { log: log.clone() }, json!({}));
    handle.await_ready().await.expect("应装载");
    handle.dispose().await.expect("应卸载");
    handle.wait_disposed().await;
    assert_eq!(
        snapshot(&log),
        vec!["dispose:c", "dispose:b", "dispose:a"],
        "effect 清理必须与注册顺序相反"
    );
}

/// 验收 5a：update waterfall——监听者不调 next() 即保留实例（不重启）。
#[tokio::test(flavor = "multi_thread")]
async fn update_keeps_instance_without_next() {
    let root = Context::new();
    let (plugin, loads) = CountingPlugin::new("counter");
    let handle = root.plugin(plugin, json!({}));
    handle.await_ready().await.expect("应装载");
    // 否决型监听器：不调 next()，update 应视为成功且不重启
    let listener = Arc::new(|_args: &[Value], _next: yunyou::WaterfallNext| {
        Box::pin(async { Ok(json!("kept")) }) as BoxFut<'static, Result<Value>>
    });
    root.on_waterfall(yunyou::keys::UPDATE, listener).expect("应注册");
    handle.update(json!({"verbose": true})).await.expect("否决应视为成功");
    assert_eq!(
        loads.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "不调 next() 不得重启实例"
    );
    assert_eq!(handle.state(), FiberState::Active);
}

/// 验收 5b：update waterfall——无监听者（或调 next()）走内置重启，setup 重跑。
#[tokio::test(flavor = "multi_thread")]
async fn update_restarts_via_next() {
    let root = Context::new();
    let (plugin, loads) = CountingPlugin::new("counter");
    let handle = root.plugin(plugin, json!({}));
    handle.await_ready().await.expect("应装载");
    handle.update(json!({"verbose": true})).await.expect("应重启成功");
    assert_eq!(
        loads.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "内置 next()（重启）应重跑 setup"
    );
    assert_eq!(handle.state(), FiberState::Active);
}

/// 验收 5c：update waterfall——next() 重复调用报 WaterfallReentry。
#[tokio::test(flavor = "multi_thread")]
async fn update_waterfall_reentry_rejected() {
    let root = Context::new();
    let (plugin, loads) = CountingPlugin::new("counter");
    let handle = root.plugin(plugin, json!({}));
    handle.await_ready().await.expect("应装载");
    let listener = Arc::new(|_args: &[Value], next: yunyou::WaterfallNext| {
        Box::pin(async move {
            let _ = next.call().await?; // 第一次合法（触发重启）
            next.call().await // 第二次必须报重入
        }) as BoxFut<'static, Result<Value>>
    });
    root.on_waterfall(yunyou::keys::UPDATE, listener).expect("应注册");
    let err = handle.update(json!({})).await.expect_err("重入应报错");
    assert_eq!(err.kind(), &ErrorKind::WaterfallReentry);
    assert_eq!(err.message(), "waterfall 的 next() 被重复调用");
    assert_eq!(
        loads.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "第一次 next() 的重启已发生，重入错误随后抛出"
    );
}

/// 验收 5d：update waterfall——否决短路：不调 next() 时内置重启不执行。
#[tokio::test(flavor = "multi_thread")]
async fn update_waterfall_veto_blocks_restart() {
    let root = Context::new();
    let (plugin, _loads) = CountingPlugin::new("counter");
    let handle = root.plugin(plugin, json!({}));
    handle.await_ready().await.expect("应装载");
    // 记录内置行为是否被触发的标记
    let builtin_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 直接对事件总线做 waterfall 分发，验证「否决即短路」的通用语义
    let marker = builtin_ran.clone();
    let builtin: WaterfallBuiltin = Arc::new(move |_args| {
        let marker = marker.clone();
        Box::pin(async move {
            marker.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Value::Null)
        })
    });
    let listener = Arc::new(|_args: &[Value], _next: yunyou::WaterfallNext| {
        Box::pin(async { Ok(json!("vetoed")) }) as BoxFut<'static, Result<Value>>
    });
    root.on_waterfall("test/veto", listener).expect("应注册");
    let outcome = root
        .waterfall("test/veto", &[json!({"step": 1})], builtin)
        .await
        .expect("否决应正常返回");
    assert_eq!(outcome, json!("vetoed"));
    assert!(
        !builtin_ran.load(std::sync::atomic::Ordering::SeqCst),
        "不调 next() 必须短路内置行为"
    );
    // update 路径的「否决即不重启」由 update_keeps_instance_without_next 覆盖
    //（该测试在 internal/update 上注册否决监听器；本测试的事件键是 test/veto）
}

/// 验收 6：Weak 依赖者存续时服务实例存活——提供者卸载后、消费者卸载前实例可用，
/// 消费者也卸载后实例才析构（引用计数释放时机，方案 §2.1.2）。
#[tokio::test(flavor = "multi_thread")]
async fn weak_dependent_keeps_service_alive() {
    let root = Context::new();
    let log = new_log();
    let consumer = root.plugin(ConsumerPlugin::new("consumer", log.clone()), json!({}));
    let provider = root.plugin(ProviderPlugin { tag: "provider", log: log.clone() }, json!({}));
    provider.await_ready().await.expect("提供者应装载");
    wait_state(&consumer, FiberState::Active).await;
    provider.dispose().await.expect("提供者应卸载");
    wait_state(&consumer, FiberState::Pending).await;
    let entries = snapshot(&log);
    let teardown = entries.iter().position(|e| e.starts_with("teardown-call")).expect("消费者 teardown 应执行");
    let drop_at = entries.iter().position(|e| e == "drop:provider").expect("实例应在全部依赖方卸载后析构");
    assert!(
        teardown < drop_at,
        "实例析构必须晚于消费者 teardown 期间的最后调用（teardown={teardown}, drop={drop_at}）"
    );
}

/// 验收 7：setup 落定后内核不再持有 setup 传入的 Context 强句柄
///（方案 §2.1.2 评审修订的强计数断言；组件侧用 WeakContext 长期持有）。
#[tokio::test(flavor = "multi_thread")]
async fn setup_strong_context_released_after_setup() {
    trait Probe: Send + Sync {
        fn at_setup(&self) -> usize;
        fn strong_count_now(&self) -> usize;
    }
    struct ProbeService {
        at_setup: usize,
        weak: yunyou::WeakContext,
    }
    impl Probe for ProbeService {
        fn at_setup(&self) -> usize {
            self.at_setup
        }
        fn strong_count_now(&self) -> usize {
            self.weak.strong_count()
        }
    }
    struct ProbePlugin;
    impl Plugin for ProbePlugin {
        fn name(&self) -> &str {
            "probe"
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                let at_setup = ctx.downgrade().strong_count();
                let service: Arc<dyn Probe> = Arc::new(ProbeService { at_setup, weak: ctx.downgrade() });
                ctx.provide(ServiceId::of::<dyn Probe>(), service)?;
                Ok(FiberOutput)
            })
        }
    }
    let root = Context::new();
    let handle = root.plugin(ProbePlugin, json!({}));
    handle.await_ready().await.expect("应装载");
    let probe = root.get::<dyn Probe>(ServiceId::of::<dyn Probe>()).expect("服务应可见");
    // §2.1.2 强句柄纪律的精确断言：setup 落定后，ContextInner 的强引用只剩
    // fiber 树自身那一份（FiberShared.ctx）；setup future 与装载任务的临时句柄
    // 都已释放（at_setup 时点比落定后多出这两份，故回落量为 2）。
    assert_eq!(
        probe.strong_count_now(),
        1,
        "setup 落定后强计数必须回落到 1（仅剩 fiber 树自身）；\
         实测 at_setup={} 落定后={}，差值应≥2",
        probe.at_setup(),
        probe.strong_count_now()
    );
}

/// 验收补充：错误信息包含服务键与中文说明（可诊断性）。
#[tokio::test(flavor = "multi_thread")]
async fn missing_service_error_is_actionable() {
    let root = Context::new();
    let err = match root.get::<dyn Greeter>(ServiceId::named("no-such-service")) {
        Ok(_) => panic!("未注册的服务不应可取"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), &ErrorKind::ServiceNotFound);
    assert!(err.message().contains("no-such-service"), "错误信息应包含服务键：{err}");
    // 消费者插件声明缺失依赖时保持 Pending 而非报错
    let log = new_log();
    let consumer = root.plugin(ConsumerPlugin::new("consumer", log), json!({}));
    assert_eq!(consumer.state(), FiberState::Pending);
    let _ = consumer; // 留在 Pending，随测试结束丢弃
}

/// 静默 ListenerOutcome 引用（供文档完整性；分发场景见 tests/config.rs）。
#[allow(dead_code)]
fn assert_listener_outcome_shape(outcome: ListenerOutcome) -> bool {
    matches!(outcome, ListenerOutcome::Continue | ListenerOutcome::Bail(_))
}

/// 静默 CordisError 引用（供文档完整性；错误分类断言见 tests/config.rs）。
#[allow(dead_code)]
fn assert_error_kind_shape(err: &CordisError) -> &ErrorKind {
    err.kind()
}

/// 静默 FiberHandle::restart 引用（失败恢复场景见 tests/config.rs）。
#[allow(dead_code)]
async fn assert_restart_available(handle: &FiberHandle) -> Result<()> {
    handle.restart().await
}

// ───────────────────────── 审查返修补充测试（差异 3 专项 + panic 隔离） ─────────────────────────

/// 返修 P1-补1：setup 返回 Err → 本次 provide 已从服务表摘除（差异 3 语义），
/// 且提供者可正常 dispose（FAILED 不卡死引用计数）。
#[tokio::test(flavor = "multi_thread")]
async fn setup_failure_cleans_provisions_and_allows_provider_dispose() {
    struct FailingProviderPlugin;
    impl Plugin for FailingProviderPlugin {
        fn name(&self) -> &str {
            "failing-provider"
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                let greeter: Arc<dyn Greeter> = Arc::new(Greeting { tag: "doomed", log: new_log() });
                ctx.provide(ServiceId::of::<dyn Greeter>(), greeter)?;
                // 注册成功后 setup 失败：本次 provide 必须被清理
                Err(CordisError::new(ErrorKind::PluginSetup, "故意失败"))
            })
        }
    }
    let root = Context::new();
    let handle = root.plugin(FailingProviderPlugin, json!({}));
    let err = handle.await_ready().await.expect_err("setup 应失败");
    assert_eq!(err.kind(), &ErrorKind::PluginSetup);
    // 服务表已摘：strict 读取不可见
    assert!(
        root.get::<dyn Greeter>(ServiceId::of::<dyn Greeter>()).is_err(),
        "setup 失败后其 provide 必须已注销"
    );
    // 提供者可正常卸载（dependents 无残留，不卡死）
    handle.dispose().await.expect("FAILED fiber 应可正常卸载");
    handle.wait_disposed().await;
    assert_eq!(handle.state(), FiberState::Disposed);
}

/// 返修 P1-补2：装载竞态回退（快照时依赖消失）→ 消费者回 Pending 而非 FAILED，无死锁。
#[tokio::test(flavor = "multi_thread")]
async fn load_race_falls_back_to_pending() {
    let root = Context::new();
    let log = new_log();
    // 消费者先注册（Pending），提供者后注册并在其 Active 时立即 dispose——
    // 消费者的装载任务与提供者卸载竞速；无论快照成败最终必须收敛到 Pending。
    let consumer = root.plugin(ConsumerPlugin::new("consumer", log.clone()), json!({}));
    let provider = root.plugin(ProviderPlugin { tag: "provider", log: log.clone() }, json!({}));
    provider.await_ready().await.expect("提供者应装载");
    let _ = provider.dispose().await;
    wait_state(&consumer, FiberState::Pending).await;
    assert_eq!(consumer.state(), FiberState::Pending, "竞态回退终态必须是 Pending");
}

/// 返修 P1-补3：dispose 途中单个 disposer panic → 后续清理继续执行（逆序完整性）。
#[tokio::test(flavor = "multi_thread")]
async fn disposer_panic_does_not_block_rest() {
    let root = Context::new();
    let log = new_log();
    struct PanicPlugin {
        log: Log,
    }
    impl Plugin for PanicPlugin {
        fn name(&self) -> &str {
            "panicky"
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                // 注册序：a → panic 的 b → c；卸载逆序应执行 c、b(panic 被隔离)、a
                for tag in ["a", "b", "c"] {
                    let log = self.log.clone();
                    ctx.effect(format!("effect-{tag}"), move || {
                        Ok(Some(Disposer::new(move || {
                            let log = log.clone();
                            let tag = tag;
                            Box::pin(async move {
                                if tag == "b" {
                                    panic!("清理器故意爆炸");
                                }
                                push(&log, format!("dispose:{tag}"));
                            })
                        })))
                    })?;
                }
                Ok(FiberOutput)
            })
        }
    }
    let handle = root.plugin(PanicPlugin { log: log.clone() }, json!({}));
    handle.await_ready().await.expect("应装载");
    handle.dispose().await.expect("dispose 应完成（panic 被隔离）");
    handle.wait_disposed().await;
    assert_eq!(
        snapshot(&log),
        vec!["dispose:c", "dispose:a"],
        "b 的 panic 被隔离，c 与 a 仍逆序执行"
    );
}

/// 返修 P1-补4：监听器 panic 三例——emit 不炸总线 / parallel 归入聚合 / serial 停止报错。
#[tokio::test(flavor = "multi_thread")]
async fn listener_panic_is_contained() {
    let root = Context::new();
    let explosive: Listener = Arc::new(|_args: &[Value]| {
        Box::pin(async {
            panic!("监听器故意爆炸");
            #[allow(unreachable_code)]
            Err::<ListenerOutcome, CordisError>(CordisError::new(ErrorKind::Internal, "不可达"))
        }) as BoxFut<'static, Result<ListenerOutcome>>
    });

    // ① emit：panic 在任务内，总线存活，后续监听器照常注册与分发
    root.on("panic/emit", explosive.clone()).expect("应注册");
    root.emit("panic/emit", &[]);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await; // 让 panic 发生并被丢弃

    // ② parallel：panic 归入 Aggregate
    let err = root.parallel("panic/emit", &[]).await.expect_err("panic 应进聚合");
    assert_eq!(err.kind(), &ErrorKind::Aggregate);

    // ③ serial：panic 以 Err 返回，不炸调用任务
    let err = root.serial("panic/emit", &[]).await.expect_err("panic 应转为错误");
    assert!(err.message().contains("监听器执行异常"), "{err}");
}

/// task-29① 一刀收两口：update 校验失败**零突变**——坏配置被拒后 core 不得
/// 持坏配置（restart 必须仍用旧配置装载成功=雷未留）。修前行为：update 先
/// core.config=坏配置突变+发 COMMIT、后校验——失败留雷，restart 用坏配置
/// 装载再败→Failed（外部独立复核实证路径）。
struct StrictConfigPlugin {
    tag: &'static str,
    loads: Arc<std::sync::atomic::AtomicUsize>,
}

impl StrictConfigPlugin {
    fn new(tag: &'static str) -> (Self, Arc<std::sync::atomic::AtomicUsize>) {
        let loads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (Self { tag, loads: Arc::clone(&loads) }, loads)
    }
}

impl Plugin for StrictConfigPlugin {
    fn name(&self) -> &str {
        self.tag
    }

    // config_schema 带必填键+禁未知键——resolve_plugin_config 的校验闸在此生效
    fn config_schema(&self) -> Option<ConfigValue> {
        Some(json!({
            "type": "object",
            "properties": {"mode": {"type": "string"}},
            "required": ["mode"],
            "additionalProperties": false
        }))
    }

    fn setup(self: Arc<Self>, _ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(FiberOutput)
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn update_校验失败零突变_restart仍用旧配置() {
    let root = Context::new();
    let (plugin, loads) = StrictConfigPlugin::new("strict");
    let handle = root.plugin(plugin, json!({"mode": "a"}));
    handle.await_ready().await.expect("首装载应成功");
    assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);

    // 坏配置（缺必填 mode）→ 校验前置上抛 Err
    let err = handle
        .update(json!({"wrong": 1}))
        .await
        .expect_err("坏配置应被校验闸拒绝");
    let _ = err; // 错误细节归 schema 面，此处断言行为面

    // 雷未留：restart 用旧配置（mode=a）重装成功（修前=坏配置装载再败→Failed）
    handle
        .restart()
        .await
        .expect("restart 应成功——update 失败不得污染 core.config");
    assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 2, "重装恰一次");
    assert_eq!(handle.state(), FiberState::Active, "主板 Active 非 Failed");

    // 合法 update 链路不回归（前置化后正常路径照旧）
    handle
        .update(json!({"mode": "b"}))
        .await
        .expect("合法 update 应成功（waterfall 内置重启）");
    assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(handle.state(), FiberState::Active);
}

/// task-29 追加刀（云枢复核新发现）判别力测试：Loading 期 dispose + setup Err
/// ——修前 finish_load_error 无 uid 分支落 Failed（drive 被 error 挡死=无任务
/// 再驱动）→wait_disposed 永挂；修后收尾按 uid 分支落 Disposed 放行。
/// 时序：setup 挂 oneshot→dispose 先行（uid=None、busy 不 spawn unload）→
/// setup Err→wait_disposed 必须在护栏内落定+state Disposed。
struct HangingSetupPlugin {
    rx: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<Result<FiberOutput>>>>,
}

impl Plugin for HangingSetupPlugin {
    fn name(&self) -> &str {
        "hanging"
    }

    fn setup(self: Arc<Self>, _ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            // Arc 字段不可 move：Mutex<Option> take 取出后 await（Arc<Self>
            // 全部权随 setup future 结束释放，take 恰一次）
            let rx = self
                .rx
                .lock()
                .expect("测试锁")
                .take()
                .expect("setup 恰执行一次");
            rx.await.expect("测试通道应存活")
        })
    }
}

// current_thread 默认档（云枢终验修正）：multi_thread 下 load 任务与测试
// dispose 首.poll 存在调度竞争（load 先完成 finish_load_error 则修前也绿=判别力
// 概率化）；current_thread 时序确定化——dispose 同步段先落 uid=None→load 后跑
// 必见 uid None→修前必挂死红、修后必 Disposed 绿。
#[tokio::test]
async fn 装载期dispose后setup失败不挂死() {
    let root = Context::new();
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<FiberOutput>>();
    let handle = root.plugin(
        HangingSetupPlugin {
            rx: std::sync::Mutex::new(Some(rx)),
        },
        json!({}),
    );
    // dispose 先行：setup 挂住（Loading 态）→uid=None、busy=true 不 spawn unload
    let disposer = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        handle.dispose().await.expect("dispose 发起应成功返回");
    });
    // setup 失败到达：finish_load_error 在 disposing 态收尾
    tx.send(Err(yunyou::CordisError::new(
        yunyou::ErrorKind::Internal,
        "setup 失败于 disposing 期",
    )))
    .expect("setup 通道应存活");
    // 修后：wait_disposed 在 dispose 内落定（护栏 5s 抓挂死）
    disposer.await.expect("dispose 不得永挂（挂死=Failed 僵尸回归）");
    assert_eq!(
        handle.state(),
        FiberState::Disposed,
        "Loading 期 dispose+setup 失败应落 Disposed 而非 Failed 僵尸"
    );
}

// 内-P1-3 回归锁（云岚报告-内核生命周期-20261117）：await_ready 在 Pending 期
// 必须挂起等待装载完成，不得当稳定态立即返回 Ok（假就绪）。
// current_thread 档时序确定化：消费者先注册（依赖缺 → Pending），旧实现此断言
// 立即拿到 Ok=红；注册提供者后依赖就绪自动装载，await_ready 解除挂起返回 Ok=绿。
#[tokio::test]
async fn await_ready在pending期挂起依赖就绪后返回() {
    let log = new_log();
    let root = Context::new();
    // 依赖未提供：消费者 fiber 注册后停留 Pending
    let consumer = root.plugin(ConsumerPlugin::new("consumer", log.clone()), json!({}));
    assert_eq!(consumer.state(), FiberState::Pending, "依赖缺失应停留 Pending");
    // Pending 期 await_ready 不得落定（50ms 护栏抓「立即 Ok」假就绪回归）
    let early = tokio::time::timeout(std::time::Duration::from_millis(50), consumer.await_ready()).await;
    assert!(
        early.is_err(),
        "Pending 期 await_ready 不得立即返回（内-P1-3 假就绪回归）"
    );
    // 提供者就位 → 依赖就绪自动装载 → 挂起的 await_ready 解除并返回 Ok
    root.plugin(ProviderPlugin { tag: "provider", log }, json!({}));
    consumer.await_ready().await.expect("依赖就绪后 await_ready 应返回 Ok");
    assert!(consumer.is_active(), "装载完成后 fiber 应 Active");
}
