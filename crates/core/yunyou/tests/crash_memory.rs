//! 崩溃隔离·插拔 N 次内存回归（task-11/S-ISO-4 拆分件，设计 §6.2）。
//!
//! 主断言=Arc 强计数（CI 确定性，C12，基线取首次插拔后=云枢验收补充②）；
//! 辅断言=RSS 观测（#[ignore] 手动档，C13）。

mod crash_fixtures;

use serde_json::json;
use std::time::Duration;
use yunyou::{Context, FiberState, ServiceId};
use crash_fixtures::{
    wait_state, BarePlugin, CountingHealthy, PoisonMode, PoisonPlugin, Probe,
};

// ── C12+C13：插拔 N 次内存不涨（主断言=强计数；辅断言=RSS 手动档） ─────────

/// C12：N=50 循环——毒化插件 restart（每次进 Failed）+ 健康插件 update 往返。
/// 强计数基线取**首次插拔后**（云枢验收补充②：防夹具自建服务的心跳计数混入）；
/// 此后每轮断言 ContextInner 强计数回落基线（装卸机器零泄漏的因果直接证据）。
#[tokio::test(flavor = "multi_thread")]
async fn c12_插拔50次强计数不涨() {
    let root = Context::new();
    let (poison, _slot, _healed) = PoisonPlugin::new("毒化己", PoisonMode::Setup);
    let poison_handle = root.plugin(poison, json!({}));
    let _ = poison_handle.await_ready().await;
    wait_state(&poison_handle, FiberState::Failed).await;

    let healthy = root.plugin(CountingHealthy::new("健康庚"), json!({}));
    healthy.await_ready().await.expect("健康插件装载");
    wait_state(&healthy, FiberState::Active).await;

    // 首轮插拔（基线采样点在其后——云枢补充②）
    poison_handle.restart().await.expect_err("毒化插件每轮 restart 应再炸");
    wait_state(&poison_handle, FiberState::Failed).await;
    healthy.update(json!({"n": 1})).await.expect("健康插件 update 往返");
    wait_state(&healthy, FiberState::Active).await;
    let probe = root.get::<dyn Probe>(ServiceId::of::<dyn Probe>()).unwrap();
    let baseline = probe.strong_count();
    assert!(baseline >= 1, "ContextInner 强计数应非零（fiber 树持有）");

    // N-1 轮插拔（设计 §A.4-4：强计数断言在全部稳定后采样——在途装卸任务的
    // 临时 Context 句柄会让逐轮即时采样出假阳性，实测竞态复现过）
    for i in 1..50 {
        poison_handle.restart().await.expect_err("循环内 restart 仍应炸");
        wait_state(&poison_handle, FiberState::Failed).await;
        healthy.update(json!({"n": i + 1})).await.expect("update 往返");
        wait_state(&healthy, FiberState::Active).await;
    }
    // 收尾断言：全部插拔落定后强计数回落基线（有界轮询避在途任务尾差）
    let mut final_count = 0;
    for _ in 0..400 {
        let probe = root.get::<dyn Probe>(ServiceId::of::<dyn Probe>()).unwrap();
        final_count = probe.strong_count();
        if final_count == baseline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        final_count, baseline,
        "50 轮插拔全部落定后强计数应回落基线（装卸机器零泄漏）"
    );
    // 主板存活 + 健康插件服务正常
    let probe = root.get::<dyn Probe>(ServiceId::of::<dyn Probe>()).unwrap();
    assert_eq!(probe.ping(), 7);
    let final_scan = root.plugin(BarePlugin::new("空辛"), json!({}));
    final_scan.await_ready().await.expect("50 轮插拔后扫描路径仍正常");
}

/// C13（#[ignore] 手动档）：N=1000 插拔，采样 VmRSS——常量级抖动、不线性增长。
/// 定档理由（设计 §6.2）：RSS 受分配器影响波动大，不宜作 CI 主断言。跑法：
/// `cargo test -p yunyou --test crash_isolation c13 -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "手动档：RSS 观测（设计 §6.2 辅断言），CI 不跑"]
async fn c13_插拔1000次rss不线性增长() {
    let root = Context::new();
    let (poison, _slot, _healed) = PoisonPlugin::new("毒化壬", PoisonMode::Setup);
    let poison_handle = root.plugin(poison, json!({}));
    let _ = poison_handle.await_ready().await;
    wait_state(&poison_handle, FiberState::Failed).await;
    let rss_start = read_vm_rss_kb();
    for _ in 0..1000 {
        poison_handle.restart().await.expect_err("每轮 restart 应再炸");
        wait_state(&poison_handle, FiberState::Failed).await;
    }
    let rss_end = read_vm_rss_kb();
    println!("RSS: start={rss_start}KB end={rss_end}KB 增量={}KB", rss_end - rss_start);
    assert!(
        rss_end.saturating_sub(rss_start) < 50 * 1024,
        "1000 次插拔 RSS 线性增长（增量 {}KB ≥ 50MB 阈值）——泄漏",
        rss_end.saturating_sub(rss_start)
    );
}

fn read_vm_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .and_then(|line| line.split_whitespace().nth(1).and_then(|n| n.parse().ok()))
        .unwrap_or(0)
}
