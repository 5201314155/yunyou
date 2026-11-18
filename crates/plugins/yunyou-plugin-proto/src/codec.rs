//! JSON Lines 帧编解码（纪律复用 ipc-types：单行帧+\n 分隔+未知 type 拒绝）。

use crate::messages::PluginProtoFrame;

/// 帧分隔符（JSON Lines 单行帧；载荷内禁含 `\n`——编码端校验）。
pub const FRAME_SEPARATOR: char = '\n';

/// 编码一帧为单行 JSON（含 type 标签；不含分隔符——调用方补 `\n`）。
pub fn encode_frame(frame: &PluginProtoFrame) -> serde_json::Result<String> {
    let line = serde_json::to_string(frame)?;
    debug_assert!(!line.contains(FRAME_SEPARATOR), "单行帧不含分隔符");
    Ok(line)
}

/// 解码一行 JSON 为帧（**未知 type 标签=Err**——封闭枚举反序列化端同铁律，
/// 私加消息类型在双端都过不了）。
pub fn decode_frame(line: &str) -> serde_json::Result<PluginProtoFrame> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;

    /// v1.1 投递对往返无损（加法演进；旧端未知标签=拒——本端双向兼容）。
    #[test]
    fn 投递对往返无损() {
        let frames = vec![
            PluginProtoFrame::DeliverMessage(DeliverMessage {
                deliver_id: "dlv-1".into(),
                session_id: None,
                text: "[定时触发 sched-x] 提醒".into(),
            }),
            PluginProtoFrame::DeliverMessage(DeliverMessage {
                deliver_id: "dlv-2".into(),
                session_id: Some("sess-1".into()),
                text: "会话级".into(),
            }),
            PluginProtoFrame::DeliverAck(DeliverAck {
                deliver_id: "dlv-1".into(),
                ok: true,
                detail: "已入当前会话注入队列".into(),
            }),
            PluginProtoFrame::DeliverAck(DeliverAck {
                deliver_id: "dlv-3".into(),
                ok: false,
                detail: "注入队列投递失败".into(),
            }),
        ];
        for frame in frames {
            let line = encode_frame(&frame).unwrap();
            assert!(!line.contains('\n'));
            let decoded = decode_frame(&line).unwrap();
            assert_eq!(decoded, frame, "往返无损: {line}");
        }
    }

    /// 八变体全量往返无损（MVP 词汇全覆盖；单行形态）。
    #[test]
    fn 八变体全量往返无损() {
        let frames = vec![
            PluginProtoFrame::Hello(Hello {
                protocol_version: PROTOCOL_VERSION,
                capabilities: vec![],
            }),
            PluginProtoFrame::HelloOk(HelloOk {
                protocol_version: PROTOCOL_VERSION,
                plugin_name: "hello".into(),
            }),
            PluginProtoFrame::HelloErr(HelloErr { reason: "协议版本不兼容".into() }),
            PluginProtoFrame::ToolsList(ToolsList),
            PluginProtoFrame::ToolsListOk(ToolsListOk {
                tools: vec![ToolDescriptor {
                    name: "hello_greet".into(),
                    description: "打招呼".into(),
                    parameters: r#"{"type":"object","properties":{}}"#.into(),
                    output_schema: None,
                    timeout_ms: Some(5000),
                    is_concurrency_safe: false,
                }],
            }),
            PluginProtoFrame::ToolInvoke(ToolInvoke {
                invoke_id: "inv-1".into(),
                name: "hello_greet".into(),
                args: r#"{"name":"小云"}"#.into(),
            }),
            PluginProtoFrame::ToolResult(ToolResult {
                invoke_id: "inv-1".into(),
                outcome: ToolOutcome::Ok(ToolResultOk {
                    value: r#"{"greeting":"你好，小云"}"#.into(),
                }),
            }),
            PluginProtoFrame::Shutdown(Shutdown),
        ];
        for frame in frames {
            let line = encode_frame(&frame).expect("编码应成功");
            assert!(!line.contains('\n'), "单行帧不含分隔符");
            assert_eq!(decode_frame(&line).expect("解码应成功"), frame, "往返无损: {line}");
        }
    }

    /// 错误结果变体（outcome err 分支）往返无损。
    #[test]
    fn 错误结果往返() {
        let frame = PluginProtoFrame::ToolResult(ToolResult {
            invoke_id: "inv-2".into(),
            outcome: ToolOutcome::Err(ToolResultErr {
                code: "plugin/args".into(),
                message: "缺少参数 name".into(),
                hint: Some("补 name 字段".into()),
            }),
        });
        let line = encode_frame(&frame).expect("编码应成功");
        assert_eq!(decode_frame(&line).expect("解码应成功"), frame);
    }

    /// 暴露面铁律（类型化执行）：未知 type 标签=解码拒绝（L2.1 预留词汇
    /// credentials_get 在 MVP 侧也拒绝——开闸前双端都过不了封闭枚举）。
    #[test]
    fn 未知消息类型解码拒绝() {
        let line = r#"{"type":"credentials_get","name":"openai_key"}"#;
        assert!(decode_frame(line).is_err(), "未列消息类型必须拒绝");
        let line = r#"{"type":"tools_changed","tools":[]}"#;
        assert!(decode_frame(line).is_err(), "L2.1 预留词汇开闸前拒绝");
        let line = r#"{"type":"tool.cancel","invoke_id":"x"}"#;
        assert!(decode_frame(line).is_err(), "点号变体名拒绝");
    }

    /// tagged 枚举线上形态：type 标签=snake_case 变体名（跨语言契约可读——
    /// Python/Node 插件端按此实现）。
    #[test]
    fn 线上形态带类型标签() {
        let line = encode_frame(&PluginProtoFrame::ToolsList(ToolsList)).expect("编码");
        assert!(line.contains(r#""type":"tools_list""#), "{line}");
        let line = encode_frame(&PluginProtoFrame::ToolInvoke(ToolInvoke {
            invoke_id: "i".into(),
            name: "t".into(),
            args: "{}".into(),
        }))
        .expect("编码");
        assert!(line.contains(r#""type":"tool_invoke""#), "{line}");
    }

    /// 缺 type 标签=拒绝（非帧形态数据不静默吞）。
    #[test]
    fn 缺标签拒绝() {
        assert!(decode_frame(r#"{"foo":1}"#).is_err());
        assert!(decode_frame("not json").is_err());
        assert!(decode_frame("").is_err());
    }
}
