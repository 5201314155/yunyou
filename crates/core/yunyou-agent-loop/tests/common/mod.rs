//! 集成测试共享基建：装配轮询 helper、脚本化 FakeLlm、标准脚本（各测试文件共用）。
#![allow(dead_code)]

use yunyou::{Context, ServiceId};
use yunyou_llm::{GenerateRequest, Llm, LlmStream, StreamDelta, ToolCallDelta, Usage};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yunyou_util::BoxFut;

/// 等待插件 fiber 到 Active（await_ready 对 Pending 稳定态会立即 Ok，装配断言必须轮询）。
pub async fn wait_active(handle: &yunyou::FiberHandle, name: &str) {
    for _ in 0..400 {
        if handle.state() == yunyou::FiberState::Active {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("插件 {name} 未能在限时内装载到 Active，当前 {:?}", handle.state());
}

/// 任务29 C4：ToolsPlugin 声明 `inject: dyn CommandsService`（/tools-info 注册
/// 依赖）——epoch 机制下 commands 未装载则 tools fiber 恒 Pending。测试装配在
/// tools 之前调用本 helper；已就绪则跳过（幂等，同 root 重复装载会 ServiceDuplicate）。
pub async fn ensure_commands(root: &Context) {
    if root
        .get::<dyn yunyou_commands::CommandsService>(ServiceId::of::<
            dyn yunyou_commands::CommandsService,
        >())
        .is_ok()
    {
        return;
    }
    let handle = root.plugin(yunyou_commands::CommandsPlugin, serde_json::json!({}));
    wait_active(&handle, "commands").await;
}

/// 脚本便捷构造：第一轮 bash 工具调用（分两片聚合），第二轮纯文本收尾。
pub fn tool_then_answer_script() -> Vec<Vec<StreamDelta>> {
    vec![
        vec![
            StreamDelta::Reasoning("先想好用哪个命令。".into()),
            StreamDelta::Text("我来跑个命令。".into()),
            StreamDelta::ToolCall(ToolCallDelta {
                index: 0,
                id: Some("call-1".into()),
                name: Some("bash".into()),
                arguments_delta: "{\"cmd\":\"echo".into(),
            }),
            StreamDelta::ToolCall(ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments_delta: " hello-from-tool\"}".into(),
            }),
            StreamDelta::Done {
                finish_reason: Some("tool_calls".into()),
                usage: Some(Usage { prompt_tokens: 10, completion_tokens: 5 }),
            },
        ],
        vec![
            StreamDelta::Text("工具输出是 hello-from-tool。".into()),
            StreamDelta::Done {
                finish_reason: Some("stop".into()),
                usage: Some(Usage { prompt_tokens: 7, completion_tokens: 3 }),
            },
        ],
    ]
}

/// FakeLlm：按脚本队列吐增量（每次调用弹出一段；耗尽回退提示文本防空回复）。
#[derive(Clone)]
pub struct FakeLlm {
    scripts: Arc<Mutex<Vec<Vec<StreamDelta>>>>,
}

impl FakeLlm {
    pub fn new(scripts: Vec<Vec<StreamDelta>>) -> Self {
        Self { scripts: Arc::new(Mutex::new(scripts)) }
    }
}

/// FlakyLlm 的单次调用结局（S10 重试测试用）。
pub enum FlakyOutcome {
    /// 本次调用直接失败（按 LLM 失败类别，into_cordis 归一化 "llm/<code>:"）。
    Fail(yunyou_llm::LlmFailure),
    /// 本次调用成功，按脚本吐增量。
    Script(Vec<StreamDelta>),
}

/// FlakyLlm：按结局队列逐次返回失败/成功（S10 消费点测试；耗尽回退成功空脚本）。
#[derive(Clone)]
pub struct FlakyLlm {
    outcomes: Arc<Mutex<Vec<FlakyOutcome>>>,
    /// 实际发起的调用计数（断言「重试了几次」的铁证）。
    pub calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl FlakyLlm {
    pub fn new(outcomes: Vec<FlakyOutcome>) -> Self {
        Self {
            outcomes: Arc::new(Mutex::new(outcomes)),
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

impl Llm for FlakyLlm {
    fn stream(&self, _req: GenerateRequest) -> BoxFut<'static, yunyou::Result<Box<dyn LlmStream>>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let outcome = {
            let mut queue = self.outcomes.lock().unwrap();
            if queue.is_empty() {
                FlakyOutcome::Script(vec![
                    StreamDelta::Text("(队列已耗尽)".into()),
                    StreamDelta::Done { finish_reason: Some("stop".into()), usage: None },
                ])
            } else {
                queue.remove(0)
            }
        };
        Box::pin(async move {
            match outcome {
                FlakyOutcome::Fail(failure) => Err(failure.into_cordis()),
                FlakyOutcome::Script(script) => {
                    Ok(Box::new(FakeStream { script, index: 0 }) as Box<dyn LlmStream>)
                }
            }
        })
    }
}

pub struct FakeStream {
    script: Vec<StreamDelta>,
    index: usize,
}

impl FakeStream {
    /// 测试侧自建脚本流（resilience.rs RecordingLlm 委托用；字段私有仅暴露构造）。
    pub fn new(script: Vec<StreamDelta>) -> Self {
        Self { script, index: 0 }
    }
}

impl LlmStream for FakeStream {
    fn next(&mut self) -> BoxFut<'_, yunyou::Result<Option<StreamDelta>>> {
        Box::pin(async move {
            let index = self.index;
            self.index += 1;
            Ok(self.script.get(index).cloned())
        })
    }
}

impl Llm for FakeLlm {
    fn stream(&self, _req: GenerateRequest) -> BoxFut<'static, yunyou::Result<Box<dyn LlmStream>>> {
        let script = {
            let mut queue = self.scripts.lock().unwrap();
            if queue.is_empty() {
                vec![
                    StreamDelta::Text("(脚本已耗尽)".into()),
                    StreamDelta::Done { finish_reason: Some("stop".into()), usage: None },
                ]
            } else {
                queue.remove(0)
            }
        };
        Box::pin(async move { Ok(Box::new(FakeStream { script, index: 0 }) as Box<dyn LlmStream>) })
    }
}

/// FakeLlm 插件（脚本注入，与真实 Provider 同一装配路径）。
pub struct FakeLlmPlugin {
    pub llm: Arc<dyn Llm>,
}

impl yunyou::Plugin for FakeLlmPlugin {
    fn name(&self) -> &str {
        "fake-llm"
    }
    fn setup(
        self: Arc<Self>,
        ctx: Context,
        _config: serde_json::Value,
    ) -> BoxFut<'static, yunyou::Result<yunyou::FiberOutput>> {
        Box::pin(async move {
            ctx.provide(ServiceId::of::<dyn Llm>(), self.llm.clone())?;
            Ok(yunyou::FiberOutput)
        })
    }
}

/// 挂起流 LLM 插件：`next` 永不完成（模拟模型长思考/慢首包），
/// 只在取消传播时收流。取消类测试（调用方线 / fiber 卸载线）共用。
pub struct HangingLlmPlugin;

impl yunyou::Plugin for HangingLlmPlugin {
    fn name(&self) -> &str {
        "hanging-llm"
    }
    fn setup(
        self: Arc<Self>,
        ctx: Context,
        _config: serde_json::Value,
    ) -> BoxFut<'static, yunyou::Result<yunyou::FiberOutput>> {
        Box::pin(async move {
            struct HangingStream;
            impl LlmStream for HangingStream {
                fn next(&mut self) -> BoxFut<'_, yunyou::Result<Option<StreamDelta>>> {
                    Box::pin(async move { std::future::pending().await })
                }
            }
            struct HangingLlm;
            impl Llm for HangingLlm {
                fn stream(
                    &self,
                    _req: GenerateRequest,
                ) -> BoxFut<'static, yunyou::Result<Box<dyn LlmStream>>> {
                    Box::pin(async move { Ok(Box::new(HangingStream) as Box<dyn LlmStream>) })
                }
            }
            let llm: Arc<dyn Llm> = Arc::new(HangingLlm);
            ctx.provide(ServiceId::of::<dyn Llm>(), llm)?;
            Ok(yunyou::FiberOutput)
        })
    }
}
