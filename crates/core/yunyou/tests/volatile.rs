//! S6 volatile 底座 + 契约⑧事件面最小集（插件化改造方案 §3-⑧ / §6.1 S6 行）。
//!
//! 验收口径（§6.1 S6 行）：volatile 热更不重挂载 + 事件只发所属 fiber。
//! 覆盖：①快照不可变（swap 后旧 snapshot 不变）②update 后 get 立见新值
//! ③事件只发所属 fiber（跨 fiber listener 不收）④fiber 卸载后归属监听器注销
//! ⑤与既有事件总线（五分发）共存不回归。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use yunyou::events::keys;
use yunyou::{
    ConfigValue, Context, FiberOutput, Listener, ListenerOutcome, Plugin, Result, Volatile,
};
use yunyou_util::BoxFut;

/// 测试日志（owned 事件 / legacy 事件共用）。
type Log = Arc<Mutex<Vec<String>>>;

fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn snapshot(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// 轮询等待日志攒够 n 条（owned/legacy 分发都是 fire-and-forget spawn）。
async fn wait_entries(log: &Log, want: usize) -> Vec<String> {
    for _ in 0..400 {
        let entries = snapshot(log);
        if entries.len() >= want {
            return entries;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等待事件超时，当前 {:?}", snapshot(log));
}

/// 测试插件：setup 里造一个 volatile 槽（`Arc` 交测试持有，模拟服务把 volatile
/// 导出给调用方）+ 注册「只收所属 fiber」的 volatile 监听器。
struct VolatilePlugin {
    name: &'static str,
    log: Log,
    slots: Arc<Mutex<Vec<Arc<Volatile<serde_json::Value>>>>>,
}

impl Plugin for VolatilePlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn setup(
        self: Arc<Self>,
        ctx: Context,
        _config: ConfigValue,
    ) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            self.slots.lock().unwrap().push(Arc::new(Volatile::labeled(
                &ctx,
                json!({"v": 0}),
                Some("cfg.v"),
            )));
            let log = self.log.clone();
            let listener: Listener = Arc::new(move |args: &[Value]| {
                let log = log.clone();
                let tag = format!(
                    "{}|{}",
                    args[1].as_str().unwrap_or("<无名>"),
                    args[2].as_str().unwrap_or("<无标签>")
                );
                Box::pin(async move {
                    log.lock().unwrap().push(tag);
                    Ok(ListenerOutcome::Continue)
                })
            });
            ctx.on_owned(keys::VOLATILE_UPDATE, listener)?;
            Ok(FiberOutput)
        })
    }
}

/// 快照语义：swap 后旧 snapshot 不变 + update 后 get 立见新值。
#[tokio::test]
async fn volatile_快照不可变且更新后立见新值() {
    let ctx = Context::new();
    let slot = Volatile::new(&ctx, json!({"a": 1}));
    let old = slot.get();

    let new = slot.update(json!({"a": 2}));

    assert_eq!(*old, json!({"a": 1}), "swap 后旧快照必须保持有效（不得被改写）");
    assert_eq!(*new, json!({"a": 2}), "update 返回的新快照即新值");
    assert_eq!(*slot.get(), json!({"a": 2}), "update 后 get 立见新值");
}

/// 事件只发所属 fiber：跨 fiber 的监听器连 spawn 都不发生；热更不重挂载。
#[tokio::test]
async fn volatile_事件只发所属_fiber_跨fiber监听器不收() {
    let ctx = Context::new();
    let log = new_log();
    let slots = Arc::new(Mutex::new(Vec::new()));
    let plug_a = ctx.plugin(
        VolatilePlugin { name: "plug-a", log: log.clone(), slots: slots.clone() },
        json!(null),
    );
    let plug_b = ctx.plugin(
        VolatilePlugin { name: "plug-b", log: log.clone(), slots: slots.clone() },
        json!(null),
    );
    plug_a.await_ready().await.expect("plug-a 应装载完成");
    plug_b.await_ready().await.expect("plug-b 应装载完成");

    let (slot_a, slot_b) = {
        let slots = slots.lock().unwrap();
        (slots[0].clone(), slots[1].clone())
    };

    slot_a.update(json!({"v": 1}));
    let entries = wait_entries(&log, 1).await;
    assert_eq!(
        entries,
        vec!["plug-a|cfg.v".to_string()],
        "只许 plug-a 自己的监听器收到（plug-b 的监听器跨 fiber 不收）"
    );

    slot_b.update(json!({"v": 2}));
    let entries = wait_entries(&log, 2).await;
    assert_eq!(
        entries.last().unwrap(),
        "plug-b|cfg.v",
        "plug-b 的事件同样只发给 plug-b 自己的监听器"
    );

    // §6.1 S6 验收口径：热更不重挂载（fiber 仍 Active，注册未失效）
    assert!(
        plug_a.is_active() && plug_b.is_active(),
        "volatile 热更不得触发重挂载"
    );
    assert_eq!(slot_a.owner().fiber_name, "plug-a");
    assert_eq!(slot_b.owner().fiber_name, "plug-b");
}

/// 与既有五分发共存不回归：legacy 表与归属表按事件键互不串台。
#[tokio::test]
async fn owned_事件面与既有五分发共存不回归() {
    let ctx = Context::new();
    let legacy = new_log();
    let owned = new_log();

    // 既有 legacy（无归属）五分发照旧：emit 到两个 legacy 监听器
    for tag in ["legacy-1", "legacy-2"] {
        let sink = legacy.clone();
        let listener: Listener = Arc::new(move |_args: &[Value]| {
            let sink = sink.clone();
            Box::pin(async move {
                sink.lock().unwrap().push(tag.to_string());
                Ok(ListenerOutcome::Continue)
            })
        });
        ctx.on("probe/legacy", listener)
            .expect("root 上应能注册 legacy 监听器");
    }
    // 在 volatile 键上再挂一个 legacy 监听器：验证串台防线（不得收到 owned 事件）
    let sink = legacy.clone();
    let listener: Listener = Arc::new(move |_args: &[Value]| {
        let sink = sink.clone();
        Box::pin(async move {
            sink.lock().unwrap().push("legacy-on-volatile".to_string());
            Ok(ListenerOutcome::Continue)
        })
    });
    ctx.on(keys::VOLATILE_UPDATE, listener)
        .expect("root 上应能在 volatile 键挂 legacy 监听器");

    let sink = owned.clone();
    let listener: Listener = Arc::new(move |args: &[Value]| {
        let sink = sink.clone();
        let tag = format!(
            "{}|{}",
            args[1].as_str().unwrap_or("<无名>"),
            args[2].as_str().unwrap_or("<无标签>")
        );
        Box::pin(async move {
            sink.lock().unwrap().push(tag);
            Ok(ListenerOutcome::Continue)
        })
    });
    ctx.on_owned(keys::VOLATILE_UPDATE, listener)
        .expect("root 上应能注册 owned 监听器");

    // 五分发照旧（emit + parallel 两条路径都在新代码在场时跑一遍）
    ctx.emit("probe/legacy", &[json!(1)]);
    let entries = wait_entries(&legacy, 2).await;
    assert_eq!(entries, vec!["legacy-1".to_string(), "legacy-2".to_string()]);

    // 另起一个键验证 parallel 分发扬旧（等全部、错误聚合）
    let sink = legacy.clone();
    let parallel_listener: Listener = Arc::new(move |_args: &[Value]| {
        let sink = sink.clone();
        Box::pin(async move {
            sink.lock().unwrap().push("parallel".to_string());
            Ok(ListenerOutcome::Continue)
        })
    });
    ctx.on("probe/parallel", parallel_listener)
        .expect("root 上应能注册 parallel 键监听器");
    ctx.parallel("probe/parallel", &[json!(2)])
        .await
        .expect("既有 parallel 分发扬旧返回 Ok");
    assert_eq!(
        snapshot(&legacy),
        vec![
            "legacy-1".to_string(),
            "legacy-2".to_string(),
            "parallel".to_string()
        ],
        "parallel 走自己的键，不动上面的账"
    );

    // 归属分发：root fiber 的 volatile 只有 root 上的 owned 监听器收得到
    let slot = Volatile::new(&ctx, json!({"r": 1}));
    slot.update(json!({"r": 2}));
    let owned_entries = wait_entries(&owned, 1).await;
    assert_eq!(
        owned_entries,
        vec!["root|<无标签>".to_string()],
        "owned payload 前两位应为 [fiber id, fiber 名]，第三位标签缺省为 null"
    );
    assert_eq!(
        snapshot(&legacy).last().unwrap(),
        "parallel",
        "legacy 表不得收到 owned 事件（两表按事件键互不串台，含 volatile 键上的 legacy 监听器）"
    );
}

/// 注册即 effect：fiber 卸载后归属监听器注销，volatile 再也发不出去。
#[tokio::test]
async fn fiber_卸载后归属监听器注销_volatile_不再发() {
    let ctx = Context::new();
    let log = new_log();
    let slots = Arc::new(Mutex::new(Vec::new()));
    let plug = ctx.plugin(
        VolatilePlugin { name: "plug-x", log: log.clone(), slots: slots.clone() },
        json!(null),
    );
    plug.await_ready().await.expect("应装载完成");
    let slot = slots.lock().unwrap()[0].clone();

    slot.update(json!({"v": 1}));
    wait_entries(&log, 1).await;

    plug.dispose().await.expect("plug-x 应卸载");
    plug.wait_disposed().await;
    tokio::time::sleep(Duration::from_millis(50)).await; // 等卸载清理 disposer 落定

    let before = snapshot(&log).len();
    slot.update(json!({"v": 2}));
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        snapshot(&log).len(),
        before,
        "fiber 卸载后不得再收到 volatile 事件（注册即 effect）"
    );
    assert!(!plug.is_active(), "卸载后 fiber 不应仍为 Active");
}

/// 反向串台防线：同一事件键上走 legacy 五分发（emit/serial），归属监听器不得收到。
///
/// 与「owned_事件面与既有五分发共存不回归」互补：那边证「owned 分发不落 legacy
/// 表」，本用例证「legacy 分发不落 owned 表」——双向都不许串台（S7 评审 P2-3）。
#[tokio::test]
async fn legacy_五分发不触发同键归属监听器() {
    let ctx = Context::new();
    let log = new_log();
    let sink = log.clone();
    let listener: Listener = Arc::new(move |_args: &[Value]| {
        let sink = sink.clone();
        Box::pin(async move {
            sink.lock().unwrap().push("owned-不应收到".to_string());
            Ok(ListenerOutcome::Continue)
        })
    });
    ctx.on_owned("probe/cross", listener)
        .expect("root 上应能注册归属监听器");

    // 同一事件键走 legacy 分发两路：emit（fire-and-forget）+ serial（await）
    ctx.emit("probe/cross", &[json!(1)]);
    ctx.serial("probe/cross", &[json!(2)])
        .await
        .expect("legacy serial 照常返回");
    tokio::time::sleep(Duration::from_millis(80)).await; // 留足 fire-and-forget spawn 窗口
    assert!(
        snapshot(&log).is_empty(),
        "legacy 五分发不得触发同键归属监听器（两表互不串台·反向）"
    );
}