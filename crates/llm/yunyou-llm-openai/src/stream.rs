//! SSE 字节流的行切分与增量流实现（对接 reqwest 的 chunk 流）。
//!
//! 设计：手动缓冲按行切（不引 eventsource-stream，收敛依赖面）；跨 chunk 的
//! 半行靠内部 buffer 拼接；每行交 wire::parse_sse_line，一块分片的多个增量
//! 进入 pending 队列逐个交出。行切分核心 [`push_and_split`] 为纯函数，可离线单测。

use super::wire::{parse_sse_line, ParsedLine};
use tokio_util::sync::CancellationToken;
use yunyou::Result;
use yunyou_llm::{LlmFailure, LlmStream, StreamDelta};
use std::collections::VecDeque;

/// 把新到的字节并入缓冲，切出其中全部完整行（不含换行符）。
///
/// 半行残留在 `buffer` 里等待下一次调用；EOF 后的残余（无换行的最后一行）
/// 由 [`OpenAiStream::take_residual`] 收走。传空切片即「只切现有缓冲」。
pub(crate) fn push_and_split(buffer: &mut Vec<u8>, bytes: &[u8]) -> Vec<String> {
    buffer.extend_from_slice(bytes);
    let mut lines = Vec::new();
    while let Some(pos) = buffer.iter().position(|&byte| byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=pos).collect();
        lines.push(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
    }
    lines
}

/// OpenAI 流式响应的增量流（实现 [`LlmStream`]）。
pub struct OpenAiStream {
    response: reqwest::Response,
    /// 跨 chunk 的半行缓冲。
    buffer: Vec<u8>,
    /// 底层流是否已到 EOF。
    finished: bool,
    /// 已切出、待解析的 SSE 行。
    line_queue: VecDeque<String>,
    /// 一块 JSON 分片切出的多个增量（FIFO）。
    pending: VecDeque<StreamDelta>,
    /// 是否已见 [DONE] 哨兵（task-49①）：EOF 且未见过哨兵=SSE 断流（连接
    /// 中途被切），静默当正常收束即「回合尾部无声失踪」的真根之一——诚实
    /// 归 StreamInterrupted（可重试），绝不装死。
    done_seen: bool,
    /// 已收增量计数（断流报错携带诊断量：业主排查「收到多少就断了」）。
    received_deltas: usize,
    /// 取消信号（R2）：SSE 读循环 select! 它，触发即断流归为取消失败。
    cancel: CancellationToken,
    /// 空闲看门狗预算（D-19-1）：chunk 间无字节超过该时长即判流死（Timeout）。
    /// `None` = 不设看门狗（不限时直连）。每次 select 重置计时=只量「字节间
    /// 间隙」，总时长不限——推理模型 thinking 可以任意久，只要有字节在流。
    idle_timeout: Option<std::time::Duration>,
}

impl OpenAiStream {
    pub(crate) fn new(
        response: reqwest::Response,
        cancel: CancellationToken,
        idle_timeout: Option<std::time::Duration>,
    ) -> Self {
        Self {
            response,
            buffer: Vec::new(),
            finished: false,
            line_queue: VecDeque::new(),
            pending: VecDeque::new(),
            done_seen: false,
            received_deltas: 0,
            cancel,
            idle_timeout,
        }
    }

    /// 取下一条 SSE 行；EOF 且无残余 → `None`。
    async fn next_line(&mut self) -> Result<Option<String>> {
        loop {
            if let Some(line) = self.line_queue.pop_front() {
                return Ok(Some(line));
            }
            if self.finished {
                return Ok(self.take_residual());
            }
            tokio::select! {
                chunk = self.response.chunk() => {
                    let chunk = chunk.map_err(|err| {
                        // 连接级超时（reqwest 层）归 Timeout（默认不重试），
                        // 真断连/协议错才归 StreamInterrupted——两型不混
                        //（任务18 §4 铁律②）。
                        if err.is_timeout() {
                            LlmFailure::timeout(format!("读取流式响应超时: {err}"))
                        } else {
                            LlmFailure::stream_interrupted(format!("读取流式响应失败: {err}"))
                        }
                        .into_cordis()
                    })?;
                    match chunk {
                        Some(bytes) => self.line_queue.extend(push_and_split(&mut self.buffer, &bytes)),
                        None => self.finished = true,
                    }
                }
                // D-19-1 空闲看门狗：chunk 间无字节超预算=真死链（总预算语义
                // 已废——推理时长不可预算，只量「字节间间隙」，每轮 select 重置）。
                _ = Self::idle_elapsed(self.idle_timeout), if self.idle_timeout.is_some() => {
                    let ms = self.idle_timeout.map(|d| d.as_millis()).unwrap_or_default();
                    return Err(LlmFailure::timeout(format!(
                        "流式响应空闲超时（{ms}ms 内无任何字节，判流死）"
                    )).into_cordis());
                }
                _ = self.cancel.cancelled() => {
                    return Err(LlmFailure::cancelled().into_cordis());
                }
            }
        }
    }

    /// EOF 后收走无换行符的残余行。
    fn take_residual(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            None
        } else {
            let rest = std::mem::take(&mut self.buffer);
            Some(String::from_utf8_lossy(&rest).into_owned())
        }
    }

    /// 空闲看门狗计时（select 分支用）：`None` 永不触发（双保险——分支
    /// precondition 已挡，future 侧也挂起防误触）。
    async fn idle_elapsed(timeout: Option<std::time::Duration>) {
        match timeout {
            Some(d) => tokio::time::sleep(d).await,
            None => std::future::pending::<()>().await,
        }
    }
}

impl LlmStream for OpenAiStream {
    fn next(&mut self) -> yunyou_util::BoxFut<'_, Result<Option<StreamDelta>>> {
        Box::pin(async move {
            loop {
                if let Some(delta) = self.pending.pop_front() {
                    return Ok(Some(delta));
                }
                let Some(line) = self.next_line().await? else {
                    // EOF 收束判定（task-49①）：见过 [DONE]=正常收尾；未见过=
                    // SSE 断流（连接中途被切、网关截断等）——静默 Ok(None) 会让
                    // 回合以空/半截回复「正常」落盘（业主实证的无声失踪），诚实
                    // 归 StreamInterrupted（默认可重试）。
                    if self.done_seen {
                        return Ok(None);
                    }
                    return Err(LlmFailure::stream_interrupted(format!(
                        "流式响应未以完成哨兵收束即结束（已收 {} 个增量），疑似连接中断",
                        self.received_deltas
                    ))
                    .into_cordis());
                };
                match parse_sse_line(&line)
                    .map_err(|err| LlmFailure::stream_interrupted(err.message().to_string()).into_cordis())?
                {
                    ParsedLine::Skip => continue,
                    // 哨兵行：先交出残余增量（通常为空），下一次调用读到 EOF 后正常收束
                    ParsedLine::Done => {
                        self.done_seen = true;
                        return Ok(self.pending.pop_front());
                    }
                    ParsedLine::Deltas(deltas) => {
                        self.received_deltas += deltas.len();
                        self.pending.extend(deltas);
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 行切分：跨 chunk 半行拼接、多行一次切出、残余保留。
    #[test]
    fn 行切分的跨chunk拼接() {
        let mut buffer = Vec::new();
        assert_eq!(push_and_split(&mut buffer, b"data: {\"ch"), Vec::<String>::new());
        assert_eq!(
            push_and_split(&mut buffer, b"oices\":[]}\n\ndata: [DONE]\n"),
            vec!["data: {\"choices\":[]}", "", "data: [DONE]"]
        );
        assert!(buffer.is_empty());
        // EOF 残余（无换行的最后一行）
        assert_eq!(push_and_split(&mut buffer, b"data: tail"), Vec::<String>::new());
        assert_eq!(buffer, b"data: tail");
    }
}
