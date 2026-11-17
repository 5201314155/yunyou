//! 定时触发缝（scheduler.v1 S1+S2，设计 docs/设计/scheduler.v1-定时触发缝设计-20261118.md）。
//!
//! 缝本体：触发器注册/注销/观测 + 内存调度循环（At/Interval/Cron 三型）。
//! 核心计算面 [`next_fire_ms`] 纯函数化——测试注入时钟（毫秒入参），禁 sleep 真等。
//! 调度线程 1s 粒度 tick（实现细节非测试赌注）；到期回调 [`SchedulerTrigger::on_fire`]
//! 错误隔离：单条目 Err 只影响本条目，不炸循环（设计 §五投递失败三态的 v1 子集）。
//!
//! 边界（本模块不含）：会话投递缝（proto v1.1 开闸）/持久化 durable（S4）/
//! 审批联动（审批词汇开闸）——见设计 §九实施批次。
//!
//! ## 最小用法
//!
//! ```no_run
//! use std::sync::Arc;
//! use yunyou_plugin_sdk::scheduler::{ScheduleSpec, Scheduler};
//!
//! let sched = Arc::new(Scheduler::new(|| 1_000_000)); // 时钟注入（生产=epoch 毫秒）
//! sched.register_schedule(
//!     None,
//!     ScheduleSpec::Interval { every_ms: 300_000 },
//!     "[定时触发 {id}] 该喝水了".into(),
//! ).expect("注册应成功");
//! // 起调度线程（进程退出即焚；durable 持久化=S4）
//! sched.spawn_thread();
//! ```

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// 触发规格（v1 三型；设计 §二）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScheduleSpec {
    /// 一次性：绝对时刻（epoch 毫秒；已过时刻注册=Err）。
    At { at_ms: u64 },
    /// 循环间隔（毫秒；下限校验=工具面 min_interval_minutes，缝层不重复设防）。
    Interval { every_ms: u64 },
    /// cron 表达式（5 段标准子集：分 时 日 月 周；DTSTART 类复杂特性 v1 拒收=
    /// cron crate 解析失败即 Err）。
    Cron { expr: String },
}

/// 调度条目（持久化单元形状；v1 schema 冻结面，设计 §二）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScheduleEntry {
    /// 调度 id（宿主铸造：`sched-<16hex>`）。
    pub id: String,
    /// 所属会话 id（会话级任务重启后只对原会话恢复；全局任务 None）。
    pub session_id: Option<String>,
    /// 触发规格（三选一）。
    pub spec: ScheduleSpec,
    /// 到期注入的消息 payload（用户角色文本；占位符 {id}/{fired_at}）。
    pub message: String,
    /// durable 标记（true=落盘跨重启，**须用户明说**——工具提示词强制；落盘=S4）。
    pub durable: bool,
    /// 创建时刻（epoch 毫秒）。
    pub created_at_ms: u64,
    /// 暂停态（list/delete 外第三态；v1 只占位不暴露工具面）。
    pub paused: bool,
}

/// 注册失败（fail-loud 人话；调用方直出模型面）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleError {
    pub message: String,
}

/// 触发器（到期回调面）：调度循环到期时逐条目调 [`Self::on_fire`]。
pub trait SchedulerTrigger: Send + Sync {
    /// 触发器显示名（诊断与观测面；同名注册 fail-loud）。
    fn name(&self) -> &str;

    /// 到期回调：输入=到期条目快照。返回 Err 只影响本条目（记录+顺延），不炸循环。
    fn on_fire(&self, entry: &ScheduleEntry, fired_at_ms: u64) -> Result<(), String>;

    /// 空闲 tick 预留（主动模式 KAIROS 缝；缺省空实现=本卡零行为，设计 §七）。
    fn on_idle_tick(&self, _now_ms: u64) {}
}

/// 计算条目的下一次触发时刻（epoch 毫秒；纯函数=测试注入时钟锚）。
///
/// 返回 None=已终态（At 型已触发过且无未来时刻）。
pub fn next_fire_ms(entry: &ScheduleEntry, now_ms: u64, last_fired_ms: Option<u64>) -> Option<u64> {
    if entry.paused {
        return None;
    }
    match &entry.spec {
        ScheduleSpec::At { at_ms } => {
            if last_fired_ms.is_some() {
                None // At 型一次即终
            } else {
                Some(*at_ms) // 未触发过=下一时刻恒为 at（已过=到期，由 tick 判定触发）
            }
        }
        ScheduleSpec::Interval { every_ms } => {
            if *every_ms == 0 {
                return None;
            }
            // 下一时刻=锚点+间隔（恰到期=due，由 tick 判定触发）；miss 多轮时
            // 触发后 last_fired=now → 下一时刻=now+every，天然「顺延不补偿」（设计 §五）
            let anchor = last_fired_ms.unwrap_or(entry.created_at_ms);
            Some(anchor + every_ms)
        }
        ScheduleSpec::Cron { expr } => {
            let schedule = cron::Schedule::from_str(expr).ok()?;
            let after_ms = last_fired_ms.unwrap_or(now_ms).max(now_ms);
            let after = chrono::DateTime::from_timestamp_millis(after_ms as i64)?;
            schedule.after(&after).next().map(|dt| {
                (dt.timestamp_millis().max(0)) as u64
            })
        }
    }
}

/// 调度器（内存注册表+tick 线程；v1 缝本体）。
pub struct Scheduler {
    inner: Mutex<Inner>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    seq: AtomicU64,
    trigger: Mutex<Option<Arc<dyn SchedulerTrigger>>>,
}

struct Inner {
    entries: Vec<(ScheduleEntry, Option<u64>)>, // (条目, 上次触发时刻)
}

impl Scheduler {
    /// 构造（时钟注入：生产=`SystemTime::now` epoch 毫秒闭包；测试=假时钟）。
    pub fn new(clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self {
            inner: Mutex::new(Inner { entries: Vec::new() }),
            clock: Box::new(clock),
            seq: AtomicU64::new(1),
            trigger: Mutex::new(None),
        }
    }

    /// 注册调度条目（校验：At 过期拒/Interval 非零/cron 合法；返回 id+下次触发时刻）。
    pub fn register_schedule(
        &self,
        session_id: Option<String>,
        spec: ScheduleSpec,
        message: String,
    ) -> Result<(String, Option<u64>), ScheduleError> {
        let now = (self.clock)();
        // 预检（注册面 fail-loud：坏规格当场报，不进循环后静默终态）
        let probe = ScheduleEntry {
            id: String::new(),
            session_id: session_id.clone(),
            spec: spec.clone(),
            message: String::new(),
            durable: false,
            created_at_ms: now,
            paused: false,
        };
        match &spec {
            ScheduleSpec::At { at_ms } if *at_ms < now => {
                return Err(ScheduleError {
                    message: format!("时刻 {at_ms} 已过（当前 {now}）——请给未来时刻"),
                });
            }
            ScheduleSpec::Interval { every_ms } if *every_ms == 0 => {
                return Err(ScheduleError { message: "间隔必须大于 0".into() });
            }
            ScheduleSpec::Cron { expr } if cron::Schedule::from_str(expr).is_err() => {
                return Err(ScheduleError {
                    message: format!("cron 表达式 \"{expr}\" 不合法（5 段标准子集：分 时 日 月 周）"),
                });
            }
            _ => {}
        }
        let first = next_fire_ms(&probe, now, None).ok_or_else(|| ScheduleError {
            message: "规格无未来触发时刻".into(),
        })?;
        let id = format!("sched-{:016x}", self.seq.fetch_add(1, Ordering::Relaxed));
        let entry = ScheduleEntry { id: id.clone(), session_id, spec, message, durable: false, created_at_ms: now, paused: false };
        #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
        self.inner.lock().expect("调度表锁").entries.push((entry, None));
        Ok((id, Some(first)))
    }

    /// 注销（按 id；不存在=Err 指路人话）。
    pub fn unregister_schedule(&self, id: &str) -> Result<(), ScheduleError> {
        #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
        let mut inner = self.inner.lock().expect("调度表锁");
        let before = inner.entries.len();
        inner.entries.retain(|(e, _)| e.id != id);
        if inner.entries.len() == before {
            return Err(ScheduleError {
                message: format!("调度 {id} 不存在（schedule_list 查看在册清单）"),
            });
        }
        Ok(())
    }

    /// 观测清单（只读快照含下次触发时刻；面板/工具面用）。
    pub fn list(&self) -> Vec<(ScheduleEntry, Option<u64>)> {
        let now = (self.clock)();
        #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
        self.inner
            .lock()
            .expect("调度表锁")
            .entries
            .iter()
            .map(|(e, last)| (e.clone(), next_fire_ms(e, now, *last)))
            .collect()
    }

    /// 挂接触发器（到期回调执行体；重复挂接=覆盖并 warn）。
    pub fn set_trigger(&self, trigger: Arc<dyn SchedulerTrigger>) {
        #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
        let mut slot = self.trigger.lock().expect("触发器槽锁");
        if slot.is_some() {
            eprintln!("[yunyou-plugin-sdk:scheduler] 警告：触发器被覆盖（重复 set_trigger）");
        }
        *slot = Some(trigger);
    }

    /// 单次 tick（循环体核心；独立成方法=测试注入时钟驱动，无需真等 1s）。
    ///
    /// 返回本 tick 触发的条目数。逻辑：收集到期→逐条回调（Err 隔离）→推进
    /// last_fired（At 型终态摘除）。
    pub fn tick(&self) -> usize {
        let now = (self.clock)();
        let mut fired = 0;
        #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
        let mut inner = self.inner.lock().expect("调度表锁");
        let mut remove_ids: Vec<String> = Vec::new();
        for (entry, last) in inner.entries.iter_mut() {
            let Some(next) = next_fire_ms(entry, now, *last) else { continue };
            if next > now {
                continue;
            }
            fired += 1;
            *last = Some(now);
            if matches!(entry.spec, ScheduleSpec::At { .. }) {
                remove_ids.push(entry.id.clone());
            }
            #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
            let trigger = self.trigger.lock().expect("触发器槽锁").clone();
            if let Some(trigger) = trigger {
                if let Err(err) = trigger.on_fire(entry, now) {
                    // 错误隔离：单条目失败只留痕顺延，不炸循环（设计 §五）
                    eprintln!(
                        "[yunyou-plugin-sdk:scheduler] 条目 {} 触发失败（顺延）: {err}",
                        entry.id
                    );
                }
            }
        }
        inner.entries.retain(|(e, _)| !remove_ids.contains(&e.id));
        if fired == 0 {
            #[allow(clippy::expect_used)] // 中毒 tripwire（std 锁保留裁决）：边界后触发=SDK 缺陷
            if let Some(trigger) = self.trigger.lock().expect("触发器槽锁").clone() {
                trigger.on_idle_tick(now);
            }
        }
        fired
    }

    /// 起调度线程（1s 粒度 tick；进程退出即焚——SDK serve() 主循环退出即收）。
    pub fn spawn_thread(self: &Arc<Self>) {
        let this = Arc::clone(self);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            this.tick();
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)] // 测试面豁免（K-14 观察期口径）

    use super::*;
    use std::sync::atomic::AtomicU64;

    /// 假时钟（注入：可拨快；禁 sleep 真等）。
    struct FakeClock(AtomicU64);
    impl FakeClock {
        fn now(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
        fn advance(&self, ms: u64) {
            self.0.fetch_add(ms, Ordering::Relaxed);
        }
    }

    fn epoch_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    #[test]
    fn at型_未来注册_到期即触发一次即终() {
        let clock = Arc::new(FakeClock(AtomicU64::new(1_000)));
        let sched = Scheduler::new({ let c = Arc::clone(&clock); move || c.now() });
        let (id, next) = sched
            .register_schedule(None, ScheduleSpec::At { at_ms: 2_000 }, "叮".into())
            .expect("未来时刻应注册成功");
        assert!(next == Some(2_000));
        assert!(clock.now() < 2_000);
        assert_eq!(sched.tick(), 0, "未到期不触发");
        clock.advance(1_100); // now=2100 ≥ at
        assert_eq!(sched.tick(), 1, "到期应触发");
        assert_eq!(sched.tick(), 0, "At 型一次即终（摘除）");
        assert!(sched.unregister_schedule(&id).is_err(), "终态条目已摘除=注销 Err");
    }

    #[test]
    fn 过期时刻注册fail_loud() {
        let clock = Arc::new(FakeClock(AtomicU64::new(5_000)));
        let sched = Scheduler::new({ let c = Arc::clone(&clock); move || c.now() });
        let err = sched
            .register_schedule(None, ScheduleSpec::At { at_ms: 1_000 }, "x".into())
            .expect_err("已过时刻应拒");
        assert!(err.message.contains("已过"));
    }

    #[test]
    fn interval型_循环触发_miss顺延不补偿() {
        let clock = Arc::new(FakeClock(AtomicU64::new(0)));
        let sched = Scheduler::new({ let c = Arc::clone(&clock); move || c.now() });
        sched
            .register_schedule(None, ScheduleSpec::Interval { every_ms: 100 }, "心跳".into())
            .expect("注册应成功");
        clock.advance(100);
        assert_eq!(sched.tick(), 1);
        clock.advance(100);
        assert_eq!(sched.tick(), 1);
        clock.advance(350); // 落后 3 轮：miss 策略=追到下一未来，不补偿补发 3 次
        assert_eq!(sched.tick(), 1, "落后多轮只触发一次（顺延不补偿）");
        clock.advance(100);
        assert_eq!(sched.tick(), 1);
    }

    #[test]
    fn cron型_五段表达式解析与下一时刻() {
        let clock = Arc::new(FakeClock(AtomicU64::new(0)));
        let sched = Scheduler::new({ let c = Arc::clone(&clock); move || c.now() });
        // 每小时 0 分（cron crate 6 段含秒；5 段经前缀兼容——设计口径=分 时 日 月 周）
        let (id, next) = sched
            .register_schedule(None, ScheduleSpec::Cron { expr: "0 0 * * * *".into() }, "整点".into())
            .expect("合法 cron 应注册");
        assert!(next.is_some(), "应有下一触发时刻");
        assert!(sched.unregister_schedule(&id).is_ok());
        let err = sched
            .register_schedule(None, ScheduleSpec::Cron { expr: "不合法".into() }, "x".into())
            .expect_err("非法 cron 应拒");
        assert!(err.message.contains("不合法"));
    }

    #[test]
    fn 触发器回调与错误隔离() {
        let clock = Arc::new(FakeClock(AtomicU64::new(0)));
        let sched = Arc::new(Scheduler::new({ let c = Arc::clone(&clock); move || c.now() }));
        struct Firing;
        impl SchedulerTrigger for Firing {
            fn name(&self) -> &str { "test" }
            fn on_fire(&self, entry: &ScheduleEntry, fired_at_ms: u64) -> Result<(), String> {
                assert!(fired_at_ms >= entry.created_at_ms);
                Err("故意的错误隔离样本".into()) // Err 只影响本条目
            }
        }
        sched.set_trigger(Arc::new(Firing));
        sched
            .register_schedule(None, ScheduleSpec::Interval { every_ms: 50 }, "m".into())
            .expect("注册应成功");
        clock.advance(50);
        assert_eq!(sched.tick(), 1, "Err 不影响触发计数，循环不炸");
        clock.advance(50);
        assert_eq!(sched.tick(), 1, "Err 后循环继续（顺延）");
    }

    #[test]
    fn 注销不存在_人话指路() {
        let clock = Arc::new(FakeClock(AtomicU64::new(0)));
        let sched = Scheduler::new({ let c = Arc::clone(&clock); move || c.now() });
        let err = sched.unregister_schedule("sched-does-not-exist").expect_err("应 Err");
        let msg = &err.message;
        assert!(msg.contains("schedule_list"), "人话指路：{msg}");
    }

    #[test]
    fn epoch时钟形态_构造不炸() {
        let _ = Scheduler::new(epoch_now);
    }
}
