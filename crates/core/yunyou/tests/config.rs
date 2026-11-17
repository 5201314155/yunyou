//! 配置/作用域/事件分发/错误恢复回归（与 tests/lifecycle.rs 互补）。
//!
//! 覆盖：serde+schemars 校验失败 → FAILED（ValidationError 聚合）、
//! FAILED 只能显式 restart 恢复、父卸载级联子、isolate 作用域隔离、
//! intercept 祖先在前的合并顺序、get/duplicate 错误路径、五种事件分发的行为面。

use yunyou::{
    ConfigValue, Context, ErrorKind, FiberOutput, FiberState, ListenerOutcome, Plugin, Result,
    ServiceId, WaterfallBuiltin,
};
use yunyou_util::BoxFut;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Log = Arc<Mutex<Vec<String>>>;

fn push(log: &Log, entry: impl Into<String>) {
    log.lock().unwrap().push(entry.into());
}

fn snapshot(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// `expect_err` 的等价物：服务句柄（Arc<dyn Trait>）无 Debug，用 match 拆错。
fn expect_service_err<T>(outcome: yunyou::Result<T>, message: &str) -> yunyou::CordisError {
    match outcome {
        Ok(_) => panic!("{message}"),
        Err(err) => err,
    }
}

/// 严格配置（schemars 派生 → 转 Value，验证 serde+schemars 全链路，方案 §2.2）。
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StepConfig {
    #[serde(default = "default_step")]
    step: u32,
    name: String,
}

fn default_step() -> u32 {
    1
}

/// 严格配置插件：setup 内用 serde 反序列化解析配置。
struct StrictPlugin {
    loads: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Plugin for StrictPlugin {
    fn name(&self) -> &str {
        "strict"
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(serde_json::to_value(schemars::schema_for!(StepConfig)).expect("schema 应可序列化"))
    }

    fn setup(self: Arc<Self>, _ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: StepConfig = serde_json::from_value(config)
                .map_err(|err| yunyou::CordisError::new(ErrorKind::ConfigValidation, err.to_string()))?;
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(format!("{}:{}", cfg.step, cfg.name));
            Ok(FiberOutput)
        })
    }
}

/// 校验失败 → 装载中止 → FAILED，错误为聚合的 ValidationError。
#[tokio::test(flavor = "multi_thread")]
async fn config_validation_fails_load() {
    let root = Context::new();
    let plugin = StrictPlugin { loads: Arc::new(AtomicUsize::new(0)), seen: Arc::new(Mutex::new(Vec::new())) };
    let handle = root.plugin(plugin, json!({"step": "NaN", "name": ""}));
    let err = handle.await_ready().await.expect_err("schema 校验失败应中止装载");
    assert_eq!(err.kind(), &ErrorKind::ConfigValidation);
    assert!(err.message().starts_with("invalid config:"), "应对齐 TS 聚合格式：{err}");
    assert_eq!(handle.state(), FiberState::Failed);
}

/// FAILED fiber 只能经配置更新/重启恢复（对齐 fiber.ts:634）；
/// update 的校验失败只拒绝候选、不动运行中的实例（对齐 fiber.ts:676-679）。
#[tokio::test(flavor = "multi_thread")]
async fn failed_fiber_recovers_via_restart() {
    let root = Context::new();
    let loads = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    // ① 非法配置启动 → FAILED（setup 未执行）
    let handle = root.plugin(
        StrictPlugin { loads: loads.clone(), seen: seen.clone() },
        json!({"step": "bad", "name": "x"}),
    );
    let err = handle.await_ready().await.expect_err("非法配置应装载失败");
    assert_eq!(err.kind(), &ErrorKind::ConfigValidation);
    assert_eq!(handle.state(), FiberState::Failed);
    assert_eq!(loads.load(Ordering::SeqCst), 0, "装载被中止，setup 不应执行");
    // ② update 携带合法配置：未装载的 fiber 直接走重启装载（对齐 fiber.ts:811-813）
    handle.update(json!({"step": 7, "name": "demo"})).await.expect("恢复后应装载成功");
    assert_eq!(handle.state(), FiberState::Active);
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    // ③ 运行中提交非法候选：被拒绝且实例保持原状
    let err = handle.update(json!({"step": "bad", "name": "x"})).await.expect_err("非法候选应被拒绝");
    assert_eq!(err.kind(), &ErrorKind::ConfigValidation);
    assert_eq!(handle.state(), FiberState::Active, "校验失败不得动运行中的实例");
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    // ④ 合法更新走内置重启，setup 重跑
    handle.update(json!({"step": 9, "name": "demo"})).await.expect("应重启成功");
    assert_eq!(loads.load(Ordering::SeqCst), 2);
    assert_eq!(*seen.lock().unwrap(), vec!["7:demo", "9:demo"], "setup 应收到解析后的配置");
}

/// 父卸载级联子卸载（子 fiber 的 dispose 挂在父 fiber 的 effect 上）。
///
/// 看门狗（S7 评审 P2-2）：dispose 级联等待链静态排查无竞态（notify_one 存储
/// 型 permit + watch 版本号语义 + busy 接力闭合），但曾实录 30+ 分钟偶发挂死
/// （两树复跑均秒过、不可复现）——30s 超时 fail-fast 防门禁被空等堵死；再挂
/// 请以 RUST_BACKTRACE=1 复跑抓现场。
#[tokio::test(flavor = "multi_thread")]
async fn parent_dispose_cascades_children() {
    tokio::time::timeout(
        Duration::from_secs(30),
        parent_dispose_cascades_children_body(),
    )
    .await
    .expect("级联卸载应在 30s 内完成（超时=dispose 等待链挂死，勿让门禁空等）");
}

async fn parent_dispose_cascades_children_body() {
    struct ChildPlugin {
        log: Log,
    }
    impl Plugin for ChildPlugin {
        fn name(&self) -> &str {
            "child"
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                let log = self.log.clone();
                ctx.effect("child-effect", move || {
                    Ok(Some(yunyou::Disposer::new(move || {
                        let log = log.clone();
                        Box::pin(async move { push(&log, "dispose:child") })
                    })))
                })?;
                Ok(FiberOutput)
            })
        }
    }
    struct ParentPlugin {
        log: Log,
    }
    impl Plugin for ParentPlugin {
        fn name(&self) -> &str {
            "parent"
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                ctx.plugin(ChildPlugin { log: self.log.clone() }, json!({}));
                let log = self.log.clone();
                ctx.effect("parent-effect", move || {
                    Ok(Some(yunyou::Disposer::new(move || {
                        let log = log.clone();
                        Box::pin(async move { push(&log, "dispose:parent") })
                    })))
                })?;
                Ok(FiberOutput)
            })
        }
    }
    let root = Context::new();
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let parent = root.plugin(ParentPlugin { log: log.clone() }, json!({}));
    parent.await_ready().await.expect("应装载");
    root.dispose().await.expect("root 应处置成功");
    assert_eq!(
        snapshot(&log),
        vec!["dispose:parent", "dispose:child"],
        "级联卸载必须发生：子 fiber 的处置挂在父的 effect 上，逆序执行意味着\
         父自有清理先于子 fiber 处置；「先消费者后提供者」的顺序由依赖广播路径\
         保证（见 tests/lifecycle.rs 的 unload_order 场景）"
    );
}

/// isolate 作用域隔离：同一服务键在不同作用域各有实现，互不可见。
#[tokio::test(flavor = "multi_thread")]
async fn isolate_scopes_separate_implementations() {
    struct NamedChannel(Arc<str>);
    impl Channel for NamedChannel {
        fn name(&self) -> String {
            self.0.to_string()
        }
    }
    struct ChannelPlugin(Arc<str>);
    impl Plugin for ChannelPlugin {
        fn name(&self) -> &str {
            &self.0
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                let channel: Arc<dyn Channel> = Arc::new(NamedChannel(self.0.clone()));
                ctx.provide(ServiceId::of::<dyn Channel>(), channel)?;
                Ok(FiberOutput)
            })
        }
    }
    let root = Context::new();
    let key = ServiceId::of::<dyn Channel>();
    let scope_a = root.allocate_scope();
    let scope_b = root.allocate_scope();
    let ctx_a = root.isolate(key.clone(), scope_a);
    let ctx_b = root.isolate(key.clone(), scope_b);
    let fiber_a = ctx_a.plugin(ChannelPlugin(Arc::from("channel-a")), json!({}));
    let fiber_b = ctx_b.plugin(ChannelPlugin(Arc::from("channel-b")), json!({}));
    fiber_a.await_ready().await.expect("a 应装载");
    fiber_b.await_ready().await.expect("b 应装载");
    let a = ctx_a.get::<dyn Channel>(key.clone()).expect("a 作用域应可见");
    let b = ctx_b.get::<dyn Channel>(key).expect("b 作用域应可见");
    assert_eq!(a.name(), "channel-a");
    assert_eq!(b.name(), "channel-b");
    // root 作用域看不到任何实现（isolated 不上溢）
    let err = expect_service_err(
        root.get::<dyn Channel>(ServiceId::of::<dyn Channel>()),
        "root 作用域不应可见",
    );
    assert_eq!(err.kind(), &ErrorKind::ServiceNotFound);
}

/// intercept 合并：祖先段在前、自身在后（后者覆盖前者，方案 §2.2）。
#[tokio::test(flavor = "multi_thread")]
async fn intercept_merges_ancestors_first() {
    trait Reader: Send + Sync {
        fn config(&self) -> Value;
    }
    struct ConfigReader(Value);
    impl Reader for ConfigReader {
        fn config(&self) -> Value {
            self.0.clone()
        }
    }
    struct ReaderPlugin;
    impl Plugin for ReaderPlugin {
        fn name(&self) -> &str {
            "reader"
        }
        fn intercepts(&self) -> Vec<ServiceId> {
            vec![ServiceId::of::<dyn Reader>()]
        }
        fn setup(self: Arc<Self>, ctx: Context, base: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                // 配置解析已在内核完成：base + intercept 链合并（祖先在前）
                let reader: Arc<dyn Reader> = Arc::new(ConfigReader(base));
                ctx.provide(ServiceId::of::<dyn Reader>(), reader)?;
                Ok(FiberOutput)
            })
        }
    }
    let root = Context::new();
    let key = ServiceId::of::<dyn Reader>();
    let child = root.intercept(key.clone(), json!({"source": "root", "level": 1}));
    let grand = child.intercept(key.clone(), json!({"source": "child", "extra": true}));
    let handle = grand.plugin(ReaderPlugin, json!({"base": "plugin"}));
    handle.await_ready().await.expect("应装载");
    let reader = grand.get::<dyn Reader>(key).expect("应可见");
    assert_eq!(
        reader.config(),
        json!({"base": "plugin", "source": "child", "level": 1, "extra": true}),
        "合并顺序：插件 base 最先，祖先 intercept 在前，自身在后覆盖"
    );
}

/// get 错误路径：未注册 → ServiceNotFound；类型不符 → ServiceTypeMismatch。
#[tokio::test(flavor = "multi_thread")]
async fn get_error_paths() {
    let root = Context::new();
    let missing = root.get::<dyn Channel>(ServiceId::named("ghost"));
    assert_eq!(expect_service_err(missing, "应报未注册").kind(), &ErrorKind::ServiceNotFound);
    // 注册 i32 具体类型，用 trait object 类型取 → 类型不匹配
    let _ = root.provide(ServiceId::named("num"), Arc::new(42_i32));
    let mismatch = root.get::<dyn Channel>(ServiceId::named("num"));
    assert_eq!(
        expect_service_err(mismatch, "应报类型不符").kind(),
        &ErrorKind::ServiceTypeMismatch
    );
}

struct DupChannel;

impl Channel for DupChannel {
    fn name(&self) -> String {
        "dup".into()
    }
}

/// 契约服务（多测试共用；object-safe：无泛型方法、无 Self 返回值，方案 §2.1.3）。
trait Channel: Send + Sync {
    fn name(&self) -> String;
}

/// 同一作用域重复注册同名服务被拒绝（对齐 reflect.ts 的 registered 错误）。
#[tokio::test(flavor = "multi_thread")]
async fn duplicate_provide_rejected() {
    struct DupPlugin {
        tag: &'static str,
    }
    impl Plugin for DupPlugin {
        fn name(&self) -> &str {
            self.tag
        }
        fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                let channel: Arc<dyn Channel> = Arc::new(DupChannel);
                ctx.provide(ServiceId::of::<dyn Channel>(), channel)?;
                Ok(FiberOutput)
            })
        }
    }
    let root = Context::new();
    let first = root.plugin(DupPlugin { tag: "first" }, json!({}));
    first.await_ready().await.expect("首次注册应成功");
    let second = root.plugin(DupPlugin { tag: "second" }, json!({}));
    let err = second.await_ready().await.expect_err("重复注册应失败");
    assert_eq!(err.kind(), &ErrorKind::ServiceDuplicate);
    assert!(err.message().contains("first"), "错误应指出已有提供者：{err}");
}

/// 事件分发行为面：emit 忽略结果、parallel 聚合错误、serial bail 即停、
/// waterfall 正常链 / 否决 / 重入（方案风险 4 的专项三例在事件层直测）。
#[tokio::test(flavor = "multi_thread")]
async fn event_dispatch_modes() {
    let root = Context::new();
    let log: Log = Arc::new(Mutex::new(Vec::new()));

    // emit：异步分发不等结果
    let log1 = log.clone();
    root.on("evt/emit", Arc::new(move |_args: &[Value]| {
        let log = log1.clone();
        Box::pin(async move {
            push(&log, "emit-ran");
            Ok(ListenerOutcome::Continue)
        }) as BoxFut<'static, Result<ListenerOutcome>>
    }))
    .expect("应注册");
    root.emit("evt/emit", &[json!(1)]);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(snapshot(&log), vec!["emit-ran"]);

    // parallel：单个监听器失败聚合为 Aggregate
    root.on(
        "evt/parallel",
        Arc::new(|_args: &[Value]| {
            Box::pin(async { Err(yunyou::CordisError::new(ErrorKind::PluginSetup, "监听器爆炸")) })
                as BoxFut<'static, Result<ListenerOutcome>>
        }),
    )
    .expect("应注册");
    let err = root.parallel("evt/parallel", &[]).await.expect_err("应聚合错误");
    assert_eq!(err.kind(), &ErrorKind::Aggregate);
    assert_eq!(err.causes().len(), 1);

    // serial：bail 即停
    for (name, outcome) in [
        ("s1", ListenerOutcome::Continue),
        ("s2", ListenerOutcome::Bail(json!("stop-here"))),
    ] {
        let log2 = log.clone();
        root.on("evt/serial", Arc::new(move |_args: &[Value]| {
            let log = log2.clone();
            let outcome = outcome.clone();
            Box::pin(async move {
                push(&log, name);
                Ok(outcome)
            }) as BoxFut<'static, Result<ListenerOutcome>>
        }))
        .expect("应注册");
    }
    let bailed = root.serial("evt/serial", &[]).await.expect("应返回 bail 值");
    assert_eq!(bailed, Some(json!("stop-here")));
    assert_eq!(snapshot(&log), vec!["emit-ran", "s1", "s2"], "bail 后不再继续（仅注册两个监听器）");

    // waterfall：正常链（l1 → l2 → builtin）与重入
    let log3 = log.clone();
    root.on_waterfall(
        "evt/wf",
        Arc::new(move |_args: &[Value], next: yunyou::WaterfallNext| {
            let log = log3.clone();
            Box::pin(async move {
                push(&log, "wf-l1");
                next.call().await
            }) as BoxFut<'static, Result<Value>>
        }),
    )
    .expect("应注册");
    let log4 = log.clone();
    root.on_waterfall(
        "evt/wf",
        Arc::new(move |_args: &[Value], next: yunyou::WaterfallNext| {
            let log = log4.clone();
            Box::pin(async move {
                push(&log, "wf-l2");
                next.call().await
            }) as BoxFut<'static, Result<Value>>
        }),
    )
    .expect("应注册");
    let builtin_ran = Arc::new(AtomicBool::new(false));
    let marker = builtin_ran.clone();
    let builtin: WaterfallBuiltin = Arc::new(move |_args| {
        let marker = marker.clone();
        Box::pin(async move {
            marker.store(true, Ordering::SeqCst);
            Ok(json!("builtin"))
        })
    });
    let value = root.waterfall("evt/wf", &[json!({})], builtin).await.expect("链条应走完");
    assert_eq!(value, json!("builtin"));
    assert!(builtin_ran.load(Ordering::SeqCst));
    assert_eq!(snapshot(&log), vec!["emit-ran", "s1", "s2", "wf-l1", "wf-l2"]);

    // waterfall 重入：直接在事件层复现 events.ts:254
    root.on_waterfall(
        "evt/wf-reentry",
        Arc::new(|_args: &[Value], next: yunyou::WaterfallNext| {
            Box::pin(async move {
                let _ = next.call().await?;
                next.call().await
            }) as BoxFut<'static, Result<Value>>
        }),
    )
    .expect("应注册");
    let noop: WaterfallBuiltin = Arc::new(|_args| Box::pin(async { Ok(Value::Null) }));
    let err = root
        .waterfall("evt/wf-reentry", &[], noop)
        .await
        .expect_err("重入应报错");
    assert_eq!(err.kind(), &ErrorKind::WaterfallReentry);
}

