//! web.v1 契约对表面（W1 批）：错误码词表 + unary 方法注册表 + fail-visible
//! 兜底点名。契约=docs/接口/web-v1/{规格.md,openapi.yaml,websocket.md}（冻结稿
//! v1.0，2026-11-19 云枢产出）。
//!
//! # 职责分家
//! - 词表：错误码三字段统一（code/message/details，message 恒中文人话）——
//!   rpc.rs/gateway.rs 的 err 体全部经 [`err_body`] 成形，词表常量防拼错；
//! - 方法注册表：契约 §7 域索引的 unary 方法全集与状态三档（已接/冻结待接/
//!   后续）。未实现方法的 not-implemented 兜底按注册表**点名状态与去向**，
//!   不许伪造值（契约 §0.3 禁假控件）；
//! - 本文件不写行为：分派仍在 rpc.rs/gateway.rs，这里只供查询。

use serde_json::{json, Value};

/// 错误码（契约 §2.2 词表；v1 冻结）。词表全量落位防拼错；未消费码属后续批
///（W2-W6 逐批接线），allow 死码=契约冻结面先行、非疏漏。
#[allow(dead_code)]
pub mod code {
    /// 信封/参数形状非法。
    pub const BAD_REQUEST: &str = "gateway/bad-request";
    /// 方法未接（fail-visible，不许伪造值）。
    pub const NOT_IMPLEMENTED: &str = "gateway/not-implemented";
    /// mux 帧 JSON 非法 / 上行 type 不认识。
    pub const BAD_FRAME: &str = "gateway/bad-frame";
    /// 回合闸门占用。
    pub const AGENT_BUSY: &str = "session/agent-busy";
    /// 无进行中回合。
    pub const NO_ACTIVE_TURN: &str = "session/no-active-turn";
    /// prompt 缺 requestId / mode 非法 / 正文空白。
    pub const PROMPT_INVALID: &str = "session/prompt-invalid";
    /// 修订号过期。
    pub const REVISION_CONFLICT: &str = "config/revision-conflict";
    /// 配置段深度校验失败（details.keys）。
    pub const CONFIG_VALIDATION: &str = "config/validation";
    /// 试图读回 secret 明文（恒拒绝）。
    pub const CREDENTIALS_WRITE_ONLY: &str = "credentials/write-only";
    /// 任务板状态机违例（details 含 from/op）。
    pub const TEAM_STATE: &str = "team/state";
    /// 非本人操作。
    pub const TEAM_NOT_OWNER: &str = "team/not-owner";
    /// 写审分离拒绝（板层强制）。
    pub const TEAM_SELF_REVIEW: &str = "team/self-review";
    /// actor/owner 不在名册。
    pub const TEAM_UNKNOWN_MEMBER: &str = "team/unknown-member";
    /// 定时规格三选一必填恰一 / 间隔低于下限。
    pub const SCHEDULE_SPEC: &str = "schedule/spec";
    /// 删除不存在的调度（details.entries 列现有 id）。
    pub const SCHEDULE_UNKNOWN_ID: &str = "schedule/unknown-id";
    /// market 插件权限未开闸。
    pub const PLUGIN_RESTRICTED: &str = "plugin/restricted";
    /// manifest ui kind=page 二期预留拒装。
    pub const PLUGIN_UI_PAGE_REJECTED: &str = "plugin/ui-page-rejected";
    /// 远程管理动作越权。
    pub const FORBIDDEN: &str = "system/forbidden";
}

/// 契约方法状态三档（契约 §0.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 现有代码已实现，形状与契约一致。
    Wired,
    /// 冻结形状，实施批按此落码。
    FrozenPending,
    /// 依赖未开闸缝，只定占位语义与错误码。
    Later,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl Status {
    /// 状态中文标注（not-implemented 点名用）。
    fn label(self) -> &'static str {
        match self {
            Status::Wired => "已接",
            Status::FrozenPending => "冻结待接（实施批排期中）",
            Status::Later => "后续（依赖未开闸缝，本期不开）",
        }
    }
}

/// 契约 unary 方法注册表（openapi.yaml x-method 全集；契约 §7 域索引同源）。
/// 流端点（session/follow 等）不在本表——mux 面分派在 gateway.rs。
const METHODS: &[(&str, Status)] = &[
    // session 域（契约 §8，全部已接）
    ("session/list", Status::Wired),
    ("session/create", Status::Wired),
    ("session/prompt", Status::Wired),
    ("session/cancel", Status::Wired),
    ("session/page", Status::Wired),
    ("session/modelCatalog", Status::Wired),
    // 会话恢复（域-会话命令与技能.md §1.1-3 冻结形状；多会话步 2 已接）
    ("session/open", Status::Wired),
    // commands 域（契约 §9.1；W2 已接）
    ("commands/list", Status::Wired),
    ("commands/execute", Status::Wired),
    // skills 域（契约 §9.2）
    ("skills/list", Status::FrozenPending),
    ("skills/get", Status::FrozenPending),
    // capabilities（契约 §9.3）
    ("capabilities/list", Status::FrozenPending),
    // team / tasks 域（域-团队与任务.md；W3 已接）
    ("team/spawn", Status::Wired),
    ("team/message", Status::Wired),
    ("team/roster", Status::Wired),
    ("team/kick", Status::Wired),
    ("tasks/create", Status::Wired),
    ("tasks/claim", Status::Wired),
    ("tasks/submit", Status::Wired),
    ("tasks/review", Status::Wired),
    ("tasks/reassign", Status::Wired),
    ("tasks/list", Status::Wired),
    // schedule 域（域-配置凭据与插件.md §3；W4 已接）
    ("schedule/create", Status::Wired),
    ("schedule/list", Status::Wired),
    ("schedule/delete", Status::Wired),
    // plugins 域（域-配置凭据与插件.md §4；W6 已接——tools 系方案 A 诚实面）
    ("plugins/list", Status::Wired),
    ("plugins/enable", Status::Wired),
    ("plugins/config", Status::Wired),
    ("plugins/tools", Status::Wired),
    // market（后续：权限未开闸）
    ("market/list", Status::Later),
    ("market/install", Status::Later),
    // settings / credentials 域（域-配置凭据与插件.md §1-2；W5 已接）
    ("settings/sections", Status::Wired),
    ("settings/form", Status::Wired),
    ("settings/write", Status::Wired),
    ("credentials/list", Status::Wired),
    ("credentials/set", Status::Wired),
    ("credentials/delete", Status::Wired),
    // devices / system 域（契约 §11）
    ("devices/current", Status::Wired),
    ("system/info", Status::Wired),
    ("system/health", Status::Wired),
];

/// typert 线约定解包：payload=`{args:{<参数名>:<值>}}`（单参方法参数名 'request'
/// 为惯例；无参方法 args={}）。兼容直接 payload 形态（探针/直调用）。
pub(crate) fn request_of(payload: &Value) -> &Value {
    payload
        .get("args")
        .and_then(|args| args.get("request"))
        .unwrap_or(payload)
}

/// 查契约状态（未注册方法=None——契约外方法，兜底文案不冒充契约内）。
pub fn status_of(method: &str) -> Option<Status> {
    METHODS
        .iter()
        .find(|(name, _)| *name == method)
        .map(|(_, status)| *status)
}

/// 契约注册表全集（测试与对表用；顺序=注册表顺序）。
#[cfg(test)]
pub(crate) fn all_methods() -> &'static [(&'static str, Status)] {
    METHODS
}

/// not-implemented 兜底信封（契约 §0.3 fail-visible）：契约内方法点名状态，
/// 契约外方法直说契约外——两者都不伪造值。
pub fn not_implemented(method: &str) -> Value {
    let message = match status_of(method) {
        Some(status) => format!("RPC 方法 {method} 未实现（web.v1 {}）", status.label()),
        None => format!("RPC 方法 {method} 不在 web.v1 契约注册表内，未实现"),
    };
    json!({"ok": false, "error": err_body(code::NOT_IMPLEMENTED, &message)})
}

/// 错误体（契约 §2.2 统一三字段；details 只放定位所需结构化字段）。
pub fn err_body(code: &str, message: &str) -> Value {
    err_body_details(code, message, serde_json::Map::new())
}

/// 带定位字段的错误体（如 revision-conflict 的 current、unknown-id 的 entries）。
pub fn err_body_details(code: &str, message: &str, details: serde_json::Map<String, Value>) -> Value {
    json!({"code": code, "message": message, "details": Value::Object(details)})
}

/// 错误 result 信封（HTTP 200 + 信封内 ok:false——契约 §2.1 业务失败一律 200）。
pub fn err_envelope(code: &str, message: &str) -> Value {
    json!({"ok": false, "error": err_body(code, message)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ServerMessage;
    use axum::extract::State;
    use axum::http::StatusCode;
    use crate::server::WebState;
    use crate::turns::TurnRunner;
    use crate::bridge::WireBridge;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use yunyou::Context;

    /// 测试态（与 rpc.rs/gateway.rs 测试同构：无装配 root，走失败路径面）。
    pub(crate) fn state() -> Arc<WebState> {
        let (sender, _) = broadcast::channel(16);
        Arc::new(WebState {
            token: "t".into(),
            model: "test-model".into(),
            turns: Arc::new(TurnRunner::new(Arc::new(Context::new), "test-model".into())),
            broadcast: sender,
            session_dir: PathBuf::from("/tmp/yunyou-contract-test-sessions"),
            wire: Arc::new(WireBridge::new("test-model".into())),
            scheduler: Arc::new(yunyou_plugin_sdk::scheduler::Scheduler::new(|| 1_000_000)),
            config_store: Arc::new(crate::config_store::ConfigStore::new(std::path::PathBuf::from("/tmp/yunyou-test-yunyou.toml"))),
            loopback: true,
        })
    }

    /// 错误码词表形态冻结：slash 形、gateway 族四码在位。
    #[test]
    fn 错误码词表形态() {
        for word in [
            code::BAD_REQUEST,
            code::NOT_IMPLEMENTED,
            code::BAD_FRAME,
            code::AGENT_BUSY,
            code::REVISION_CONFLICT,
            code::CREDENTIALS_WRITE_ONLY,
            code::TEAM_SELF_REVIEW,
            code::SCHEDULE_SPEC,
            code::PLUGIN_RESTRICTED,
            code::FORBIDDEN,
        ] {
            assert!(word.contains('/'), "错误码应 slash 形：{word}");
            assert!(!word.contains(char::is_whitespace), "错误码不含空白：{word}");
        }
    }

    /// 注册表状态查询：已接/冻结待接/后续三档点名，契约外返回 None。
    #[test]
    fn 注册表三档状态查询() {
        assert_eq!(status_of("session/prompt"), Some(Status::Wired));
        assert_eq!(status_of("skills/list"), Some(Status::FrozenPending));
        assert_eq!(status_of("market/install"), Some(Status::Later));
        assert_eq!(status_of("nope/x"), None);
    }

    /// not-implemented 兜底：契约内点名状态、契约外点名契约外；两态 code 一致。
    #[test]
    fn not_implemented点名() {
        let frozen = not_implemented("skills/list");
        assert_eq!(frozen["error"]["code"], code::NOT_IMPLEMENTED);
        assert!(
            frozen["error"]["message"].as_str().unwrap_or("").contains("冻结待接"),
            "契约内应点名状态：{frozen}"
        );
        assert!(!frozen["error"]["message"].as_str().unwrap_or("").contains("session/prompt"));
        let outside = not_implemented("made/up");
        assert_eq!(outside["error"]["code"], code::NOT_IMPLEMENTED);
        assert!(
            outside["error"]["message"].as_str().unwrap_or("").contains("不在 web.v1 契约"),
            "契约外应直说：{outside}"
        );
        // 信封三字段形状冻结：code/message/details 齐
        let body = &outside["error"];
        assert!(body["message"].is_string() && body["details"].is_object(), "三字段齐：{body}");
    }

    /// err_body_details：details 定位字段透传（revision-conflict/unknown-id 形态）。
    #[test]
    fn details定位字段透传() {
        let mut details = serde_json::Map::new();
        details.insert("current".into(), json!(3));
        let body = err_body_details(code::REVISION_CONFLICT, "配置已在别处更新", details);
        assert_eq!(body["details"]["current"], 3, "{body}");
        assert_eq!(body["code"], code::REVISION_CONFLICT);
    }

    /// 注册表与 openapi.yaml x-method 全集对表：数量与三档计数锚点冻结
    /// （契约变更时本测试红=提醒同步本表与契约）。
    #[test]
    fn 注册表与契约全集对表锚点() {
        let all = all_methods();
        let wired = all.iter().filter(|(_, s)| *s == Status::Wired).count();
        let frozen = all.iter().filter(|(_, s)| *s == Status::FrozenPending).count();
        let later = all.iter().filter(|(_, s)| *s == Status::Later).count();
        // 锚点：openapi 已接 8 方法（session 6+commands 2）+session/open（域细则
        // 冻结形状，openapi v1.1 已补录）+team/tasks 域 9 方法（W3）；待接 20；后续 2（market）
        assert_eq!((wired, frozen, later), (34, 4, 2), "契约方法计数漂移，先同步契约再改表：{all:?}");
        // 已接方法必须确有分派（rpc.rs 契约面行为测试锁定形状，此处锁无遗漏）
        for (name, status) in all {
            if *status == Status::Wired {
                assert!(
                    name.starts_with("session/")
                        || name.starts_with("commands/")
                        || name.starts_with("team/")
                        || name.starts_with("tasks/")
                        || name.starts_with("schedule/")
                        || name.starts_with("settings/")
                        || name.starts_with("credentials/")
                        || name.starts_with("plugins/")
                        || name.starts_with("system/")
                        || name == "devices/current",
                    "已接面现态=session+commands+team+tasks 域：{name}"
                );
            }
        }
    }

    /// 兜底信封经真实 unary 通道回流：未接契约方法（schedule/list）应
    /// ok:false 且点名冻结待接（fail-visible 全量点名，不伪造值）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 未接方法经unary兜底点名() {
        let state = state();
        let body = r#"{"type":"client-request","rpcId":"r9","method":"skills/list","payload":{}}"#;
        let resp = crate::rpc::rpc_unary(State(state.clone()), axum::body::Bytes::from(body)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.expect("体可读");
        let value: Value = serde_json::from_slice(&bytes).expect("合法 JSON");
        assert_eq!(value["type"], "server-response");
        assert_eq!(value["rpcId"], "r9", "rpcId 原样回带");
        assert_eq!(value["result"]["ok"], false);
        assert_eq!(value["result"]["error"]["code"], code::NOT_IMPLEMENTED);
        assert!(value["result"]["error"]["message"].as_str().unwrap_or("").contains("冻结待接"));
        // ServerMessage 词汇不受影响（旧 /ws 面回归锚：tag 形态仍 kebab-case）
        let tag = serde_json::to_string(&ServerMessage::Error { text: "x".into() }).unwrap();
        assert!(tag.contains(r#""type":"error""#), "{tag}");
    }
}
