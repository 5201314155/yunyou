//! 装载竞态回归锁（任务41 P1·内核「已准入保护」修复配套）。
//!
//! 与 lifecycle.rs 的分工：那边是生命周期回归全集（已 597 行，按「新逻辑新文件」
//! 纪律不再加行）；本文件专锁装载竞态窗口——消费者被准入后、提供者进入卸载窗口
//! 时，消费者 setup 内的依赖取用必须照常成功、fiber 终态收敛 Pending（修复前为
//! FAILED，机制与实证见 /home/wu/.cache/yunyou-yun-luo/task41-复现实验记录.md）。

use std::sync::Arc;
use std::time::Duration;
use serde_json::json;
use yunyou::{
    ConfigValue, Context, FiberHandle, FiberOutput, FiberState, Plugin, Result, ServiceId,
};
use yunyou_util::BoxFut;

/// 契约服务（与 lifecycle.rs 同构的最小测试服务；测试文件不共享代码，各自自洽）。
trait Greeter: Send + Sync {
    fn greet(&self) -> String;
}

struct Greeting {
    tag: &'static str,
}

impl Greeter for Greeting {
    fn greet(&self) -> String {
        format!("hello from {}", self.tag)
    }
}

/// 提供者插件：setup 里 provide `Greeter`（与 lifecycle.rs ProviderPlugin 同构）。
struct ProviderPlugin {
    tag: &'static str,
}

impl Plugin for ProviderPlugin {
    fn name(&self) -> &str {
        self.tag
    }

    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let greeter: Arc<dyn Greeter> = Arc::new(Greeting { tag: self.tag });
            ctx.provide(ServiceId::of::<dyn Greeter>(), greeter)?;
            Ok(FiberOutput)
        })
    }
}

/// 轮询等待 fiber 到达目标状态（装卸任务异步推进；5ms×400≈2s 上限）。
async fn wait_state(handle: &FiberHandle, want: FiberState) {
    for _ in 0..400 {
        if handle.state() == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等待状态 {want:?} 超时，当前 {:?}", handle.state());
}

/// 确定性回归锁：把「快照成功 → 提供者卸载窗口 → 消费者 get」的竞态交错用
/// 握手信号钉死（无 sleep 无时序赌注）：
/// ①消费者 setup 第一行落 started 信号——此刻 run_load 的快照+dependent 登记
///   必然已完成（内核序：setup 晚于快照），不靠观测推断；
/// ②测试等提供者真正进入 Unloading（其卸载任务等消费者释放，状态确定停留）；
/// ③才放行消费者 get——修复前此处 get_strict 撞 Unloading → ServiceNotFound →
///   setup Err → FAILED；修复后已准入保护照常取用 → setup 完成 → epoch 已被
///   重算为 Inactive → 消费者卸载并释放 dependent → 提供者续卸 → 终态 Pending。
#[tokio::test(flavor = "multi_thread")]
async fn load_in_flight_consumer_survives_provider_unload() {
    use tokio::sync::Notify;
    let root = Context::new();
    let started = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    struct HandshakeConsumer {
        started: Arc<Notify>,
        proceed: Arc<Notify>,
    }
    impl Plugin for HandshakeConsumer {
        fn name(&self) -> &str {
            "handshake-consumer"
        }

        fn inject(&self) -> Vec<ServiceId> {
            vec![ServiceId::of::<dyn Greeter>()]
        }

        fn setup(
            self: Arc<Self>,
            ctx: Context,
            _config: ConfigValue,
        ) -> BoxFut<'static, Result<FiberOutput>> {
            Box::pin(async move {
                self.started.notify_one();
                self.proceed.notified().await;
                let greeter =
                    ctx.get::<dyn Greeter>(ServiceId::of::<dyn Greeter>())?;
                assert!(greeter.greet().contains("provider"), "准入条目应照常可用");
                Ok(FiberOutput)
            })
        }
    }
    let consumer = root.plugin(
        HandshakeConsumer { started: started.clone(), proceed: proceed.clone() },
        json!({}),
    );
    let provider = root.plugin(ProviderPlugin { tag: "provider" }, json!({}));
    provider.await_ready().await.expect("提供者应装载");
    started.notified().await; // 消费者已快照（dependent 已登记）并停在 setup
    let provider_state = provider.clone(); // 状态观测句柄（dispose 需 move 进任务）
    let disposer = tokio::spawn(async move { provider.dispose().await });
    wait_state(&provider_state, FiberState::Unloading).await; // 卸载窗口确定打开
    // 同窗对照（保护精度两侧）：非准入 fiber（root，无 committed 快照）同刻取用
    // 必须仍拒——已准入保护不得放宽成「卸载中永远可见」。
    let rejection = match root.get::<dyn Greeter>(ServiceId::of::<dyn Greeter>()) {
        Ok(_) => panic!("非准入 fiber 的 get 在卸载窗口必须仍拒"),
        Err(err) => err,
    };
    assert!(
        rejection.message().contains("未注册或提供者未就绪"),
        "拒因应为 strict 不可见（ServiceNotFound 语义）: {rejection}"
    );
    proceed.notify_one(); // 放行：消费者 get 恰在窗口内执行
    disposer.await.expect("dispose 任务不应 panic").expect("dispose 应成功");
    wait_state(&consumer, FiberState::Pending).await;
    assert_eq!(
        consumer.state(),
        FiberState::Pending,
        "终态必须收敛 Pending（修复前为 FAILED）"
    );
}

// ── task-28：Miss 路径泄漏 dependents 回归锁（两段式快照+回滚分支） ──────
//
// 机制（双源印证+云枢 scratch 确定性场景）：泄漏需要「双依赖齐→C 进入
// Loading→快照前 B vanish」的形状——B 自始缺席则 C 首次 drive 即 Pending
// （从未进 Loading）=零登记路径无泄漏可言（第一版测试形状错误判别力为零，
// 云枢打回实录）。确定性序=current_thread 档钉任务入队序：A、B 双上线→
// C 注册（drive 入队未跑）→B.dispose 同步段先落 Unloading→C 的 load 任务
// 在 dispose 首个 await 点被调度→快照时刻 A 在 B 缺→修前 partial retain
// 泄漏（dispose A 挂死）/修后两段式纯丢弃。
//
// 断言反转自 scratch（修前断言挂死=red）为修后形态：dispose A 限时完成+
// 收敛链保留。判别力自证：修前内核（stash load.rs/service.rs）跑本测试必红，
// 红输出在交审单（云枢硬标「回归锁与修法同批」的锁=确定性锁）。

/// 双服务契约（本文件自洽，不共享）。
trait Counter: Send + Sync {
    fn value(&self) -> u32;
}

struct Counting {
    v: u32,
}

impl Counter for Counting {
    fn value(&self) -> u32 {
        self.v
    }
}

struct CountingPlugin {
    tag: &'static str,
    v: u32,
}

impl Plugin for CountingPlugin {
    fn name(&self) -> &str {
        self.tag
    }
    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let svc: Arc<dyn Counter> = Arc::new(Counting { v: self.v });
            ctx.provide(ServiceId::of::<dyn Counter>(), svc)?;
            Ok(FiberOutput)
        })
    }
}

struct DoubleDepsPlugin {
    tag: &'static str,
}

impl Plugin for DoubleDepsPlugin {
    fn name(&self) -> &str {
        self.tag
    }
    fn inject(&self) -> Vec<ServiceId> {
        // 注入 [Counter(=A), Answer(=B)]——快照前 B vanish 的泄漏形态本体
        vec![ServiceId::of::<dyn Counter>(), ServiceId::of::<dyn Answer>()]
    }
    fn setup(self: Arc<Self>, _ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move { Ok(FiberOutput) })
    }
}

trait Answer: Send + Sync {
    fn answer(&self) -> u32;
}

struct AnswerImpl;
impl Answer for AnswerImpl {
    fn answer(&self) -> u32 {
        42
    }
}

struct AnswerPlugin {
    tag: &'static str,
}

impl Plugin for AnswerPlugin {
    fn name(&self) -> &str {
        self.tag
    }
    fn setup(self: Arc<Self>, ctx: Context, _config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let svc: Arc<dyn Answer> = Arc::new(AnswerImpl);
            ctx.provide(ServiceId::of::<dyn Answer>(), svc)?;
            Ok(FiberOutput)
        })
    }
}

/// Miss 泄漏回归锁（current_thread 确定性序）：C 入 Loading 后快照前 B vanish
/// →修后 dispose A 限时完成（修前挂死）+补齐后 C 收敛 Active。
#[tokio::test(flavor = "current_thread")]
async fn miss泄漏后提供者卸载不挂死() {
    let root = Context::new();
    // A、B 双上线
    let a = root.plugin(CountingPlugin { tag: "提供者甲", v: 1 }, json!({}));
    wait_state(&a, FiberState::Active).await;
    let b = root.plugin(AnswerPlugin { tag: "提供者乙" }, json!({}));
    wait_state(&b, FiberState::Active).await;

    // C 注册：drive 入队（current_thread 下未跑）——load 任务在队
    let c = root.plugin(DoubleDepsPlugin { tag: "消费者丙" }, json!({}));

    // B vanish：dispose 同步段先落 Unloading（epoch=Inactive+notify）；
    // 首个 await 点让出→C 的 load 任务被调度→快照时刻 A 在 B 缺
    b.dispose().await.expect("dispose 乙应完成");
    wait_state(&c, FiberState::Pending).await;

    // 泄漏断言本体：dispose A 限时完成（修前 partial retain 残留=挂死）
    let dispose = a.dispose();
    let _ = tokio::time::timeout(Duration::from_secs(5), dispose)
        .await
        .expect("dispose 提供者甲必须完成——超时=Miss 泄漏 dependents（task-28 根因回归）");

    // 收敛路径：补 A 二代+B 二代→C epoch 重算装载→Active
    let a2 = root.plugin(CountingPlugin { tag: "提供者甲二", v: 2 }, json!({}));
    let b2 = root.plugin(AnswerPlugin { tag: "提供者乙二" }, json!({}));
    wait_state(&a2, FiberState::Active).await;
    wait_state(&b2, FiberState::Active).await;
    wait_state(&c, FiberState::Active).await;
    let counter = root
        .get::<dyn Counter>(ServiceId::of::<dyn Counter>())
        .expect("A 二代服务应可见");
    assert_eq!(counter.value(), 2);
    let answer = root
        .get::<dyn Answer>(ServiceId::of::<dyn Answer>())
        .expect("B 二代服务应可见");
    assert_eq!(answer.answer(), 42);
    let _ = a2.dispose().await;
    let _ = b2.dispose().await;
    let _ = c.dispose().await;
}
