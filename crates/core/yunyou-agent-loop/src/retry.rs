//! stream_once 的重试包装（S10 消费点，任务18；《超时重试插件设计》§1.3/§2/§3.2）。
//!
//! 职责切分（设计 §2「策略归调用侧」）：策略卡（yunyou-retry → RetryPolicyProvider）
//! 只产出**无量决策**（Retry{delay} | Fail）；本模块管退避执行——sleep 与取消信号
//! `select!` 竞争（§4 铁律③：退避等待可被取消打断，取消即断链）。
//!
//! # 三条铁律的落点（设计 §4）
//! ① 取消优先于一切决策：循环头 `is_cancelled` 直通检查 + classify 对 Cancelled
//!    恒 Fail（retry_policy.rs）双闸；取消链上不产生新尝试。
//! ② 超时独立类别：失败经 `LlmFailure::from_cordis` 还原（`llm/<code>:` 前缀），
//!    非 llm 域错误（会话写失败等）还原为 None → 不重试直出，不与网络错误混判。
//! ③ 重试计数与超时预算独立：退避 sleep 不挤占 timeout_secs（各自计时）。
//!
//! # 可观测（设计 §3.2，裁决 D10：observer/日志层，不落 JSONL）
//! 每次重试调度发 [`TurnObserver::on_retry_scheduled`]（默认 no-op，加法演进）
//! + tracing 日志双轨。

use crate::aggregate::Aggregated;
use crate::turn::AgentLoopRunner;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use yunyou::{CordisError, ErrorKind};
use yunyou_llm::{GenerateRequest, LlmFailure, RetryAction};

/// 一次模型调用的重试包装：失败 → 还原 [`LlmFailure`] → 策略分类 →
/// [`RetryAction::Retry`] 则抖动退避（可与取消竞争）后重发整次调用。
///
/// `request` 为重发单元（流中途失败=本轮无产出，重发幂等，设计 §3.1 流中断行）。
pub(crate) async fn stream_once_with_retry(
    runner: &AgentLoopRunner,
    request: GenerateRequest,
    cancel: &CancellationToken,
    observer: &Option<Arc<dyn crate::observer::TurnObserver>>,
) -> Result<Aggregated, CordisError> {
    let mut attempt: u32 = 0;
    loop {
        // 铁律①第一闸：取消链上不发起任何尝试（classify 前先查）
        if cancel.is_cancelled() {
            return Err(CordisError::new(ErrorKind::Internal, "loop/cancelled"));
        }
        attempt += 1;
        match runner.stream_once(request.clone(), cancel, observer).await {
            Ok(aggregated) => return Ok(aggregated),
            Err(err) => {
                let Some(policy) = runner.retry.as_ref() else { return Err(err) };
                // 非 llm 域错误（会话写失败/取消态等）不在还原面 → 直出
                let Some(failure) = LlmFailure::from_cordis(&err) else { return Err(err) };
                match policy.classify(&failure, attempt) {
                    RetryAction::Retry { delay_ms } => {
                        let cause = failure.code();
                        if let Some(obs) = observer {
                            obs.on_retry_scheduled(attempt, delay_ms, cause).await;
                        }
                        let delay = policy.delay_ms(delay_ms, jitter_sample());
                        tracing::info!(attempt, delay_ms = delay, cause, "llm/重试调度");
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
                            // 铁律③：退避等待与取消竞争，取消即中止整个重试链
                            _ = cancel.cancelled() => {
                                return Err(CordisError::new(ErrorKind::Internal, "loop/cancelled"));
                            }
                        }
                    }
                    RetryAction::Fail => return Err(err),
                }
            }
        }
    }
}

/// 抖动随机源（一期）：系统时间纳秒低位 → [0,1) 伪随机。
/// 不引 rand 依赖（收敛依赖面）；抖动只防惊群，不承担随机性安全语义。
fn jitter_sample() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    f64::from(nanos % 1000) / 1000.0
}
