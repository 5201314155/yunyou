//! OpenAI Chat Completions 线协议类型与 SSE 行解析（纯函数，可离线单测）。
//!
//! 解析器是 Provider 的正确性核心：端点实测行为（reasoning_content / content:null /
//! 空 choices）全部在此容忍，见 crate 文档的容忍清单。

use super::llm_error;
use yunyou::{CordisError, Result};
use yunyou_llm::{StreamDelta, ToolCallDelta, Usage};

/// 一条 SSE data 行的解析结果。
pub enum ParsedLine {
    /// 流结束哨兵（`data: [DONE]`）。
    Done,
    /// 忽略行（空行 / 非 data 行 / 注释）。
    Skip,
    /// 解析出一个或多个增量（一块 JSON 可能同时含文本/思考/工具分片）。
    Deltas(Vec<StreamDelta>),
}

/// 解析一条 SSE 行（不含行尾换行）。
///
/// # 失败语义
/// `data:` 行的 JSON 非法 → `Err`（协议错误，调用方应终止流）。
pub fn parse_sse_line(line: &str) -> Result<ParsedLine> {
    let line = line.trim_start_matches('\u{feff}').trim_end_matches('\r');
    if line.is_empty() || !line.starts_with("data:") {
        return Ok(ParsedLine::Skip);
    }
    let payload = line["data:".len()..].trim();
    if payload == "[DONE]" {
        return Ok(ParsedLine::Done);
    }
    let chunk: ChatChunk = serde_json::from_str(payload)
        .map_err(|err| llm_error("sse", format!("流式分片解析失败: {err}")))?;
    Ok(ParsedLine::Deltas(chunk.into_deltas()))
}

/// `/chat/completions` 流式分片（只建模一期用到的字段；未知字段 serde 忽略）。
#[derive(Debug, serde::Deserialize)]
struct ChatChunk {
    /// 可能为空数组（仅 usage 的终止块），故默认空。
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, serde::Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

/// 增量体。**容忍清单的核心**：`content` 可为 null（缺省 None），
/// `reasoning_content` 为推理端点扩展字段（Option + 默认空）。
#[derive(Debug, Default, serde::Deserialize)]
struct Delta {
    /// 可为 null（纯工具/纯思考分片），null 与缺省等价处理。
    #[serde(default)]
    content: Option<String>,
    /// DeepSeek-R1 等推理模型的思考增量字段。
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallWire>,
}

#[derive(Debug, serde::Deserialize)]
struct ToolCallWire {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ToolFunctionWire>,
}

#[derive(Debug, serde::Deserialize)]
struct ToolFunctionWire {
    #[serde(default)]
    name: Option<String>,
    /// 参数 JSON 文本分片（可缺省）。
    #[serde(default)]
    arguments: Option<String>,
}

impl ChatChunk {
    /// 把一块分片展开为增量序列（文本 → 思考 → 工具分片 → 终止）。
    fn into_deltas(self) -> Vec<StreamDelta> {
        let mut out = Vec::new();
        for choice in &self.choices {
            // content 为 null 或缺失：不产出任何 Text（容忍清单第 2 条）
            if let Some(text) = choice.delta.content.as_deref() {
                if !text.is_empty() {
                    out.push(StreamDelta::Text(text.to_string()));
                }
            }
            if let Some(reasoning) = choice.delta.reasoning_content.as_deref() {
                if !reasoning.is_empty() {
                    out.push(StreamDelta::Reasoning(reasoning.to_string()));
                }
            }
            for call in &choice.delta.tool_calls {
                let function = call.function.as_ref();
                out.push(StreamDelta::ToolCall(ToolCallDelta {
                    index: call.index,
                    id: call.id.clone(),
                    name: function.and_then(|f| f.name.clone()),
                    arguments_delta: function.and_then(|f| f.arguments.clone()).unwrap_or_default(),
                }));
            }
            if choice.finish_reason.is_some() || self.usage.is_some() {
                out.push(StreamDelta::Done {
                    finish_reason: choice.finish_reason.clone(),
                    usage: self.usage,
                });
            }
        }
        // 空 choices + 仅 usage 的终止块：仍要产出 Done（容忍清单第 3 条）
        if self.choices.is_empty() {
            if let Some(usage) = self.usage {
                out.push(StreamDelta::Done { finish_reason: None, usage: Some(usage) });
            }
        }
        out
    }
}

/// 解析非流式错误响应体（HTTP 非 2xx 时调用），尽量提取 message。
pub fn parse_error_body(body: &str) -> CordisError {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|err| err.get("message"))
                .and_then(|msg| msg.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.chars().take(200).collect());
    llm_error("http", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 容忍清单 1+2：reasoning_content 透传；content:null 不产空文本。
    #[test]
    fn 推理端点的_null_content与思考增量() {
        let line = r#"data: {"choices":[{"delta":{"content":null,"reasoning_content":"思考中"}}]}"#;
        match parse_sse_line(line).unwrap() {
            ParsedLine::Deltas(deltas) => {
                assert_eq!(deltas.len(), 1);
                assert!(matches!(&deltas[0], StreamDelta::Reasoning(text) if text == "思考中"));
            }
            _ => panic!("应产出增量"),
        }
    }

    /// 容忍清单 3：空 choices + 仅 usage → Done。
    #[test]
    fn 仅usage的终止块() {
        let line = r#"data: {"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        match parse_sse_line(line).unwrap() {
            ParsedLine::Deltas(deltas) => match &deltas[..] {
                [StreamDelta::Done { finish_reason, usage }] => {
                    assert!(finish_reason.is_none());
                    assert_eq!(usage.unwrap().completion_tokens, 5);
                }
                other => panic!("应为单个 Done，实得 {other:?}"),
            },
            _ => panic!("应产出增量"),
        }
    }

    /// 工具调用分片聚合所需字段完整透传。
    #[test]
    fn 工具调用分片() {
        let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"bash","arguments":"{\"cmd\""}}]},"finish_reason":null}]}"#;
        match parse_sse_line(line).unwrap() {
            ParsedLine::Deltas(deltas) => match &deltas[..] {
                [StreamDelta::ToolCall(call)] => {
                    assert_eq!(call.index, 0);
                    assert_eq!(call.id.as_deref(), Some("call_1"));
                    assert_eq!(call.name.as_deref(), Some("bash"));
                    assert_eq!(call.arguments_delta, "{\"cmd\"");
                }
                other => panic!("应为工具分片，实得 {other:?}"),
            },
            _ => panic!("应产出增量"),
        }
    }

    /// 哨兵、空行、非 data 行、BOM、非法 JSON 的处理。
    #[test]
    fn 哨兵与杂行() {
        assert!(matches!(parse_sse_line("data: [DONE]").unwrap(), ParsedLine::Done));
        assert!(matches!(parse_sse_line("").unwrap(), ParsedLine::Skip));
        assert!(matches!(parse_sse_line(": keep-alive").unwrap(), ParsedLine::Skip));
        assert!(matches!(parse_sse_line("event: ping").unwrap(), ParsedLine::Skip));
        assert!(matches!(parse_sse_line("\u{feff}data: [DONE]").unwrap(), ParsedLine::Done));
        assert!(parse_sse_line("data: {非法").is_err(), "非法 JSON 应报协议错误");
    }

    /// 非流式错误体提取 message。
    #[test]
    fn 错误体提取() {
        let err = parse_error_body(r#"{"error":{"message":"余额不足","type":"insufficient"}}"#);
        assert!(err.message().contains("余额不足"));
        let fallback = parse_error_body("not-json");
        assert!(fallback.message().contains("not-json"));
    }
}
