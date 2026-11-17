//! 工具循环 IO 次数回归（task-15，挑刺 P1-4；复核人=云驰）。
//!
//! 判别力锚：修后一回合 load_all **恰 1 次**（循环外单次载入，循环内账本化）——
//! 比卡面 O(N×S)→O(S) 口径更严（S=1），未来任何一步重读回潮即红；改断言须留痕。
//! 修前基线（757d5fe^ 同测试）：16 步回合 load_all=16 → 红（判别力证明，
//! 交审单「修前红 16 / 修后绿 1」两数字）。wall time 只打印不断言（共享机
//! 负载噪声，次数断言为判别力主体——云尺纪律备注）。

mod common;

use common::{ensure_commands, wait_active, FakeLlm, FakeLlmPlugin};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use yunyou::{Context, ServiceId};
use yunyou_agent_loop::AgentLoopRunner;
use yunyou_llm::{Llm, StreamDelta, ToolCallDelta, Usage};
use yunyou_session::{JsonlStore, SessionEvent, SessionStore};
use yunyou_system_prompt::SystemPrompt;
use yunyou_tools::RegistryService;
use yunyou_util::BoxFut;

/// 计数装饰器：包真 store，统计 load_all 调用次数（append 透传）。
struct CountingStore {
    inner: Arc<JsonlStore>,
    load_all_count: Arc<AtomicUsize>,
}

impl SessionStore for CountingStore {
    fn append(&self, event: &SessionEvent) -> BoxFut<'static, yunyou::Result<()>> {
        self.inner.append(event)
    }
    fn load_all(&self) -> BoxFut<'static, yunyou::Result<Vec<SessionEvent>>> {
        self.load_all_count.fetch_add(1, Ordering::SeqCst);
        self.inner.load_all()
    }
}

/// 单个工具步脚本：bash echo（无副作用）+ tool_calls 收尾。
fn tool_step(n: usize) -> Vec<StreamDelta> {
    vec![
        StreamDelta::ToolCall(ToolCallDelta {
            index: 0,
            id: Some(format!("call-{n}")),
            name: Some("bash".into()),
            arguments_delta: format!("{{\"cmd\":\"echo step-{n}\"}}"),
        }),
        StreamDelta::Done {
            finish_reason: Some("tool_calls".into()),
            usage: Some(Usage { prompt_tokens: 1, completion_tokens: 1 }),
        },
    ]
}

/// 16 步回合脚本：15 个工具步 + 第 16 步纯文本收尾（护栏默认全关，
/// 步数与终止无涉——task-23 起主循环无步数硬顶）。
fn sixteen_step_script() -> Vec<Vec<StreamDelta>> {
    let mut scripts: Vec<Vec<StreamDelta>> = (0..15).map(tool_step).collect();
    scripts.push(vec![
        StreamDelta::Text("十五步工具全部完成。".into()),
        StreamDelta::Done {
            finish_reason: Some("stop".into()),
            usage: Some(Usage { prompt_tokens: 2, completion_tokens: 2 }),
        },
    ]);
    scripts
}

/// 16 步工具回合：load_all 恰 1 次（修前=16）+ 行为不变（15 次工具调用+文本收尾）。
#[tokio::test(flavor = "multi_thread")]
async fn 十六步回合会话单次载入() {
    let dir = std::env::temp_dir().join(format!("dsh-io-count-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let root = Context::new();
    ensure_commands(&root).await;
    wait_active(
        &root.plugin(yunyou_tools::ToolsPlugin, json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(
        &root.plugin(
            FakeLlmPlugin { llm: Arc::new(FakeLlm::new(sixteen_step_script())) },
            json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, json!({})),
        "system-prompt",
    )
    .await;
    let llm = root
        .get::<dyn Llm>(ServiceId::of::<dyn Llm>())
        .expect("llm 服务应可见");
    let tools = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("注册表应可见");
    let prompt = root
        .get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>())
        .expect("系统提示服务应可见");

    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(CountingStore {
        inner: JsonlStore::new(dir.join("session.jsonl")),
        load_all_count: Arc::clone(&counter),
    });
    let runner = AgentLoopRunner {
        budget_warn_ratio: 0.8,
        auto_winddown: false,
        winddown_grace_steps: 4,
        sessions: store,
        llm,
        tools,
        prompt,
        persona: String::new(),
        tool_timeout: None,
        // 护栏两旋钮默认全关（task-23）：本文件测 IO 账本与默认终止语义。
        max_steps: None,
        token_budget: None,
        context_window: None,
        compact_reserved_output: 20_000,
        compact_safety_margin: 32_000,
        auto_compact_enabled: true,
        compaction_failure_limit: 3,
        compaction_state: yunyou_agent_loop::CompactionState::new(),
        fiber_cancel: CancellationToken::new(),
        retry: None,
        // task-11 钩子缝：测试直构 runner 空注册表（零钩子=现状逐字节一致）
        hooks: Default::default(),
    };

    let started = Instant::now();
    let outcome = runner
        .run_turn("跑十五步工具再收尾".to_string(), CancellationToken::new())
        .await
        .expect("16 步回合应成功");
    let elapsed = started.elapsed();

    // 行为不变面：15 次工具调用 + 文本收尾 + 未截断
    assert_eq!(outcome.tool_calls, 15, "工具调用次数");
    assert!(!outcome.truncated, "16 步内收尾不应截断");
    assert!(outcome.assistant_text.contains("十五步"), "{}", outcome.assistant_text);

    // 判别力锚（云驰裁决①锁死）：整回合 load_all 恰 1 次（修前基线=16）
    let count = counter.load(Ordering::SeqCst);
    assert_eq!(count, 1, "一回合应恰 1 次全量载入（修前=16，回潮即红）");

    // wall time 对照（只打印不断言——共享机噪声，云尺纪律备注）
    println!("16 步回合 wall time: {elapsed:?}；load_all 次数: {count}（修前基线 16）");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 双步最小判别（云驰裁决①裁量注记：N≥2 已判别）——2 步回合同样恰 1 次载入。
#[tokio::test(flavor = "multi_thread")]
async fn 两步回合同样单次载入() {
    let dir = std::env::temp_dir().join(format!("dsh-io-count2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let root = Context::new();
    ensure_commands(&root).await;
    wait_active(
        &root.plugin(yunyou_tools::ToolsPlugin, json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(
        &root.plugin(
            FakeLlmPlugin {
                llm: Arc::new(FakeLlm::new(vec![tool_step(0), vec![
                    StreamDelta::Text("完成。".into()),
                    StreamDelta::Done {
                        finish_reason: Some("stop".into()),
                        usage: Some(Usage { prompt_tokens: 1, completion_tokens: 1 }),
                    },
                ]])),
            },
            json!({}),
        ),
        "fake-llm",
    )
    .await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, json!({})),
        "system-prompt",
    )
    .await;
    let llm = root.get::<dyn Llm>(ServiceId::of::<dyn Llm>()).expect("llm");
    let tools = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("registry");
    let prompt = root
        .get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>())
        .expect("prompt");

    let counter = Arc::new(AtomicUsize::new(0));
    let runner = AgentLoopRunner {
        budget_warn_ratio: 0.8,
        auto_winddown: false,
        winddown_grace_steps: 4,
        sessions: Arc::new(CountingStore {
            inner: JsonlStore::new(dir.join("session.jsonl")),
            load_all_count: Arc::clone(&counter),
        }),
        llm,
        tools,
        prompt,
        persona: String::new(),
        tool_timeout: None,
        // 护栏两旋钮默认全关（task-23）：本文件测 IO 账本与默认终止语义。
        max_steps: None,
        token_budget: None,
        context_window: None,
        compact_reserved_output: 20_000,
        compact_safety_margin: 32_000,
        auto_compact_enabled: true,
        compaction_failure_limit: 3,
        compaction_state: yunyou_agent_loop::CompactionState::new(),
        fiber_cancel: CancellationToken::new(),
        retry: None,
        // task-11 钩子缝：测试直构 runner 空注册表（零钩子=现状逐字节一致）
        hooks: Default::default(),
    };
    let outcome = runner
        .run_turn("两步即可".to_string(), CancellationToken::new())
        .await
        .expect("2 步回合应成功");
    assert_eq!(outcome.tool_calls, 1);
    assert_eq!(counter.load(Ordering::SeqCst), 1, "2 步回合也应恰 1 次载入（N≥2 即判别）");
    let _ = std::fs::remove_dir_all(&dir);
}
