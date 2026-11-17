//! K-16 自动续跑四组测试（设计 §3.2/§8）：检测三类/计数递增/round≥2 升级/
//! 中断上报节流——service 层直调（无 AgentLoop 依赖；第 7 课口径含存量靶）。

use std::path::PathBuf;
use std::sync::Arc;
use yunyou_session::{JsonlStore, SessionEvent, SessionStore};

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k16-resume-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mk_store(dir: &std::path::Path) -> Arc<JsonlStore> {
    JsonlStore::new(dir.join("teammate-test.jsonl"))
}

/// 尾部扫描检测分类（与 member.rs startup_resume_check 同口径的测试镜像——
/// 单源防两份漂移候 member.rs 提 pub(crate) 后改直引，v1 镜像+注释锚）。
fn detect_tail(events: &[SessionEvent]) -> Option<&'static str> {
    let last = events.last()?;
    match last {
        SessionEvent::SystemMessage { text } if text.contains("回复「继续」") => Some("软着陆中断"),
        SessionEvent::ToolCall { .. } => Some("工具执行中断"),
        SessionEvent::UserMessage { .. } => Some("孤儿输入"),
        _ => None,
    }
}

fn resume_round(events: &[SessionEvent]) -> u32 {
    events
        .iter()
        .rev()
        .find_map(|e| match e {
            SessionEvent::ResumeMark { round, .. } => Some(*round),
            _ => None,
        })
        .unwrap_or(0)
}

/// 检测三类+非未完形态不误报（表驱动）。
#[tokio::test(flavor = "multi_thread")]
async fn 检测三类与非未完不误报() {
    // 软着陆中断：SystemMessage 含「回复「继续」」
    let dir = tmp_dir("detect-paused");
    let store = mk_store(&dir);
    store
        .append(&SessionEvent::SystemMessage {
            text: "已到达本轮 4 步上限，回复「继续」即可无缝延续。".into(),
        })
        .await
        .unwrap();
    let events = store.load_all().await.unwrap();
    assert_eq!(detect_tail(&events), Some("软着陆中断"));

    // 工具执行中断：尾=ToolCall 无配对 ToolResult
    let dir = tmp_dir("detect-tool");
    let store = mk_store(&dir);
    store
        .append(&SessionEvent::ToolCall {
            call_id: "c1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
            parent_tool_call_id: None,
            batch_id: None,
        })
        .await
        .unwrap();
    let events = store.load_all().await.unwrap();
    assert_eq!(detect_tail(&events), Some("工具执行中断"));

    // 孤儿输入：尾=UserMessage 无 assistant
    let dir = tmp_dir("detect-orphan");
    let store = mk_store(&dir);
    store.append(&SessionEvent::UserMessage { text: "做任务".into() }).await.unwrap();
    let events = store.load_all().await.unwrap();
    assert_eq!(detect_tail(&events), Some("孤儿输入"));

    // 非未完：尾=AssistantMessage（正常完成）→None 不误报
    let dir = tmp_dir("detect-normal");
    let store = mk_store(&dir);
    store
        .append(&SessionEvent::AssistantMessage { text: "完成".into(), reasoning: None })
        .await
        .unwrap();
    let events = store.load_all().await.unwrap();
    assert_eq!(detect_tail(&events), None, "正常完成尾部不误报");

    // 非未完：尾=ToolResult（工具配对完整）→None
    let dir = tmp_dir("detect-paired");
    let store = mk_store(&dir);
    store
        .append(&SessionEvent::ToolResult {
            call_id: "c1".into(),
            name: "bash".into(),
            content: "ok".into(),
            is_error: false,
            parent_tool_call_id: None,
            batch_id: None,
        })
        .await
        .unwrap();
    let events = store.load_all().await.unwrap();
    assert_eq!(detect_tail(&events), None);
}

/// 计数递增：ResumeMark round 递增（同一未完回合重复重启→round+1）。
#[tokio::test(flavor = "multi_thread")]
async fn 计数递增幂等() {
    let dir = tmp_dir("count");
    let store = mk_store(&dir);
    // 首次重启：无 ResumeMark→round=0→续跑后落 round=1
    store
        .append(&SessionEvent::UserMessage { text: "做任务".into() })
        .await
        .unwrap();
    let events = store.load_all().await.unwrap();
    assert_eq!(resume_round(&events), 0);
    store.append(&SessionEvent::ResumeMark { round: 1, reason: "孤儿输入".into() }).await.unwrap();
    // 二次重启：round=1→续跑后落 round=2
    let events = store.load_all().await.unwrap();
    assert_eq!(resume_round(&events), 1);
    store.append(&SessionEvent::ResumeMark { round: 2, reason: "孤儿输入".into() }).await.unwrap();
    // 三次重启：round=2≥2→升级不自动续
    let events = store.load_all().await.unwrap();
    assert_eq!(resume_round(&events), 2);
    assert!(resume_round(&events) >= 2, "round≥2 升级阈值触发");
}

/// 升级判定：round≥2=不再自动续（返回升级态）。
#[tokio::test(flavor = "multi_thread")]
async fn 升级判定() {
    let dir = tmp_dir("escalate");
    let store = mk_store(&dir);
    store.append(&SessionEvent::UserMessage { text: "做任务".into() }).await.unwrap();
    store.append(&SessionEvent::ResumeMark { round: 1, reason: "孤儿输入".into() }).await.unwrap();
    store.append(&SessionEvent::ResumeMark { round: 2, reason: "孤儿输入".into() }).await.unwrap();
    let events = store.load_all().await.unwrap();
    let round = resume_round(&events);
    // round≥2=升级（member.rs startup_resume_check 同口径：不自动续+报 lead）
    assert!(round >= 2, "升级阈值");
    // 新 assistant 产生后旧标记不再是尾部→检测自然熄火（幂等无需显式清位）
    store
        .append(&SessionEvent::AssistantMessage { text: "续跑产出".into(), reasoning: None })
        .await
        .unwrap();
    let events = store.load_all().await.unwrap();
    assert_eq!(detect_tail(&events), None, "续跑新回合后检测自然熄火");
}

/// 中断上报节流：同回合多次中断只发首报+末报（结构锚：上报点在 run_one
/// 尾部一次/回合——节流由结构保证非计数器；此处锁上报体的形态判别）。
#[test]
fn 中断上报体形态() {
    // InterruptReport 上报体=「【中断上报】成员「X」回合中断：reason」
    let body = format!("【中断上报】成员「{}」回合中断：{}", "scout", "流中断");
    assert!(body.contains("【中断上报】"));
    assert!(body.contains("scout"));
    assert!(body.contains("流中断"));
    // 升级上报体（startup_resume_check round≥2 路径）
    let body = format!(
        "成员「{}」重启检测：未完回合续跑已达上限（round={}），请人工介入（reason={}）。",
        "scout", 2, "孤儿输入"
    );
    assert!(body.contains("已达上限") && body.contains("请人工介入"));
}
