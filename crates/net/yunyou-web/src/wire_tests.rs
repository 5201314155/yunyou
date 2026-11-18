//! wire 状态机测试（外挂：#[path] 先例，源文件行数纪律——295cc25 同款）。

use super::*;
use crate::protocol::ServerMessage;

/// 一回合标准事件流（七事件全形；测试三例共用）。
fn turn_messages() -> Vec<ServerMessage> {
    vec![
        ServerMessage::UserMessage { text: "你好".into() },
        ServerMessage::ReasoningDelta { text: "想".into() },
        ServerMessage::AssistantDelta { text: "我来".into() },
        ServerMessage::ToolCall {
            call_id: "c1".into(),
            name: "read".into(),
            arguments_summary: "{\"path\":\"a.rs\"}".into(),
            parent_tool_call_id: None,
            batch_id: None,
        },
        ServerMessage::ToolResult {
            call_id: "c1".into(),
            name: "read".into(),
            summary: "26 行".into(),
            content: serde_json::json!({"lines": 26}),
            exit_code: Some(0),
            is_error: false,
            parent_tool_call_id: None,
            batch_id: None,
        },
        ServerMessage::AssistantMessage {
            text: "看完了".into(),
            reasoning: Some("想".into()),
        },
        ServerMessage::TurnComplete {
            elapsed_ms: 1200,
            prompt_tokens: Some(10),
            completion_tokens: Some(5),
            model: "test-model".into(),
            tool_calls: 1,
            truncated: false,
        },
    ]
}

/// 空状态机（测试构造；字段私有性由同模块树保证）。
fn state(model: &str, request_id: Option<&str>) -> WireState {
    let mut state = WireState::new(model.into());
    state.set_request_id(request_id.map(str::to_string));
    state
}

/// 一回合完整序列的帧形状——seq 致密、revision 连续、surface 事件带
/// surfaceOp、end 帧 committed seq 指向 assistant/message、rpcId echo 一次。
#[test]
fn 回合序列帧形状与对账铁律() {
    let mut state = state("test-model", Some("req-1"));
    let mut all = Vec::new();
    for msg in turn_messages() {
        let (mut frames, control) = state.apply(&msg);
        frames.extend(control);
        all.extend(frames);
    }

    let kinds: Vec<String> = all
        .iter()
        .map(|f| match f["type"].as_str() {
            Some("event") => format!("event/{}", f["event"]["type"].as_str().unwrap_or("")),
            Some("assistant-stream") => {
                format!("stream/{}", f["frame"]["type"].as_str().unwrap_or(""))
            }
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "event/user/message",
            "event/turn/start",
            "event/step/start",
            "stream/start",       // attempt 开闸
            "stream/chunk",       // reasoning
            "stream/chunk",       // text
            "event/tool/call",
            "event/tool/result",
            "event/assistant/message",
            "stream/end",         // committed → assistant/message
            "event/step/end",
            "event/turn/end",
        ],
        "帧序列：{all:?}"
    );
    // 持久事件 seq 致密 0..=7（journal follows 铁律）
    let seqs: Vec<u64> = all
        .iter()
        .filter(|f| f["type"] == "event")
        .map(|f| f["event"]["seq"].as_u64().expect("seq 数值"))
        .collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4, 5, 6, 7], "seq 致密：{seqs:?}");
    // surface 事件带 append；非 surface 事件不带
    for f in all.iter().filter(|f| f["type"] == "event") {
        let is_surface = matches!(
            f["event"]["type"].as_str(),
            Some("user/message" | "assistant/message" | "system/message" | "tool/result")
        );
        assert_eq!(
            f["event"].get("surfaceOp").is_some(),
            is_surface,
            "surfaceOp 判定：{}",
            f["event"]
        );
    }
    // user/message data=Message 本体 + rpcId echo（一次，未串号）
    assert_eq!(all[0]["event"]["data"]["role"], "user");
    assert_eq!(all[0]["event"]["data"]["source"]["rpcId"], "req-1");
    let (second, _) = state.apply(&ServerMessage::UserMessage { text: "再问".into() });
    assert!(
        second[0]["event"]["data"]["source"].get("rpcId").is_none(),
        "requestId 用后即清，不得串号：{:?}",
        second[0]
    );
    // assistant/message：model source 落款 + text/reasoning 块
    let assistant = &all[8]["event"];
    assert_eq!(assistant["data"]["message"]["source"]["model"], "test-model");
    assert_eq!(assistant["data"]["message"]["content"][0]["type"], "text");
    assert_eq!(assistant["data"]["message"]["content"][1]["type"], "reasoning");
    // end 帧：committed seq 指向 assistant/message（seq 5），index=2（两 chunk）
    let end = &all[9]["frame"];
    assert_eq!(end["outcome"]["kind"], "committed");
    assert_eq!(end["outcome"]["seq"], 5);
    assert_eq!(end["index"], 2);
    // 通知帧 revision 逐帧 +1（1..=4）；chunk index 致密
    let revisions: Vec<u64> = all
        .iter()
        .filter(|f| f["type"] == "assistant-stream")
        .map(|f| f["frame"]["revision"].as_u64().expect("revision 数值"))
        .collect();
    assert_eq!(revisions, vec![1, 2, 3, 4], "revision 逐帧 +1：{revisions:?}");
    assert_eq!(all[4]["frame"]["index"], 0);
    assert_eq!(all[5]["frame"]["index"], 1);
    assert_eq!(all[11]["event"]["data"]["reason"]["kind"], "completed");
}

/// 错误路径——回合中错误=abandoned+step/end+system/message+turn/end(error)；
/// 无回合错误=仅 system/message。
#[test]
fn 错误路径两态() {
    let mut state = state("m", None);
    // 无回合错误：仅 system/message（fallback 渲染人话卡）
    let (frames, _) = state.apply(&ServerMessage::Error { text: "装配异常".into() });
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["event"]["type"], "system/message");
    assert_eq!(frames[0]["event"]["surfaceOp"], "append");
    // 回合中错误：四件套（attempt 弃置+步/回合闭合+人话卡）
    state.apply(&ServerMessage::UserMessage { text: "hi".into() });
    state.apply(&ServerMessage::AssistantDelta { text: "写".into() });
    let (frames, _) = state.apply(&ServerMessage::Error { text: "回合失败".into() });
    let kinds: Vec<String> = frames
        .iter()
        .map(|f| {
            if f["type"] == "event" {
                f["event"]["type"].as_str().unwrap_or("").to_string()
            } else {
                format!("stream/{}", f["frame"]["type"].as_str().unwrap_or(""))
            }
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["stream/end", "step/end", "system/message", "turn/end"],
        "错误四件套：{kinds:?}"
    );
    assert_eq!(frames[0]["frame"]["outcome"]["kind"], "abandoned");
    assert_eq!(frames[3]["event"]["data"]["reason"]["kind"], "error");
}

/// 快照基线——重放环=已发持久事件、cursor=已分配最大 seq、
/// assistantStream 基线必带（transport.ts 缺失即抛错）。
#[test]
fn 快照重放环与流基线() {
    let mut state = state("m", None);
    for msg in turn_messages() {
        state.apply(&msg);
    }
    let snapshot = state.snapshot(Some("s-1"));
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["cursor"], 7, "cursor=已分配最大 seq");
    assert_eq!(snapshot["records"].as_array().map(Vec::len), Some(8));
    assert_eq!(snapshot["header"]["id"], "s-1");
    assert!(snapshot["assistantStream"]["revision"].as_u64().is_some());
    // address 缺席回退 create 登记的会话 id
    state.test_set_session_id(Some("created-9".into()));
    assert_eq!(state.snapshot(None)["header"]["id"], "created-9");
}

/// 投影面（批⑬）：未铸会话=空投影块+空列表行（不猜 id，不伪造块）；
/// 铸会话后基线带 sessionListMetadata（blank=true，title 缺席=对端
/// session.untitled 兜底）。
#[test]
fn 投影基线与列表行_未铸与铸后() {
    let mut ws = state("test-model", None);
    assert_eq!(ws.control_baseline()["value"]["projections"], serde_json::json!({}));
    assert!(ws.session_summaries().is_empty(), "未铸会话=空列表");
    ws.test_set_session_id(Some("s-1".into()));
    let baseline = ws.control_baseline();
    let block = &baseline["value"]["projections"]["s-1"];
    assert_eq!(block["values"]["sessionListMetadata"]["blank"], true, "无回合=blank");
    assert!(block["values"].get("title").is_none(), "无标题不伪造");
    let rows = ws.session_summaries();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sessionId"], "s-1");
    assert_eq!(rows[0]["blank"], true);
    assert_eq!(rows[0]["projections"]["kind"], "sequenced");
}

/// 投影面：首条用户消息派生标题（首行截 40 字）+ 控制投影帧出料
/// （title+sessionListMetadata 两键，seq 与 user/message 事件对齐）；
/// 后续消息不换名；无会话 id 不出控制帧。
#[test]
fn 投影live帧_标题派生与控制出料() {
    let mut ws = state("test-model", Some("req-1"));
    ws.test_set_session_id(Some("s-1".into()));
    let long_line = "一".repeat(50);
    let (frames, control) = ws.apply(&ServerMessage::UserMessage { text: format!("{long_line}\n第二行") });
    assert!(!frames.is_empty(), "会话帧照常产出");
    assert_eq!(control.len(), 1, "一条控制投影帧");
    let title = {
        let frame = &control[0];
        assert_eq!(frame["type"], "projection");
        assert_eq!(frame["sessionId"], "s-1");
        assert_eq!(frame["seq"], frames[0]["event"]["seq"], "投影 cut 与 user/message 事件对齐");
        assert_eq!(frame["changes"][0]["key"], "title");
        let title = frame["changes"][0]["val"].as_str().expect("标题在").to_string();
        assert_eq!(title.chars().count(), 41, "40 字+省略号：{title}");
        assert!(!title.contains('\n'), "单行派生");
        assert_eq!(frame["changes"][1]["key"], "sessionListMetadata");
        assert_eq!(frame["changes"][1]["val"]["blank"], false);
        assert!(frame["changes"][1]["val"]["lastPromptAt"].as_u64().is_some());
        title
    };
    // 第二条消息不换名、不出 title 键（只推 metadata）
    let (_frames2, control2) = ws.apply(&ServerMessage::UserMessage { text: "改名尝试".into() });
    assert_eq!(control2.len(), 1);
    let changes = control2[0]["changes"].as_array().expect("changes 在");
    assert!(changes.iter().all(|c| c["key"] != "title"), "标题只在首条消息派生");
    // 列表行带真名
    assert_eq!(ws.session_summaries()[0]["projections"]["values"]["title"], title);
    // 无会话 id：不出控制帧（不猜 id）
    let mut anon = state("test-model", None);
    let (_f, c) = anon.apply(&ServerMessage::UserMessage { text: "你好".into() });
    assert!(c.is_empty(), "无会话 id 不出控制帧");
}
