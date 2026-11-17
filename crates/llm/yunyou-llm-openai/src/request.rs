//! 请求体序列化（OpenAI Chat Completions wire 格式，纯函数可离线快照测试）。
//!
//! # 契约
//! - [`build_request_body`] 是**实际发出的请求体的唯一来源**：stream_inner 与
//!   快照测试共用同一函数，杜绝「测试测的序列化 ≠ 发出去的序列化」的盲区；
//! - 翻译规则（缺一即 400，live 联调实录 2026-11-13：missing field `type`）：
//!   ① `tools[]` → `{"type":"function","function":{name,description,parameters}}`；
//!   ② assistant 消息的 `tool_calls[]` → `{"id","type":"function","function":{name,arguments}}`；
//!   ③ 工具结果消息 → `{"role":"tool","tool_call_id","content"}`；
//!   ④ 思考内容（reasoning）**不回传端点**；assistant 无工具调用时发纯文本消息；
//!   ⑤ assistant 有工具调用且文本为空时 `content` 发 `null`（严格端点兼容）。

use yunyou_llm::{GenerateRequest, Message, Role};
use serde_json::{json, Value};

/// 构造 `/chat/completions` 请求体。
///
/// # 参数
/// - `default_model`：配置的默认模型；`req.model` 为空时回退。
pub(crate) fn build_request_body(default_model: &str, req: &GenerateRequest) -> Value {
    let model = if req.model.is_empty() { default_model.to_string() } else { req.model.clone() };
    // temperature（task-33）：None=不传该字段（端点默认值，请求体与现状逐
    // 字节一致）；Some(t)=显式映射（压缩摘要路径传 0.0——忠实不创造）。
    let mut body = json!({
        "model": model,
        "messages": req.messages.iter().map(message_to_wire).collect::<Vec<_>>(),
        "tools": req.tools.iter().map(tool_to_wire).collect::<Vec<_>>(),
        "stream": true,
        // 工具调用由流分片聚合，端点侧不做增量截断；带上 usage 便于计费观测
        "stream_options": { "include_usage": true },
    });
    if let Some(temperature) = req.temperature {
        body["temperature"] = json!(temperature);
    }
    body
}

/// 单条消息 → OpenAI wire 格式（规则见模块契约 ②③④⑤）。
pub(crate) fn message_to_wire(message: &Message) -> Value {
    match message.role {
        Role::Assistant if !message.tool_calls.is_empty() => {
            let content = if message.content.is_empty() {
                Value::Null
            } else {
                Value::String(message.content.clone())
            };
            json!({
                "role": "assistant",
                "content": content,
                "tool_calls": message
                    .tool_calls
                    .iter()
                    .map(|call| json!({
                        "id": call.id,
                        "type": "function",
                        "function": { "name": call.name, "arguments": call.arguments },
                    }))
                    .collect::<Vec<_>>(),
            })
        }
        Role::Tool => json!({
            "role": "tool",
            "tool_call_id": message.tool_call_id,
            "content": message.content,
        }),
        ref role => json!({ "role": role, "content": message.content }),
    }
}

/// 工具声明 → OpenAI wire 格式（规则见模块契约 ①）。
pub(crate) fn tool_to_wire(schema: &yunyou_llm::ToolSchema) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": schema.name,
            "description": schema.description,
            "parameters": schema.parameters,
        },
    })
}

#[cfg(test)]
mod tests {

    /// task-33：temperature 映射——Some(t)=显式入端点 JSON；None=键不出现
    /// （请求体与现状逐字节一致——加法演进默认行为锁）。
    #[test]
    fn temperature_映射与省略() {
        let mut req = GenerateRequest {
            model: "m".into(),
            messages: vec![],
            tools: vec![],
            temperature: Some(0.0),
        };
        let body = build_request_body("fallback", &req);
        assert_eq!(body["temperature"], serde_json::json!(0.0), "Some 显式映射");
        req.temperature = None;
        let body = build_request_body("fallback", &req);
        assert!(
            body.get("temperature").is_none(),
            "None 不得出现该键（逐字节一致）: {body}"
        );
    }
    use super::*;
    use yunyou_llm::{ToolCallRequest, ToolSchema};

    /// 请求体快照（golden）：含工具声明的完整请求体逐字段断言。
    /// 背景：live 联调实测端点要求 tools 项含 "type":"function"（缺失即 400）。
    #[test]
    fn 含工具的请求体快照() {
        let req = GenerateRequest {
            model: "glm-5.3-flash".into(),
            temperature: None,
            messages: vec![
                Message::text(Role::System, "你是助手"),
                Message::text(Role::User, "现在几点"),
                Message {
                    role: Role::Assistant,
                    content: String::new(),
                    tool_calls: vec![ToolCallRequest {
                        id: "call-1".into(),
                        name: "bash".into(),
                        arguments: "{\"cmd\":\"date\"}".into(),
                    }],
                    tool_call_id: None,
                    reasoning: Some("内部思考，绝不外发".into()),
                },
                Message::tool_result("call-1", "2026-11-13"),
            ],
            tools: vec![ToolSchema {
                name: "bash".into(),
                description: "执行命令".into(),
                parameters: serde_json::json!({"type":"object","properties":{"cmd":{"type":"string"}}}),
            }],
        };
        let body = build_request_body("fallback-model", &req);
        let body_text = serde_json::to_string_pretty(&body).unwrap();
        eprintln!("[请求体 dump]\n{body_text}");

        // ① 顶层字段
        assert_eq!(body["model"], "glm-5.3-flash");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);

        // ② 工具声明：type:"function" + function 三件套包装（本次 400 的缺失处）
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "function", "tools 项必须含 type:function（live 400 根因）");
        assert_eq!(tool["function"]["name"], "bash");
        assert_eq!(tool["function"]["description"], "执行命令");
        assert_eq!(tool["function"]["parameters"]["type"], "object");

        // ③ assistant + tool_calls：type:"function" + function 包装；空文本发 null
        let assistant = &body["messages"][2];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["content"], Value::Null, "纯工具调用消息的空文本应发 null");
        let call = &assistant["tool_calls"][0];
        assert_eq!(call["id"], "call-1");
        assert_eq!(call["type"], "function", "tool_calls 项必须含 type:function");
        assert_eq!(call["function"]["name"], "bash");
        assert_eq!(call["function"]["arguments"], "{\"cmd\":\"date\"}");

        // ④ 思考内容绝不外发（容忍清单：reasoning 仅落盘）
        assert!(
            !body_text.contains("内部思考"),
            "reasoning 不得出现在请求体中"
        );

        // ⑤ 工具结果消息：role=tool + tool_call_id
        let tool_result = &body["messages"][3];
        assert_eq!(tool_result["role"], "tool");
        assert_eq!(tool_result["tool_call_id"], "call-1");
        assert_eq!(tool_result["content"], "2026-11-13");

        // ⑥ 普通文本消息（system/user）保持纯字符串 content
        assert_eq!(body["messages"][0]["content"], "你是助手");
        assert_eq!(body["messages"][1]["content"], "现在几点");
    }

    /// 无工具时 tools 为空数组（不省略字段——端点按存在性判断是否启用工具）。
    #[test]
    fn 无工具时tools为空数组() {
        let req = GenerateRequest {
            model: String::new(),
            messages: vec![Message::text(Role::User, "hi")],
            tools: vec![],
            temperature: None,
        };
        let body = build_request_body("fallback-model", &req);
        assert_eq!(body["model"], "fallback-model", "空模型回退配置默认值");
        assert_eq!(body["tools"].as_array().map(Vec::len), Some(0));
        assert_eq!(body["messages"][0]["role"], "user");
    }
}
