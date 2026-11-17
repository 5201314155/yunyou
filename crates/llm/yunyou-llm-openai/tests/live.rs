//! 真实对话集成测试（验收标准 2）：env `DSH_TEST_LLM_BASE_URL` /
//! `DSH_TEST_LLM_API_KEY` / `DSH_TEST_LLM_MODEL` 由 lead 注入；**env 缺失时跳过
//! 不失败**；密钥绝不写进代码/测试/git。
//!
//! 验证路径：内核装配 OpenAiPlugin → 真实端点流式一轮 → 校验文本/思考增量
//! 与终止块形态（含 reasoning_content / content:null 容忍路径的线上形态）。

use tokio_util::sync::CancellationToken;
use yunyou::{Context, ServiceId};
use yunyou_llm::{GenerateRequest, Llm, Message, Role, StreamDelta};
use yunyou_llm_openai::OpenAiPlugin;

/// 集成测试入口：env 缺失直接跳过。
#[tokio::test(flavor = "multi_thread")]
async fn 真实对话一轮() {
    let Some(base_url) = std::env::var("DSH_TEST_LLM_BASE_URL").ok().filter(|v| !v.is_empty())
    else {
        eprintln!("SKIP: 未设置 DSH_TEST_LLM_BASE_URL，跳过真实对话集成测试");
        return;
    };
    let api_key = std::env::var("DSH_TEST_LLM_API_KEY").unwrap_or_default();
    let model = std::env::var("DSH_TEST_LLM_MODEL").unwrap_or_else(|_| "deepseek-chat".into());

    let root = Context::new();
    let config = serde_json::json!({ "base_url": base_url, "api_key": api_key, "model": model });
    let handle = root.plugin(OpenAiPlugin, config);
    handle.await_ready().await.expect("OpenAI Provider 应装载成功");
    let llm = root
        .get::<dyn Llm>(ServiceId::of::<dyn Llm>())
        .expect("Llm 服务应已注册");

    let request = GenerateRequest {
        model: model.clone(),
        messages: vec![Message::text(Role::User, "用一句中文回答：1+1 等于几？")],
        tools: Vec::new(),
        temperature: None,
    };
    let mut stream = llm.stream_cancellable(request, CancellationToken::new()).await.expect("流应建立");
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut saw_done = false;
    while let Some(delta) = stream.next().await.expect("流不应中途失败") {
        match delta {
            StreamDelta::Text(piece) => text.push_str(&piece),
            StreamDelta::Reasoning(piece) => reasoning.push_str(&piece),
            StreamDelta::Done { .. } => saw_done = true,
            StreamDelta::ToolCall(_) => panic!("本用例未声明工具，不应有工具调用"),
        }
    }
    assert!(saw_done, "流应产出终止块");
    assert!(!text.trim().is_empty(), "应产出可见文本");
    eprintln!("模型回复: {text}（思考 {len} 字符）", len = reasoning.chars().count());
}

/// 声明工具的真实调用链路（可选：同样受 env 开关控制）。
#[tokio::test(flavor = "multi_thread")]
async fn 真实工具调用声明() {
    let Some(base_url) = std::env::var("DSH_TEST_LLM_BASE_URL").ok().filter(|v| !v.is_empty())
    else {
        eprintln!("SKIP: 未设置 DSH_TEST_LLM_BASE_URL，跳过真实工具调用集成测试");
        return;
    };
    let api_key = std::env::var("DSH_TEST_LLM_API_KEY").unwrap_or_default();
    let model = std::env::var("DSH_TEST_LLM_MODEL").unwrap_or_else(|_| "deepseek-chat".into());

    let root = Context::new();
    let config = serde_json::json!({ "base_url": base_url, "api_key": api_key, "model": model });
    root.plugin(OpenAiPlugin, config).await_ready().await.expect("应装载");
    let llm = root.get::<dyn Llm>(ServiceId::of::<dyn Llm>()).expect("服务应可见");

    let request = GenerateRequest {
        model,
        messages: vec![Message::text(Role::User, "请调用 get_time 工具查询当前时间。")],
        temperature: None,
        tools: vec![yunyou_llm::ToolSchema {
            name: "get_time".into(),
            description: "获取当前时间".into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }],
    };
    let mut stream = llm.stream_cancellable(request, CancellationToken::new()).await.expect("流应建立");
    let mut saw_tool_call = false;
    while let Some(delta) = stream.next().await.expect("流不应中途失败") {
        if matches!(delta, StreamDelta::ToolCall(_)) {
            saw_tool_call = true;
        }
    }
    eprintln!("端点是否发起工具调用: {saw_tool_call}（信息性断言，不同模型行为可能不同）");
}
