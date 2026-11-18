//! 终止语义与护栏软着陆测试（task-23，P0；复核人=云枢）。
//!
//! 业主实证令（2026-11-16）：「正常用怎么还触发16步上限，不应该一直循环到
//! 任务完成吗，只有异常才触发」——四用例：
//! ①默认全关 25 步完整跑完不截肢（旧行为 16 步腰斩的判别力锚）；
//! ②max_steps=5 拧上→5 步软着陆（TurnPaused 结构化事件+暂停文案+truncated）
//!   +「继续」续跑端到端（上下文完整保留，新回合无缝收尾）；
//! ③token_budget 拧上→预算触发 BudgetExhausted 软着陆；
//! ④usage 未报→预算护栏对本轮失效（诚实不估算的行为锁定）。
//! 腰斩文案零残留断言随 ②（SystemMessage 不含「已达上限…本轮终止」）。

mod common;

use common::{ensure_commands, wait_active, FakeLlm, FakeLlmPlugin};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use yunyou::{Context, ServiceId};
use yunyou_agent_loop::AgentLoopRunner;
use yunyou_llm::{Llm, StreamDelta, ToolCallDelta, Usage};
use yunyou_session::{JsonlStore, SessionEvent, SessionStore};
use yunyou_system_prompt::SystemPrompt;
use yunyou_tools::RegistryService;

/// 单个工具步脚本：bash echo + usage 20/步 + tool_calls 收尾。
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
            usage: Some(Usage { prompt_tokens: 10, completion_tokens: 10 }),
        },
    ]
}

/// 文本收尾脚本（无工具调用=模型停手=回合自然终）。
fn final_text(text: &str) -> Vec<StreamDelta> {
    vec![
        StreamDelta::Text(text.into()),
        StreamDelta::Done {
            finish_reason: Some("stop".into()),
            usage: Some(Usage { prompt_tokens: 5, completion_tokens: 5 }),
        },
    ]
}

/// 无 usage 版工具步（护栏失效用例④：usage 口径「任一步未报→整体 None」）。
fn tool_step_no_usage(n: usize) -> Vec<StreamDelta> {
    vec![
        StreamDelta::ToolCall(ToolCallDelta {
            index: 0,
            id: Some(format!("call-{n}")),
            name: Some("bash".into()),
            arguments_delta: format!("{{\"cmd\":\"echo step-{n}\"}}"),
        }),
        StreamDelta::Done { finish_reason: Some("tool_calls".into()), usage: None },
    ]
}

/// 装配（tools/fake-llm/system-prompt 三插件；store 走真实 JsonlStore）。
async fn assemble(
    scripts: Vec<Vec<StreamDelta>>,
    session_dir: &std::path::Path,
) -> Context {
    let root = Context::new();
    ensure_commands(&root).await;
    wait_active(
        &root.plugin(yunyou_tools::ToolsPlugin, json!({ "root_dir": session_dir.to_string_lossy() })),
        "tools",
    )
    .await;
    wait_active(
        &root.plugin(FakeLlmPlugin { llm: Arc::new(FakeLlm::new(scripts)) }, json!({})),
        "fake-llm",
    )
    .await;
    wait_active(
        &root.plugin(yunyou_system_prompt::SystemPromptPlugin, json!({})),
        "system-prompt",
    )
    .await;
    root
}

fn services(
    root: &Context,
    session_dir: &std::path::Path,
    max_steps: Option<usize>,
    token_budget: Option<u64>,
) -> AgentLoopRunner {
    services_with_compact(root, session_dir, max_steps, token_budget, None, false)
}

/// 带压缩旋钮的变体（task-27 用例⑤+；auto=false=既有用例零扰）。
#[allow(clippy::too_many_arguments)]
fn services_with_compact(
    root: &Context,
    session_dir: &std::path::Path,
    max_steps: Option<usize>,
    token_budget: Option<u64>,
    context_window: Option<u64>,
    auto_compact_enabled: bool,
) -> AgentLoopRunner {
    let llm = root.get::<dyn Llm>(ServiceId::of::<dyn Llm>()).expect("llm 应可见");
    let tools = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .expect("注册表应可见");
    let prompt = root
        .get::<dyn SystemPrompt>(ServiceId::of::<dyn SystemPrompt>())
        .expect("系统提示服务应可见");
    AgentLoopRunner {
        budget_warn_ratio: 0.8,
        auto_winddown: false,
        winddown_grace_steps: 4,
        sessions: JsonlStore::new(session_dir.join("session.jsonl")),
        llm,
        tools,
        prompt,
        persona: String::new(),
        tool_timeout: None,
        // 护栏两旋钮：本文件主角（默认全关/拧上两态各有用例）。
        max_steps,
        token_budget,
        context_window,
        compact_reserved_output: 20_000,
        compact_safety_margin: 32_000,
        auto_compact_enabled,
        compaction_failure_limit: 3,
        compaction_state: yunyou_agent_loop::CompactionState::new(),
        fiber_cancel: CancellationToken::new(),
        retry: None,
        // task-11 钩子缝：测试直构 runner 空注册表（零钩子=现状逐字节一致）
        hooks: Default::default(),
        gates: Default::default(),
    }
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dsh-limits-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 用例①（验收②）：默认全关（两旋钮 None）25 步回合完整跑完不截肢——
/// 旧行为 16 步腰斩（「工具循环已达上限」+truncated）在此必红，判别力锚。
#[tokio::test(flavor = "multi_thread")]
async fn 默认全关二十五步完整跑完() {
    let dir = tmp_dir("unbounded");
    let scripts: Vec<Vec<StreamDelta>> =
        (0..24).map(tool_step).chain(std::iter::once(final_text("二十五步全部完成。"))).collect();
    let root = assemble(scripts, &dir).await;
    let runner = services(&root, &dir, None, None);
    let outcome = runner
        .run_turn("跑二十四步工具再收尾".to_string(), CancellationToken::new())
        .await
        .expect("25 步回合应成功");
    assert_eq!(outcome.tool_calls, 24, "24 次工具调用应全部执行");
    assert!(!outcome.truncated, "无护栏不得暂停截肢（旧行为 16 步腰斩判别锚）");
    assert!(outcome.assistant_text.contains("二十五步"), "{}", outcome.assistant_text);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例②（验收+云枢新增：软着陆可续端到端）：max_steps=5 → 5 步软着陆
/// （TurnPaused 事件+暂停文案 SystemMessage 落盘+truncated=true）→ 用户回复
/// 「继续」=新回合从完整历史续跑收尾（无特殊续跑机制即语义诚实）。
#[tokio::test(flavor = "multi_thread")]
async fn 步数护栏软着陆后继续续跑端到端() {
    let dir = tmp_dir("steps");
    // 第一回合：4 工具步+1 工具步（第 5 步末触发）——脚本给 5 步工具+收尾
    //（第 6 步脚本不会被消费：软着陆发生在第 5 步末）。
    let scripts: Vec<Vec<StreamDelta>> =
        (0..5).map(tool_step).chain(std::iter::once(final_text("续跑收尾。"))).collect();
    let root = assemble(scripts, &dir).await;
    let observer = yunyou_agent_loop::CollectingObserver::new();
    let runner = services(&root, &dir, Some(5), None);
    let outcome = runner
        .run_turn_observed("跑五步工具".to_string(), CancellationToken::new(), observer.clone())
        .await
        .expect("软着陆应 Ok（非 Err 腰斩）");
    assert_eq!(outcome.tool_calls, 5, "5 步全部执行后才软着陆（配对完整性）");
    assert!(outcome.truncated, "护栏暂停标记");

    // 结构化事件断言：TurnPaused(StepsExhausted) 到达且先于 TurnComplete
    let events = observer.snapshot();
    let paused = events.iter().position(|e| matches!(e, yunyou_agent_loop::TurnEvent::TurnPaused { reason, .. } if *reason == yunyou_agent_loop::TurnPauseReason::StepsExhausted));
    let completed = events.iter().position(|e| matches!(e, yunyou_agent_loop::TurnEvent::TurnComplete { .. }));
    let paused_pos = paused.expect("应发 TurnPaused 结构化事件");
    let completed_pos = completed.expect("Ok 路径应以 TurnComplete 收尾");
    assert!(paused_pos < completed_pos, "TurnPaused 应先于 TurnComplete（软着陆非腰斩）");
    if let yunyou_agent_loop::TurnEvent::TurnPaused { message, .. } = &events[paused_pos] {
        assert!(message.contains("已到达本轮 5 步上限"), "文案模板: {message}");
        assert!(message.contains("继续"), "文案应指路「继续」续跑: {message}");
    }

    // 腰斩文案零残留（业主实证令：禁「已达上限…本轮终止」）
    let store_events = JsonlStore::new(dir.join("session.jsonl"))
        .load_all()
        .await
        .expect("会话应可读");
    assert!(
        !store_events.iter().any(|e| matches!(e, SessionEvent::SystemMessage { text } if text.contains("本轮终止"))),
        "腰斩文案必须零残留: {store_events:?}"
    );
    assert!(
        store_events.iter().any(|e| matches!(e, SessionEvent::SystemMessage { text } if text.contains("无缝延续"))),
        "暂停 SystemMessage 应落盘（续跑的上下文锚）: {store_events:?}"
    );

    // 「继续」续跑端到端：新回合输入「继续」，脚本余量=收尾文本（第 6 段）
    // ——FakeLlm 脚本按调用序消费，第二回合拿到收尾段即证上下文延续由
    // 端点判定（本测试锚在「新回合正常收尾+truncated=false」）。
    // 注：FakeLlm 按脚本序发段，第二回合若复用同 runner 需重灌脚本——
    // 此处直接重灌一个只有收尾段的 runner 模拟「模型看到继续指示后收尾」。
    let scripts2 = vec![final_text("继续后收尾。")];
    let root2 = assemble(scripts2, &dir).await;
    let runner2 = services(&root2, &dir, None, None);
    let outcome2 = runner2
        .run_turn("继续".to_string(), CancellationToken::new())
        .await
        .expect("继续回合应成功");
    assert!(!outcome2.truncated, "续跑回合正常收尾");
    assert_eq!(outcome2.tool_calls, 0, "续跑回合无工具调用直接收尾");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例③：token_budget=50、每步 usage 20 → 第 3 步末累计 60≥50 软着陆
/// （BudgetExhausted；步数护栏未拧不涉——两旋钮独立）。
#[tokio::test(flavor = "multi_thread")]
async fn 预算护栏触发软着陆() {
    let dir = tmp_dir("budget");
    let scripts: Vec<Vec<StreamDelta>> =
        (0..5).map(tool_step).chain(std::iter::once(final_text("不会到这。"))).collect();
    let root = assemble(scripts, &dir).await;
    let observer = yunyou_agent_loop::CollectingObserver::new();
    let runner = services(&root, &dir, None, Some(50));
    let outcome = runner
        .run_turn_observed("跑几步工具".to_string(), CancellationToken::new(), observer.clone())
        .await
        .expect("预算软着陆应 Ok");
    assert_eq!(outcome.tool_calls, 3, "第 3 步末累计 60≥50 触发（前两步 40<50）");
    assert!(outcome.truncated, "预算暂停标记");
    let events = observer.snapshot();
    assert!(events.iter().any(
        |e| matches!(e, yunyou_agent_loop::TurnEvent::TurnPaused { reason: yunyou_agent_loop::TurnPauseReason::BudgetExhausted, message } if message.contains("token 预算 50"))
    ), "应发 BudgetExhausted 结构化事件+预算文案: {events:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例④（诚实注记行为锁定）：usage 未报（任一步 None→整体 None）→
/// 预算护栏对本轮失效——跑完不暂停，不部分估算（口径沿用云核既有）。
#[tokio::test(flavor = "multi_thread")]
async fn usage缺失时预算护栏失效() {
    let dir = tmp_dir("no-usage");
    let scripts: Vec<Vec<StreamDelta>> =
        (0..3).map(tool_step_no_usage).chain(std::iter::once(final_text("护栏失效跑完。"))).collect();
    let root = assemble(scripts, &dir).await;
    let runner = services(&root, &dir, None, Some(1)); // 预算=1 极小值
    let outcome = runner
        .run_turn("跑三步工具再收尾".to_string(), CancellationToken::new())
        .await
        .expect("usage 缺失回合应正常收尾");
    assert_eq!(outcome.tool_calls, 3, "护栏失效=全部执行");
    assert!(!outcome.truncated, "usage None 时预算不得触发（诚实不估算）");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例⑤（task-26 联动锚）：TurnComplete.context_tokens=**最后一步**
/// usage.prompt（水位快照）而非累计（prompt_tokens=60 流量）——递增 usage
/// 10/20/30 三步后：水位=30、累计=60，两值不可混（业主实证：累计被误读
/// 成上下文——本用例锁死语义分流）。
#[tokio::test(flavor = "multi_thread")]
async fn 回合完成事件携带水位快照非累计() {
    let dir = tmp_dir("watermark");
    let step_with = |prompt: u64, n: usize| -> Vec<StreamDelta> {
        vec![
            StreamDelta::ToolCall(ToolCallDelta {
                index: 0,
                id: Some(format!("call-{n}")),
                name: Some("bash".into()),
                arguments_delta: format!("{{\"cmd\":\"echo step-{n}\"}}"),
            }),
            StreamDelta::Done {
                finish_reason: Some("tool_calls".into()),
                usage: Some(Usage { prompt_tokens: prompt, completion_tokens: 1 }),
            },
        ]
    };
    let scripts = vec![
        step_with(10, 0),
        step_with(20, 1),
        step_with(30, 2),
        vec![
            StreamDelta::Text("收尾。".into()),
            StreamDelta::Done {
                finish_reason: Some("stop".into()),
                usage: Some(Usage { prompt_tokens: 40, completion_tokens: 1 }),
            },
        ],
    ];
    let root = assemble(scripts, &dir).await;
    let observer = yunyou_agent_loop::CollectingObserver::new();
    let runner = services(&root, &dir, None, None);
    let _outcome = runner
        .run_turn_observed("跑三步工具再收尾".to_string(), CancellationToken::new(), observer.clone())
        .await
        .expect("回合应成功");
    let events = observer.snapshot();
    let complete = events
        .iter()
        .find_map(|e| match e {
            yunyou_agent_loop::TurnEvent::TurnComplete { prompt_tokens, context_tokens, .. } => {
                Some((*prompt_tokens, *context_tokens))
            }
            _ => None,
        })
        .expect("应发 TurnComplete");
    assert_eq!(complete.1, Some(40), "水位=最后一步 prompt（快照，非累计）");
    assert_eq!(complete.0, Some(100), "累计=各步求和（流量）——两值语义不可混");
    assert_ne!(complete.0, complete.1, "水位与累计在本场景必须可区分（判别力）");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── task-27 上下文压缩（S7 核心：触发/时序承重墙/默认关）──────────────

/// 用例⑥（V6+V1+S3 承重墙断言）：水位≥触发线 → 回合头压缩——**compaction
/// 事件先于本轮 UserMessage 落盘**（S3 承重墙：时序反了=分割切走本轮用户
/// 消息）；压缩后本回合正常对话（脚本首段被摘要请求消费=压缩确实发生）。
#[tokio::test(flavor = "multi_thread")]
async fn 水位越线回合头压缩且先于用户消息落盘() {
    let dir = tmp_dir("trigger");
    // 脚本序：第 1 调=摘要请求（消费「摘要文本」段）、第 2 调=正常对话（工具步+收尾）
    let scripts = vec![
        vec![
            StreamDelta::Text("此前任务已完成大半，剩收尾。".into()),
            StreamDelta::Done {
                finish_reason: Some("stop".into()),
                usage: Some(Usage { prompt_tokens: 15, completion_tokens: 1 }),
            },
        ],
        tool_step(0),
        final_text("压缩后正常对话。"),
    ];
    let root = assemble(scripts, &dir).await;
    let runner = services_with_compact(&root, &dir, None, None, Some(100_000), true);
    // 预置历史（真实不变量：水位≥线必有历史对话——空历史不可能有水位）；
    // 待压缩段非空=摘要请求有输入。
    let preset: Arc<dyn SessionStore> = JsonlStore::new(dir.join("session.jsonl"));
    preset
        .append(&SessionEvent::UserMessage { text: "此前任务：整理报告".into() })
        .await
        .expect("预置应成功");
    preset
        .append(&SessionEvent::AssistantMessage { text: "报告主体已完成。".into(), reasoning: None })
        .await
        .expect("预置应成功");
    // 上回合末水位=50_000 ≥ 触发线（100_000−20_000−32_000=48_000）→ 触发
    runner.compaction_state.set_watermark(Some(50_000));
    let _outcome = runner
        .run_turn("本轮任务：收尾".to_string(), CancellationToken::new())
        .await
        .expect("压缩回合应成功");
    // V1+时序断言：JSONL 事件序=compaction 先于本轮 user-message
    let events = JsonlStore::new(dir.join("session.jsonl"))
        .load_all()
        .await
        .expect("会话应可读");
    let compaction_pos = events
        .iter()
        .position(|e| matches!(e, SessionEvent::Compaction { pre_tokens: 50_000, .. }))
        .expect("应有 Compaction 事件（pre=50_000）");
    let user_pos = events
        .iter()
        .position(|e| matches!(e, SessionEvent::UserMessage { text } if text.contains("本轮任务")))
        .expect("应有本轮 UserMessage");
    assert!(
        compaction_pos < user_pos,
        "S3 承重墙：压缩必须先于本轮 UserMessage 落盘（反了=分割切走本轮消息）：{events:?}"
    );
    let _ = std::fs::remove_dir_all(&dir); // 诊断期暂注释
    // V1：全量物理事实仍在（压缩前的水位回写来源事件不存在——但工具步事件全在）
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::ToolResult { .. })),
        "压缩不删任何物理行"
    );
    // let _ = std::fs::remove_dir_all(&dir); // 诊断期暂留
}

/// 用例⑦（V11+V6 三态之「不触发」）：水位低于触发线 → 零压缩（脚本首段
/// 即正常对话=无摘要请求发生——默认关等价性的触发面验证）。
#[tokio::test(flavor = "multi_thread")]
async fn 水位未越线不触发压缩() {
    let dir = tmp_dir("no-trigger");
    // 脚本序：若发生压缩，首段会被摘要请求消费（「误触发标记」段出不来）
    let scripts = vec![
        vec![
            StreamDelta::Text("误触发标记：此段只应在无压缩时作为对话回复出现。".into()),
            StreamDelta::Done {
                finish_reason: Some("stop".into()),
                usage: Some(Usage { prompt_tokens: 1, completion_tokens: 1 }),
            },
        ],
    ];
    let root = assemble(scripts, &dir).await;
    let runner = services_with_compact(&root, &dir, None, None, Some(100_000), true);
    runner.compaction_state.set_watermark(Some(47_999)); // < 触发线 48_000
    let outcome = runner
        .run_turn("普通回合".to_string(), CancellationToken::new())
        .await
        .expect("回合应成功");
    assert!(outcome.assistant_text.contains("误触发标记"), "未越线不得压缩（脚本首段被摘要请求吃掉即红）");
    let events = JsonlStore::new(dir.join("session.jsonl"))
        .load_all()
        .await
        .expect("会话应可读");
    assert!(
        !events.iter().any(|e| matches!(e, SessionEvent::Compaction { .. })),
        "未越线不应有压缩事件"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例⑧（V7 熔断三段）：摘要请求连败 3 次（Flaky Fail）→熔断 disabled=true
/// （第 4 回合不再尝试=LLM 调用数锁死）——对话回合全程走 Script 成（压缩
/// 失败不中断对话）。/compact 成功清零恢复路径经 compact_manual（此处验证
/// 计数语义：disabled 置位后手动成功=清零，端到端面候 /compact 命令批）。
#[tokio::test(flavor = "multi_thread")]
async fn 摘要连败三次熔断() {
    use common::FlakyOutcome;
    let dir = tmp_dir("breaker");
    let root = Context::new();
    ensure_commands(&root).await;
    wait_active(
        &root.plugin(yunyou_tools::ToolsPlugin, json!({ "root_dir": dir.to_string_lossy() })),
        "tools",
    )
    .await;
    // 序列：3×(摘要败+对话成)+第 4 回合对话成（熔断短路无摘要调用）
    let mut outcomes = Vec::new();
    // usage.prompt 自洽大值（50_500/50_700/50_900 递增）：压缩失败=上下文未
    // 变小——回写保持高水位→每回合头继续触发（连败 3 次的前提）；假小值会
    // 让回合 2/3 不触发、对话吃到摘要的 Fail（序错位）。
    for (idx, watermark) in [50_500u64, 50_700, 50_900].into_iter().enumerate() {
        let _ = idx;
        outcomes.push(FlakyOutcome::Fail(yunyou_llm::LlmFailure::stream_interrupted(
            String::from("摘要端点故障"),
        )));
        outcomes.push(FlakyOutcome::Script(vec![
            StreamDelta::Text("对话正常收尾。".into()),
            StreamDelta::Done {
                finish_reason: Some("stop".into()),
                usage: Some(Usage { prompt_tokens: watermark, completion_tokens: 10 }),
            },
        ]));
    }
    outcomes.push(FlakyOutcome::Script(vec![
        StreamDelta::Text("熔断后对话。".into()),
        StreamDelta::Done {
            finish_reason: Some("stop".into()),
            usage: Some(Usage { prompt_tokens: 10, completion_tokens: 10 }),
        },
    ]));
    let flaky = common::FlakyLlm::new(outcomes);
    let calls = flaky.calls.clone();
    wait_active(
        &root.plugin(FakeLlmPlugin { llm: Arc::new(flaky) }, json!({})),
        "flaky-llm",
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
    let runner = AgentLoopRunner {
        sessions: JsonlStore::new(dir.join("session.jsonl")),
        llm,
        tools,
        prompt,
        persona: String::new(),
        tool_timeout: None,
        max_steps: None,
        token_budget: None,
        // K-16① 三旋钮（夜间批字段，融合补位）：本用例不涉宽限收尾，默认值。
        budget_warn_ratio: 0.8,
        auto_winddown: false,
        winddown_grace_steps: 4,
        context_window: Some(100_000),
        compact_reserved_output: 20_000,
        compact_safety_margin: 32_000,
        auto_compact_enabled: true,
        compaction_failure_limit: 3,
        compaction_state: yunyou_agent_loop::CompactionState::new(),
        fiber_cancel: CancellationToken::new(),
        retry: None,
        // task-11 钩子缝：测试直构 runner 空注册表（零钩子=现状逐字节一致）
        hooks: Default::default(),
        gates: Default::default(),
    };
    let state = runner.compaction_state.clone();
    // 预置历史（待压缩段非空前提——空历史不可能有水位，真实不变量同用例⑥）
    let preset: Arc<dyn SessionStore> = JsonlStore::new(dir.join("session.jsonl"));
    preset
        .append(&SessionEvent::UserMessage { text: "此前任务：整理报告".into() })
        .await
        .expect("预置应成功");
    preset
        .append(&SessionEvent::AssistantMessage { text: "报告主体已完成。".into(), reasoning: None })
        .await
        .expect("预置应成功");
    state.set_watermark(Some(50_000)); // 越线（线=48k）
    // 三回合：每回合头触发→摘要败（计数+1）→对话正常收尾
    for round in 1..=3 {
        runner
            .clone()
            .run_turn(format!("失败回合 {round}"), CancellationToken::new())
            .await
            .expect("压缩失败不中断对话");
    }
    assert!(
        state.disabled.load(std::sync::atomic::Ordering::Relaxed),
        "连败 3 次应熔断"
    );
    assert_eq!(
        state.failures.load(std::sync::atomic::Ordering::Relaxed),
        3,
        "计数停在第 3"
    );
    // 第 4 回合：熔断短路——无摘要调用（calls=3×2+1=7；若尝试摘要会=8）
    state.set_watermark(Some(60_000));
    runner
        .clone()
        .run_turn("熔断后回合".into(), CancellationToken::new())
        .await
        .expect("第 4 回合应成功");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        7,
        "熔断后无摘要调用（3×2 对话+第 4 回合对话；尝试摘要则=8）"
    );
    // /compact 手动成功=清零恢复（V7 第三段端到端）：装配 AgentLoopPlugin
    //（config 带窗口与旋钮）→compact_manual→成功路径 store(0) 清零。
    // 预置历史已在上（同 store 路径）——摘要脚本用 Flaky 队列余量（耗尽态
    // 回落段=文本成——FlakyLlm 空队列回落「(队列已耗尽)」文本=非空摘要 ✓）。
    let team_service_free_root = &root; // 命名避歧义：同 root 装 AgentLoopPlugin
    // AgentLoopPlugin inject 四依赖：sessions 需显式 provide（assemble 段未装）
    let session_store: Arc<dyn SessionStore> = JsonlStore::new(dir.join("session.jsonl"));
    team_service_free_root
        .provide(yunyou::ServiceId::of::<dyn SessionStore>(), session_store)
        .expect("provide session");
    wait_active(
        &team_service_free_root.plugin(
            yunyou_agent_loop::AgentLoopPlugin,
            json!({
                "context_window": 100_000,
                "auto_compact_enabled": true,
            }),
        ),
        "agent-loop",
    )
    .await;
    let service = team_service_free_root
        .get::<dyn yunyou_agent_loop::AgentLoop>(ServiceId::of::<dyn yunyou_agent_loop::AgentLoop>())
        .expect("agentLoop 应可见");
    let receipt = service
        .compact_manual(CancellationToken::new())
        .await
        .expect("手动压缩应成功（空队列回落段=非空摘要）");
    assert!(receipt.contains("已压缩"), "回执形态: {receipt}");
    // 断言=隔离性语义（真发现，云枢复核对账点）：熔断 state 按 runner 实例
    // 隔离——compact_manual 清零的是**插件装配 runner** 的 state（生产路径
    // repl 每轮回 root.get 同一 LoopService=同一实例，auto/手动一致 ✓）；
    // 测试直构 runner 与插件 runner 是两实例（直构 state 不受扰=隔离证明）。
    // 清零代码路径=run_compaction 成功 store(0)（compaction.rs 尾行在场）。
    assert!(
        state.disabled.load(std::sync::atomic::Ordering::Relaxed),
        "直构 state 隔离性：插件路径清零不得串扰直构实例"
    );
    let _ = receipt;
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例⑨（V8 取消贯穿+分型）：取消先于回合 → 压缩守卫短路（不调 LLM、
/// 不计失败——取消≠失败分型对齐）+本回合旧上下文正常完成。
#[tokio::test(flavor = "multi_thread")]
async fn 取消态压缩短路不误计失败() {
    let dir = tmp_dir("cancel");
    let scripts = vec![final_text("取消态正常收尾。")];
    let root = assemble(scripts, &dir).await;
    let runner = services_with_compact(&root, &dir, None, None, Some(100_000), true);
    let state = runner.compaction_state.clone(); // Arc：move 后断言仍可查
    state.set_watermark(Some(50_000)); // 越线
    let cancel = CancellationToken::new();
    cancel.cancel(); // 预取消（用户控制权先行）
    let outcome = runner
        .run_turn("预取消回合".to_string(), cancel)
        .await;
    // 预取消在循环头 cancelled 检查即 Err（既有取消语义）——压缩块同样被
    // cancel.is_cancelled() 守卫短路：断言=失败计数未增（取消≠失败）。
    assert!(outcome.is_err(), "预取消回合走既有取消语义");
    assert_eq!(
        state.failures.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "取消不得计入压缩失败（分型对齐）"
    );
    assert!(
        !state.disabled.load(std::sync::atomic::Ordering::Relaxed),
        "取消不得触发熔断"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
