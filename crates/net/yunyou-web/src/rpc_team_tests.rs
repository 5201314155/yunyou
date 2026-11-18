//! rpc_team 测试（外挂：#[path] 先例，源文件行数纪律——wire_tests 同款）。

use crate::commands::tests::setup as cmd_setup;
    use crate::rpc::rpc_unary;
    use crate::server::WebState;
    use axum::extract::State;
    use axum::http::StatusCode;
    use serde_json::Value;
    use std::sync::Arc;
    use yunyou::ServiceId;
    use yunyou_tools::RegistryService;
    use yunyou_team::TeamPlugin;

    /// 装配：真 TeamPlugin（SessionStore 由 cmd_setup 的 SessionPlugin 提供；
    /// RegistryService 显式补；不装 Llm——本测试不拉成员回合）。返回
    /// (state, 会话文件路径)——TeamPlugin config.path 需真实会话路径。
    pub(crate) async fn team_setup() -> (Arc<WebState>, std::path::PathBuf) {
        let (state, session_file) = cmd_setup().await;
        let root = state.turns.root();
        let registry: Arc<dyn RegistryService> = yunyou_tools::ToolsRegistry::new();
        root.provide(ServiceId::of::<dyn RegistryService>(), registry)
            .expect("provide registry");
        let handle = root.plugin(
            TeamPlugin,
            serde_json::json!({"path": session_file.display().to_string()}),
        );
        // 装载到 Active（与 team.rs 集成测试同款轮询；400×5ms 上限）
        let mut active = false;
        for _ in 0..400 {
            if handle.state() == yunyou::FiberState::Active {
                active = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(active, "TeamPlugin 应在限时内装载");
        (state, session_file)
    }

    /// 经真实 unary 通道发一封 client-request。
    async fn call(state: &Arc<WebState>, method: &str, payload: &str) -> Value {
        let body = format!(
            r#"{{"type":"client-request","rpcId":"t1","method":"{method}","payload":{payload}}}"#
        );
        let resp = rpc_unary(State(state.clone()), axum::body::Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("体可读");
        serde_json::from_slice(&bytes).expect("合法 JSON")
    }

    /// 无团队插件装配（cmd_setup 不装 TeamPlugin）：全域人话拒绝不伪造。
    #[tokio::test(flavor = "multi_thread")]
    async fn 服务缺失面人话拒绝() {
        let (state, _) = cmd_setup().await;
        for method in ["team/roster", "tasks/list"] {
            let resp = call(&state, method, "{}").await;
            assert_eq!(resp["result"]["ok"], false, "{method}: {resp}");
            assert!(resp["result"]["error"]["message"].as_str().unwrap_or("").contains("团队服务未就绪"));
        }
    }

    /// tasks 域生命周期：create→claim→submit→review 全链 + tasks/changed
    /// 事件广播 + 错误映射（self-review/not-owner/state/unknown）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 任务板生命周期与事件广播() {
        let (state, _) = team_setup().await;
        let mut events = state.wire.subscribe();
        // create
        let value = call(&state, "tasks/create",
            r#"{"args":{"request":{"title":"写作","description":"写完验收"}}}"#)
            .await["result"]["value"].clone();
        let task_id = value["taskId"].as_u64().expect("taskId 数值");
        assert_eq!(task_id, 1);
        // tasks/changed 事件已广播（pending）
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
            .await.expect("事件在").expect("流在");
        assert_eq!(event.0, crate::bridge::StreamKind::Events);
        assert_eq!(event.2["type"], "tasks/changed");
        assert_eq!(event.2["state"], "pending");
        // claim（lead 代派）
        let value = call(&state, "tasks/claim", &format!(r#"{{"args":{{"request":{{"taskId":{task_id},"owner":"lead"}}}}}}"#))
            .await["result"]["value"].clone();
        assert_eq!(value["claimed"], true);
        // submit（actor=lead 与 owner 同人→合法；再 submit→state）
        let value = call(&state, "tasks/submit", &format!(r#"{{"args":{{"request":{{"taskId":{task_id}}}}}}}"#))
            .await["result"]["value"].clone();
        assert_eq!(value["submitted"], true);
        // 重复 submit → team/state
        let resp = call(&state, "tasks/submit", &format!(r#"{{"args":{{"request":{{"taskId":{task_id}}}}}}}"#)).await;
        assert_eq!(resp["result"]["error"]["code"], "team/state", "{resp}");
        // review：lead 自审自己的提交 → 板层强制 team/self-review（lead 不可绕过）
        let resp = call(&state, "tasks/review", &format!(r#"{{"args":{{"request":{{"taskId":{task_id},"approve":true}}}}}}"#)).await;
        assert_eq!(resp["result"]["error"]["code"], "team/self-review", "{resp}");
        // tasks/list 全量投影：submittedBy 在列
        let value = call(&state, "tasks/list", "{}").await["result"]["value"].clone();
        let items = value["items"].as_array().expect("items 在");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["state"], "submitted");
        assert_eq!(items[0]["submittedBy"], "lead");
        assert_eq!(items[0]["createdBy"], "lead");
        // roster：未拉人=诚实空表（lead 身份在 hub 不进名册投影——名册=
        // 被拉成员观测面，域细则 §1.1）
        let value = call(&state, "team/roster", "{}").await["result"]["value"].clone();
        assert_eq!(value["items"].as_array().map(Vec::len), Some(0), "未拉人空名册：{value}");
        // 载荷缺陷：缺 title/缺 taskId/缺 approve → bad-request
        for (method, payload) in [
            ("tasks/create", r#"{"args":{"request":{"description":"x"}}}"#),
            ("tasks/claim", r#"{"args":{"request":{}}}"#),
            ("tasks/review", r#"{"args":{"request":{"taskId":1}}}"#),
        ] {
            let resp = call(&state, method, payload).await;
            assert_eq!(resp["result"]["error"]["code"], "gateway/bad-request", "{method}: {resp}");
        }
    }

    /// team/message：chat 受理（logId/recipients 回显）；report 代发人话拒绝
    /// （身份派发模型，备案云枢）；未知成员 → team/unknown-member。
    #[tokio::test(flavor = "multi_thread")]
    async fn 成员消息与身份纪律() {
        let (state, _) = team_setup().await;
        // chat → lead 自身信箱受理（recipients≥1）
        let value = call(&state, "team/message",
            r#"{"args":{"request":{"to":"lead","body":"你好"}}}"#)
            .await["result"]["value"].clone();
        assert_eq!(value["accepted"], true, "{value}");
        assert!(value["logId"].is_u64());
        // report 代发拒绝
        let resp = call(&state, "team/message",
            r#"{"args":{"request":{"to":"lead","body":"x","kind":"report"}}}"#).await;
        assert_eq!(resp["result"]["error"]["code"], "team/state", "{resp}");
        assert!(resp["result"]["error"]["message"].as_str().unwrap_or("").contains("不可代发"));
        // 未知成员 → team/unknown-member
        let resp = call(&state, "team/message",
            r#"{"args":{"request":{"to":"ghost","body":"hi"}}}"#).await;
        assert_eq!(resp["result"]["error"]["code"], "team/unknown-member", "{resp}");
    }
