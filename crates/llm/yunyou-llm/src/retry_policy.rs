//! 重试策略纯类型（P0-7，超时重试插件设计 §5/§3.2）。
//!
//! 对标官方 llm-retry 的 ResolvedRetryPolicy（index.ts:66-77）：参数集（全 Default
//! 预置）+ 分类决策 [`RetryPolicy::classify`] + 指数退避（§3.2 / 官方 localDelay
//! index.ts:59-65 同构：`initial × 2^(attempt-1)` 封顶 `max`，再乘 `1±jitter`）。
//! 退避执行（sleep 与取消 select）属 S10 策略插件——本类型只产出无量决策与延迟毫秒数。

use crate::error::{LlmFailure, LlmFailureCategory};
use serde::{Deserialize, Serialize};

/// §5 预置白名单（连接/5xx/429/流中断；超时显式排除，安全默认律）。
fn default_retryable_codes() -> Vec<String> {
    ["connection", "http_5xx", "http_429", "stream_interrupted"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// 重试模式。默认 [`Self::Whitelist`]（白名单驱动，设计 §5）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryMode {
    /// 白名单驱动：仅 `retryable_codes` 内的类别可重试。
    #[default]
    Whitelist,
    /// 一律不重试（kill switch；等价关停重试）。
    Never,
}

/// 分类动作（设计 §2：`RetryAction = Retry{delay} | Fail`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAction {
    /// 重试：等待 `delay_ms` 后重发。
    Retry { delay_ms: u64 },
    /// 不重试：失败直出。
    Fail,
}

impl RetryAction {
    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }
}

/// 重试策略参数集（均带 Default 预置值，设计 §5「装好即预置」）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub mode: RetryMode,
    /// 最多重试次数（默认 2 = 最多 3 次尝试，设计 §3.2）。
    pub max_retries: u32,
    /// 可重试类别码白名单（§5 预置连接/5xx/429/流中断；超时显式排除）。
    pub retryable_codes: Vec<String>,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    /// 抖动比例（0..1；§5 预置 0.1）。
    pub jitter_ratio: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            mode: RetryMode::Whitelist,
            max_retries: 2,
            retryable_codes: default_retryable_codes(),
            initial_delay_ms: 500,
            max_delay_ms: 8000,
            jitter_ratio: 0.1,
        }
    }
}

impl RetryPolicy {
    /// 分类决策。
    ///
    /// - `attempt`：本次将是第几次**重试**（1 = 首次重试）。`attempt > max_retries`
    ///   视为预算耗尽 → `Fail`（§3.2 预算是重试上限的兜底）。
    /// - 命中白名单 → `Retry{delay}`（延迟 = 退避基数，中性抖动不改变基数）；
    ///   否则 / `mode = Never` → `Fail`。
    pub fn classify(&self, failure: &LlmFailure, attempt: u32) -> RetryAction {
        // §4 铁律①：取消信号优先级高于一切重试决策——取消链上不产生新尝试，
        // 白名单不得 override（Cancelled 恒 Fail，上层取消语义接管）。
        if failure.category == LlmFailureCategory::Cancelled {
            return RetryAction::Fail;
        }
        if self.mode == RetryMode::Never || attempt > self.max_retries {
            return RetryAction::Fail;
        }
        if self.allows(failure) {
            RetryAction::Retry { delay_ms: self.backoff_base_ms(attempt) }
        } else {
            RetryAction::Fail
        }
    }

    /// 是否命中白名单（按归一化类别码）。
    pub fn allows(&self, failure: &LlmFailure) -> bool {
        let code = failure.code();
        self.retryable_codes.iter().any(|c| c.as_str() == code)
    }

    /// 退避基数（纯计算，无抖动）：`initial × 2^min(attempt-1, 指数上限)`，封顶 `max`。
    /// 指数上限取 20（远超 max_delay 收敛点，仅防 u64 左移溢出）。
    pub fn backoff_base_ms(&self, attempt: u32) -> u64 {
        let exp = attempt.saturating_sub(1).min(20);
        let raw = self.initial_delay_ms.saturating_mul(1u64 << exp);
        raw.min(self.max_delay_ms)
    }

    /// 施加抖动：`base × (1 - ratio + 2×ratio×sample)`，`sample ∈ [0,1)`。
    /// sample=0.5 → 不抖（乘数=1）；S10 由随机数提供 sample，纯计算可确定性单测。
    /// ratio 自身按 [0,1] 钳制：配置层失守（越界/负值）时不产出负延迟或反向抖动。
    pub fn apply_jitter(&self, base_ms: u64, sample01: f64) -> u64 {
        let ratio = self.jitter_ratio.clamp(0.0, 1.0);
        let s = sample01.clamp(0.0, 1.0);
        let mult = 1.0 - ratio + 2.0 * ratio * s;
        (base_ms as f64 * mult).round().max(0.0) as u64
    }
}

/// 重试策略**服务缝**（S10 策略卡 provide、agent-loop 消费；设计 §2 终裁）。
///
/// [`RetryPolicy`] 是参数集与决策算法（主板规格层，P0-7）；本 trait 是它的服务
/// 投影——消费方只认缝不认卡，策略卡（一期 bundled `yunyou-retry`，二期 market）
/// 替换实现而不动消费方。退避执行（sleep 与取消 select）不在此：属消费侧
/// `agent-loop`（设计 §2「策略归调用侧」）。
pub trait RetryPolicyProvider: Send + Sync {
    /// 分类决策（语义与 [`RetryPolicy::classify`] 一致：`attempt` 1 = 首次重试）。
    fn classify(&self, failure: &LlmFailure, attempt: u32) -> RetryAction;

    /// 退避延迟整形（抖动防惊群）：`base_ms` 为 `classify` 产出的退避基数，
    /// `sample01 ∈ [0,1)` 由消费侧提供（一期=系统时间纳秒伪随机，不引 rand 依赖）。
    /// 默认直通（实现方选择不抖动时零成本）。
    fn delay_ms(&self, base_ms: u64, sample01: f64) -> u64 {
        let _ = (base_ms, sample01);
        base_ms
    }
}

impl RetryPolicyProvider for RetryPolicy {
    fn classify(&self, failure: &LlmFailure, attempt: u32) -> RetryAction {
        // 显式调固有方法（固有方法优先于 trait 方法，无递归风险）
        RetryPolicy::classify(self, failure, attempt)
    }

    fn delay_ms(&self, base_ms: u64, sample01: f64) -> u64 {
        self.apply_jitter(base_ms, sample01)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{LlmFailure, LlmFailureCategory::*};

    fn failure(category: crate::error::LlmFailureCategory) -> LlmFailure {
        LlmFailure::new(category, "msg")
    }

    /// §3.1 全类分类：默认白名单下 4 类重试、3 类不重试。
    #[test]
    fn 分类表全类覆盖() {
        let p = RetryPolicy::default();
        for c in [Connection, ServerError, RateLimited, StreamInterrupted] {
            let a = p.classify(&failure(c), 1);
            assert!(a.is_retry(), "{c:?} 应重试");
            assert_eq!(a, RetryAction::Retry { delay_ms: p.backoff_base_ms(1) });
        }
        for c in [ClientError, Timeout, Cancelled] {
            assert!(!p.classify(&failure(c), 1).is_retry(), "{c:?} 不应重试");
        }
    }

    /// 白名单驱动：opt-in timeout 可重试；不在白名单则不重试；空白名单一律 fail。
    #[test]
    fn 白名单驱动() {
        let p = RetryPolicy { retryable_codes: vec!["timeout".into()], ..Default::default() };
        assert!(p.classify(&failure(Timeout), 1).is_retry(), "opt-in timeout 应可重试");
        assert!(!p.classify(&failure(Connection), 1).is_retry(), "不在白名单的 connection 不重试");

        let empty = RetryPolicy { retryable_codes: vec![], ..Default::default() };
        for c in [Connection, ServerError, RateLimited, StreamInterrupted, ClientError, Timeout, Cancelled] {
            assert!(!empty.classify(&failure(c), 1).is_retry(), "空白名单一律 fail");
        }
    }

    /// mode = Never kill switch：一律不重试。
    #[test]
    fn never模式一律不重试() {
        let p = RetryPolicy { mode: RetryMode::Never, ..Default::default() };
        assert!(!p.classify(&failure(Connection), 1).is_retry());
    }

    /// §4 铁律①：取消硬排除——白名单含 cancelled 也不得判 Retry
    /// （取消是用户意志，取消链上不产生新尝试，白名单不得 override）。
    #[test]
    fn 取消硬排除不受白名单影响() {
        let p = RetryPolicy { retryable_codes: vec!["cancelled".into()], ..Default::default() };
        assert!(
            !p.classify(&failure(Cancelled), 1).is_retry(),
            "取消链上不产生新尝试（白名单不得 override）"
        );
    }

    /// 预算：attempt > max_retries（默认 2）→ Fail。
    #[test]
    fn 预算耗尽即fail() {
        let p = RetryPolicy::default();
        assert!(p.classify(&failure(Connection), 1).is_retry(), "首次重试允许");
        assert!(p.classify(&failure(Connection), 2).is_retry(), "第二次重试允许");
        assert!(!p.classify(&failure(Connection), 3).is_retry(), "超过 max_retries=2 不重试");
    }

    /// 退避基数确定性：500→1000→2000→4000→8000（封顶）→8000。
    #[test]
    fn 退避基数确定性() {
        let p = RetryPolicy::default();
        assert_eq!(p.backoff_base_ms(1), 500);
        assert_eq!(p.backoff_base_ms(2), 1000);
        assert_eq!(p.backoff_base_ms(3), 2000);
        assert_eq!(p.backoff_base_ms(4), 4000);
        assert_eq!(p.backoff_base_ms(5), 8000);
        assert_eq!(p.backoff_base_ms(6), 8000, "封顶 max_delay_ms");
        assert_eq!(p.backoff_base_ms(100), 8000, "巨大 attempt 也封顶");
    }

    /// 抖动确定性：sample 0/1 落 [1-ratio, 1+ratio]，0.5 中性；classify 用中性抖动。
    #[test]
    fn 抖动确定性() {
        let p = RetryPolicy { jitter_ratio: 0.1, ..Default::default() };
        let base = 1000;
        assert_eq!(p.apply_jitter(base, 0.5), 1000, "中性不改变基数");
        assert_eq!(p.apply_jitter(base, 0.0), 900, "下界 1-0.1");
        assert_eq!(p.apply_jitter(base, 1.0), 1100, "上界 1+0.1");
        let wild = RetryPolicy { jitter_ratio: 5.0, ..Default::default() };
        assert_eq!(wild.apply_jitter(base, 0.5), 1000, "越界 ratio 钳制后 0.5 仍中性");
        assert_eq!(p.classify(&failure(Connection), 1), RetryAction::Retry { delay_ms: 500 });
    }
}