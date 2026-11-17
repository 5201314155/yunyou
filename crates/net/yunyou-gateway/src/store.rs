//! gateway.db 存储层（rusqlite bundled；方案 §3.1 令牌表/§3.2 锁定/§2.5 会话映射）。
//!
//! 表结构 v1：pairings（配对会话：哈希/时效/一次性/失败锁定双列）+tokens（会话
//! 令牌：哈希/设备/scope/generation/吊销）+session_map（device↔session 惰性映射）
//! +locks（双维失败锁定计数）。审计不落本库（§3.3 jsonl 文件介质，见 audit.rs）。
//!
//! 原子一次性语义（§3.1 防重放）由 SQL 行数判定承载：`UPDATE ... WHERE
//! consumed IS NULL AND expires>now` 影响行数 0=已消费/过期，并发重复消费只有
//! 一方成功——判定逻辑在本层单测锁死。
//!
//! 并发形态：rusqlite Connection 非 Sync——Mutex 包装=官方推荐形态（内核
//! Send+Sync 纪律 1 的存储侧兑现）；锁内纯同步短临界区零 await（家规「锁内
//! 不 await」同律），单网关进程内单连接足够。

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

/// 令牌行（会话令牌；配对令牌不建行——一次性消费后即焚）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub token_hash: String,
    pub device_id: String,
    pub device_name: String,
    /// scope 逗号分隔（chat,panel:read,panel:write 子集；解析归 scope.rs）。
    pub scope_csv: String,
    /// 令牌表版本号（吊销/锁定 bump=缓存立即失效，§3.1）。
    pub generation: u64,
    pub created_at_ms: u64,
    pub last_active_ms: u64,
    /// 吊销时刻（None=活跃）。
    pub revoked_at_ms: Option<u64>,
}

/// IP 级锁定现态（bump_ip_lock/ip_lock_state 产出；锁定中=locked_until 在未来）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpLockState {
    /// 滑窗内当前失败计数。
    pub fail_count: u32,
    /// 锁定截止（None=未锁）。
    pub locked_until_ms: Option<u64>,
}

impl IpLockState {
    /// 是否锁定中（毫秒时钟由调用方注入）。
    pub fn is_locked(&self, now_ms: u64) -> bool {
        self.locked_until_ms.is_some_and(|until| now_ms < until)
    }
}

/// 配对会话行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRow {
    pub token_hash: String,
    pub method: String,
    pub network: String,
    /// 验证码流程的短码哈希（扫码流程=None——短码与令牌一一映射）。
    pub device_code_hash: Option<String>,
    pub expires_at_ms: u64,
    /// 消费时刻（None=未消费；一次性语义锚）。
    pub consumed_at_ms: Option<u64>,
    /// 连续失败次数（§3.2：5 次→锁 15 分钟）。
    pub fail_count: u32,
    /// 锁定截止（None=未锁）。
    pub locked_until_ms: Option<u64>,
}

/// gateway.db 存储句柄。
pub struct GatewayStore {
    conn: Mutex<Connection>,
}

impl GatewayStore {
    /// 打开/创建库（父目录调用方保证；`:memory:` 用于测试）。
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// 内存库（测试/纯逻辑单测用）。
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pairings (
                token_hash TEXT PRIMARY KEY,
                method TEXT NOT NULL,
                network TEXT NOT NULL,
                device_code_hash TEXT,
                expires_at_ms INTEGER NOT NULL,
                consumed_at_ms INTEGER,
                fail_count INTEGER NOT NULL DEFAULT 0,
                locked_until_ms INTEGER
            );
            CREATE TABLE IF NOT EXISTS tokens (
                token_hash TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                device_name TEXT NOT NULL,
                scope_csv TEXT NOT NULL,
                generation INTEGER NOT NULL DEFAULT 1,
                created_at_ms INTEGER NOT NULL,
                last_active_ms INTEGER NOT NULL,
                revoked_at_ms INTEGER
            );
            CREATE TABLE IF NOT EXISTS session_map (
                device_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS locks (
                lock_key TEXT PRIMARY KEY,
                fail_count INTEGER NOT NULL,
                window_start_ms INTEGER NOT NULL,
                locked_until_ms INTEGER
            );",
        )?;
        // 既有库迁移：locks 表 v1 无锁定截止列（G-2 接线批补列；失败忽略=列已在）。
        let _ = conn.execute_batch("ALTER TABLE locks ADD COLUMN locked_until_ms INTEGER;");
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// 登记配对会话（一次性令牌哈希入库，未消费）。
    pub fn create_pairing(&self, row: &PairingRow) -> rusqlite::Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO pairings (token_hash, method, network, device_code_hash,
                expires_at_ms, consumed_at_ms, fail_count, locked_until_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, 0, NULL)",
            params![
                row.token_hash,
                row.method,
                row.network,
                row.device_code_hash,
                row.expires_at_ms
            ],
        )?;
        Ok(())
    }

    /// 按短码哈希找配对会话（验证码流程；扫码流程直接按令牌哈希）。
    pub fn find_pairing_by_code(
        &self,
        device_code_hash: &str,
    ) -> rusqlite::Result<Option<PairingRow>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT token_hash, method, network, device_code_hash, expires_at_ms,
                    consumed_at_ms, fail_count, locked_until_ms
             FROM pairings WHERE device_code_hash = ?1",
            params![device_code_hash],
            map_pairing,
        )
        .optional()
    }

    /// 按令牌哈希找配对会话（扫码流程）。
    pub fn find_pairing_by_token(&self, token_hash: &str) -> rusqlite::Result<Option<PairingRow>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT token_hash, method, network, device_code_hash, expires_at_ms,
                    consumed_at_ms, fail_count, locked_until_ms
             FROM pairings WHERE token_hash = ?1",
            params![token_hash],
            map_pairing,
        )
        .optional()
    }

    /// **原子一次性消费**（§3.1 防重放）：UPDATE 影响行数 0=已消费/过期→false；
    /// 并发重复消费只有一方拿到 true。
    pub fn consume_pairing(&self, token_hash: &str, now_ms: u64) -> rusqlite::Result<bool> {
        let conn = self.lock();
        let changed = conn.execute(
            "UPDATE pairings SET consumed_at_ms = ?1
             WHERE token_hash = ?2 AND consumed_at_ms IS NULL AND expires_at_ms > ?1",
            params![now_ms, token_hash],
        )?;
        Ok(changed == 1)
    }

    /// 配对会话失败计数 +1（§3.2：连续 5 次→锁 15 分钟由调用方判定落列）。
    pub fn bump_pairing_fail(&self, token_hash: &str) -> rusqlite::Result<u32> {
        let conn = self.lock();
        conn.execute(
            "UPDATE pairings SET fail_count = fail_count + 1 WHERE token_hash = ?1",
            params![token_hash],
        )?;
        let count: u32 = conn.query_row(
            "SELECT fail_count FROM pairings WHERE token_hash = ?1",
            params![token_hash],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// 落锁定截止（§3.2 锁定 15 分钟）。
    pub fn lock_pairing(&self, token_hash: &str, until_ms: u64) -> rusqlite::Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE pairings SET locked_until_ms = ?1 WHERE token_hash = ?2",
            params![until_ms, token_hash],
        )?;
        Ok(())
    }

    /// 签发会话令牌（登记新行；重配对=调用方先吊销旧令牌再签发，§3.1 每设备 1 枚）。
    pub fn insert_token(&self, row: &TokenRow) -> rusqlite::Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO tokens (token_hash, device_id, device_name, scope_csv,
                generation, created_at_ms, last_active_ms, revoked_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                row.token_hash,
                row.device_id,
                row.device_name,
                row.scope_csv,
                row.generation,
                row.created_at_ms,
                row.last_active_ms
            ],
        )?;
        Ok(())
    }

    /// 按哈希查活跃令牌（revoked 过滤；中间件查表入口，§3.1 吊销立即生效）。
    pub fn find_active_token(&self, token_hash: &str) -> rusqlite::Result<Option<TokenRow>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT token_hash, device_id, device_name, scope_csv, generation,
                    created_at_ms, last_active_ms, revoked_at_ms
             FROM tokens WHERE token_hash = ?1 AND revoked_at_ms IS NULL",
            params![token_hash],
            map_token,
        )
        .optional()
    }

    /// 全活跃令牌快照（中间件 generation 缓存的行源；设备量个位级）。
    pub fn all_active_tokens(&self) -> rusqlite::Result<Vec<TokenRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT token_hash, device_id, device_name, scope_csv, generation,
                    created_at_ms, last_active_ms, revoked_at_ms
             FROM tokens WHERE revoked_at_ms IS NULL",
        )?;
        let rows = stmt.query_map([], map_token)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 吊销单设备全部令牌（DELETE /devices/{id} 语义；返回吊销枚数）。
    pub fn revoke_device(&self, device_id: &str, now_ms: u64) -> rusqlite::Result<usize> {
        let conn = self.lock();
        conn.execute(
            "UPDATE tokens SET revoked_at_ms = ?1 WHERE device_id = ?2 AND revoked_at_ms IS NULL",
            params![now_ms, device_id],
        )
    }

    /// 吊销全部（DELETE /devices 语义）。
    pub fn revoke_all(&self, now_ms: u64) -> rusqlite::Result<usize> {
        let conn = self.lock();
        conn.execute(
            "UPDATE tokens SET revoked_at_ms = ?1 WHERE revoked_at_ms IS NULL",
            params![now_ms],
        )
    }

    /// bump generation（吊销/锁定=缓存立即失效，§3.1）。
    pub fn bump_generation(&self, device_id: &str) -> rusqlite::Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE tokens SET generation = generation + 1 WHERE device_id = ?1",
            params![device_id],
        )?;
        Ok(())
    }

    /// 活跃设备数（未吊销令牌数=设备数，§3.1 每设备 1 枚；上限检查用）。
    pub fn active_device_count(&self) -> rusqlite::Result<u32> {
        let conn = self.lock();
        let count: u32 = conn.query_row(
            "SELECT COUNT(DISTINCT device_id) FROM tokens WHERE revoked_at_ms IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// 设备↔会话惰性映射（§2.5）：存在即复用（同设备重连续聊），不存在即新建
    /// （new_session_id=调用方经 CSPRNG 铸的新会话 id，冲突时被既有映射覆盖）。
    pub fn upsert_session_map(
        &self,
        device_id: &str,
        new_session_id: &str,
    ) -> rusqlite::Result<String> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO session_map (device_id, session_id) VALUES (?1, ?2)
             ON CONFLICT(device_id) DO NOTHING",
            params![device_id, new_session_id],
        )?;
        let session_id: String = conn.query_row(
            "SELECT session_id FROM session_map WHERE device_id = ?1",
            params![device_id],
            |row| row.get(0),
        )?;
        Ok(session_id)
    }

    /// 双维失败锁定计数（§3.2 锁定双维：源 IP+配对会话；key=调用方拼装）。
    /// 15 分钟滑窗（window_start 重置语义由调用方按 now 判定）。
    pub fn bump_lock(&self, lock_key: &str, now_ms: u64) -> rusqlite::Result<u32> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO locks (lock_key, fail_count, window_start_ms) VALUES (?1, 1, ?2)
             ON CONFLICT(lock_key) DO UPDATE SET fail_count = fail_count + 1",
            params![lock_key, now_ms],
        )?;
        let count: u32 = conn.query_row(
            "SELECT fail_count FROM locks WHERE lock_key = ?1",
            params![lock_key],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// IP 级失败锁定（§3.2 双维之「源 IP 维」，G-2 接线；key=调用方拼装
    /// 如 "ip:1.2.3.4"）。语义：滑窗（window_ms）内累计失败，达 fail_limit
    /// →锁 lock_ms（写 locked_until_ms 并清零计数重新起窗）；锁定期内不增计
    /// 只回报锁；窗外旧计数清零重计。返回现态供调用方判定 429。
    pub fn bump_ip_lock(
        &self,
        lock_key: &str,
        now_ms: u64,
        window_ms: u64,
        fail_limit: u32,
        lock_ms: u64,
    ) -> rusqlite::Result<IpLockState> {
        let conn = self.lock();
        let existing: Option<(u32, u64, Option<u64>)> = conn
            .query_row(
                "SELECT fail_count, window_start_ms, locked_until_ms FROM locks
                 WHERE lock_key = ?1",
                params![lock_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((_, _, Some(until))) = &existing {
            if now_ms < *until {
                return Ok(IpLockState { fail_count: fail_limit, locked_until_ms: Some(*until) });
            }
        }
        // 窗外（或无记录）=计数清零重计；窗内=累加
        let (count, window_start) = match existing {
            Some((count, start, _)) if now_ms.saturating_sub(start) < window_ms => {
                (count + 1, start)
            }
            _ => (1, now_ms),
        };
        if count >= fail_limit {
            let until = now_ms + lock_ms;
            conn.execute(
                "INSERT INTO locks (lock_key, fail_count, window_start_ms, locked_until_ms)
                 VALUES (?1, 0, ?2, ?3)
                 ON CONFLICT(lock_key) DO UPDATE SET
                    fail_count = 0, window_start_ms = ?2, locked_until_ms = ?3",
                params![lock_key, now_ms, until],
            )?;
            return Ok(IpLockState { fail_count: fail_limit, locked_until_ms: Some(until) });
        }
        conn.execute(
            "INSERT INTO locks (lock_key, fail_count, window_start_ms, locked_until_ms)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(lock_key) DO UPDATE SET
                fail_count = ?2, window_start_ms = ?3, locked_until_ms = NULL",
            params![lock_key, count, window_start],
        )?;
        Ok(IpLockState { fail_count: count, locked_until_ms: None })
    }

    /// 会话令牌活跃触点（G-3 滑动过期：每次认证成功调用——last_active_ms
    /// 前滚=30 天滑动窗口的滑动动作）。
    pub fn touch_token_last_active(&self, token_hash: &str, now_ms: u64) -> rusqlite::Result<()> {
        self.lock().execute(
            "UPDATE tokens SET last_active_ms = ?2 WHERE token_hash = ?1",
            params![token_hash, now_ms],
        )?;
        Ok(())
    }

    /// IP 锁现态查询（接线端点前置闸用：锁定中=直接 429 不落计数）。
    pub fn ip_lock_state(&self, lock_key: &str, now_ms: u64) -> rusqlite::Result<IpLockState> {
        let conn = self.lock();
        let state: Option<(u32, Option<u64>)> = conn
            .query_row(
                "SELECT fail_count, locked_until_ms FROM locks WHERE lock_key = ?1",
                params![lock_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(match state {
            Some((count, Some(until))) if now_ms < until => {
                IpLockState { fail_count: count, locked_until_ms: Some(until) }
            }
            _ => IpLockState { fail_count: 0, locked_until_ms: None },
        })
    }

    /// 锁守卫（全部方法的统一入口；expect=A 类锁守卫，K-14 台账口径）。
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("gateway 库锁")
    }
}

fn map_pairing(row: &rusqlite::Row<'_>) -> rusqlite::Result<PairingRow> {
    Ok(PairingRow {
        token_hash: row.get(0)?,
        method: row.get(1)?,
        network: row.get(2)?,
        device_code_hash: row.get(3)?,
        expires_at_ms: row.get(4)?,
        consumed_at_ms: row.get(5)?,
        fail_count: row.get(6)?,
        locked_until_ms: row.get(7)?,
    })
}

fn map_token(row: &rusqlite::Row<'_>) -> rusqlite::Result<TokenRow> {
    Ok(TokenRow {
        token_hash: row.get(0)?,
        device_id: row.get(1)?,
        device_name: row.get(2)?,
        scope_csv: row.get(3)?,
        generation: row.get(4)?,
        created_at_ms: row.get(5)?,
        last_active_ms: row.get(6)?,
        revoked_at_ms: row.get(7)?,
    })
}

// 测试外迁 tests/store.rs（行数清偿批 task-22，先例=cli tests/shell_commands.rs）。
