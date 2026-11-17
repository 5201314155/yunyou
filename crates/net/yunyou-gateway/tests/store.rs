//! gateway.db 存储层回归（自 src/store.rs tests mod 外迁，行数清偿批 task-22；
//! 纯搬移零行为变更；先例=cli tests/shell_commands.rs 外迁）。单测即契约：
//! 全部走 pub 面（GatewayStore/PairingRow/TokenRow），src 侧 store.rs 退安全水位。

use yunyou_gateway::store::{GatewayStore, PairingRow, TokenRow};

fn pairing_row(hash: &str, expires: u64) -> PairingRow {
    PairingRow {
        token_hash: hash.into(),
        method: "qr".into(),
        network: "lan".into(),
        device_code_hash: None,
        expires_at_ms: expires,
        consumed_at_ms: None,
        fail_count: 0,
        locked_until_ms: None,
    }
}

/// 原子一次性语义（§3.1 防重放）：并发重复消费只有一方成功；过期拒绝。
#[test]
fn 原子一次性消费与过期拒绝() {
    let store = GatewayStore::open_in_memory().unwrap();
    store.create_pairing(&pairing_row("h1", 1_000)).unwrap();
    // 恰好未过期（now < expires）
    assert!(store.consume_pairing("h1", 999).unwrap(), "首次消费应成功");
    // 重放=已消费拒绝
    assert!(!store.consume_pairing("h1", 999).unwrap(), "重放必须拒绝");
    // 过期拒绝（新 pairing，now ≥ expires）
    store.create_pairing(&pairing_row("h2", 1_000)).unwrap();
    assert!(!store.consume_pairing("h2", 1_000).unwrap(), "过期必须拒绝");
}

/// 失败计数与锁定落列（§3.2 配对会话级）。
#[test]
fn 配对失败计数与锁定() {
    let store = GatewayStore::open_in_memory().unwrap();
    store.create_pairing(&pairing_row("h1", 9_999)).unwrap();
    assert_eq!(store.bump_pairing_fail("h1").unwrap(), 1);
    assert_eq!(store.bump_pairing_fail("h1").unwrap(), 2);
    store.lock_pairing("h1", 5_000).unwrap();
    let row = store.find_pairing_by_token("h1").unwrap().unwrap();
    assert_eq!(row.fail_count, 2);
    assert_eq!(row.locked_until_ms, Some(5_000));
}

/// 令牌生命周期：签发→查活跃→吊销→不可见→generation 递增（§3.1 吊销立即生效）。
#[test]
fn 令牌签发吊销与代际() {
    let store = GatewayStore::open_in_memory().unwrap();
    store
        .insert_token(&TokenRow {
            token_hash: "sess1".into(),
            device_id: "d1".into(),
            device_name: "手机".into(),
            scope_csv: "chat,panel:read".into(),
            generation: 1,
            created_at_ms: 100,
            last_active_ms: 100,
            revoked_at_ms: None,
        })
        .unwrap();
    assert!(store.find_active_token("sess1").unwrap().is_some());
    assert_eq!(store.active_device_count().unwrap(), 1);
    store.revoke_device("d1", 200).unwrap();
    assert!(store.find_active_token("sess1").unwrap().is_none(), "吊销立即生效");
    assert_eq!(store.active_device_count().unwrap(), 0);
}

/// 设备↔会话惰性映射（§2.5）：同设备续聊复用同会话。
#[test]
fn 会话映射同设备复用() {
    let store = GatewayStore::open_in_memory().unwrap();
    let s1 = store.upsert_session_map("d1", "gen-1").unwrap();
    let s2 = store.upsert_session_map("d1", "gen-2").unwrap();
    assert_eq!(s1, s2, "同设备重连映射同一 session（续聊）");
    assert_eq!(s1, "gen-1");
}
