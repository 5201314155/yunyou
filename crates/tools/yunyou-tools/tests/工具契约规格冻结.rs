//! S4 工具契约规格冻结断言（契约①⑥，吸收 P0-9 五点合一）：
//! 1. **白名单纪律**：`RegistryService::schemas()` 只投 name/description/parameters
//!    三元组，timeout_ms/is_concurrency_safe/presentation_meta 的值**零出现**
//!    （键集合锁死＋探针特征值值级双断言，对标官方 contracts.ts:63-84）；
//! 2. **默认值断言**：未覆盖新钩子的工具走安全默认（超时 None／串行 false／
//!    无卡片声明／render=统一渲染）；未注册名经访问器回落保守值；
//! 3. **收敛纪律**：output_schema 的 S4 收敛声明在位（源码文本断言锁死
//!    rustdoc，防「规格悄悄改」）＋我辖区五内置工具示范已填（bash 归
//!    云铠辖区，不代不断言）；
//! 4. **访问器直取**：注册后 per-tool 声明即时经注册表可见（渲染/调度侧通道）；
//! 5. **render 覆盖**：探针覆盖生效＋grep 匹配行平铺示范＋错误路径回落；
//! 6. **hint serde 兼容（v1→v1.1 加法演进）**：旧数据缺 hint 可读＋新数据带 hint
//!    往返无损（serde default 行为锁死，云镜 P2-A1）。

use std::path::Path;
use std::sync::Arc;
use yunyou_tools::builtin::cwd::CwdService;
use yunyou_tools::builtin::{builtin_tools, search::GrepTool};
use yunyou_tools::contract::{PresentationMeta, ToolDefinition, ToolError, ToolExec, ToolOutcome, render_outcome};
use yunyou_tools::registry::{RegistryService, ToolsRegistry};
use yunyou_util::BoxFut;

/// 全量覆盖的示范工具（模拟 S4 后新工具：五点全覆盖）。
struct FullProbe;

impl ToolDefinition for FullProbe {
    fn name(&self) -> &str {
        "full_probe"
    }

    fn description(&self) -> &str {
        "全量覆盖示范"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({"type": "object"}))
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(99_999)
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn presentation_meta(&self) -> Option<PresentationMeta> {
        Some(PresentationMeta::new("探针", "演示卡片", "probe"))
    }

    fn render(&self, _outcome: &ToolOutcome) -> String {
        "PROBE_RENDER".into()
    }

    fn execute(&self, _args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        Box::pin(async { ToolOutcome::ok(serde_json::json!({"ok": true})) })
    }
}

/// 裸工具：只实现四件套（模拟未声明新钩子的存量/新工具，走全部安全默认）。
struct BareProbe;

impl ToolDefinition for BareProbe {
    fn name(&self) -> &str {
        "bare_probe"
    }

    fn description(&self) -> &str {
        "最小实现"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    fn execute(&self, _args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        Box::pin(async { ToolOutcome::ok(serde_json::json!({"ok": true})) })
    }
}

/// 场景一：白名单纪律——schemas() 对六内置＋全量探针只投三元组；元数据值零出现。
#[test]
fn schemas白名单只投三元组且元数据零出现() {
    let registry = ToolsRegistry::new();
    // task-20：默认装配形态=后台注册表在位（三管理工具注册进模型面）
    let background = Some((Arc::new(yunyou_tools::tasks::TaskRegistry::new()), 60u64));
    for tool in builtin_tools(Arc::from(Path::new(".")), None, background) {
        registry.register(tool).expect("内置工具注册");
    }
    registry.register(Arc::new(FullProbe)).expect("探针注册");
    let schemas = registry.schemas();
    assert_eq!(schemas.len(), 11, "十内置（bash/read/write/edit/glob/grep/apply_patch/task_output/task_stop/task_list）＋探针（task-20 后台三工具演进随批对齐实测）");
    for schema in &schemas {
        let json = serde_json::to_value(schema).expect("schema 可序列化");
        let mut keys: Vec<&str> = json.as_object().expect("schema 为对象").keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["description", "name", "parameters"], "模型面键集合锁死（集合语义；serde_json 默认按字母序重排，声明序不作保证）");
        // 值级零出现：探针的特征值（超时 99999／卡片文案／覆盖渲染串）不得泄漏进模型面
        let text = json.to_string();
        assert!(!text.contains("99999"), "timeout_ms 值泄漏进模型面");
        assert!(!text.contains("探针") && !text.contains("演示卡片"), "presentationMeta 值泄漏进模型面");
        assert!(!text.contains("PROBE_RENDER"), "render 覆盖痕迹泄漏进模型面");
    }
}

/// 场景二：默认值安全默认（未覆盖新钩子的工具走保守默认；未注册名回落）。
#[test]
fn 新钩子默认值安全() {
    let bare = BareProbe;
    assert_eq!(bare.timeout_ms(), None, "默认不限时＝用全局 tool_timeout_secs");
    assert!(!bare.is_concurrency_safe(), "默认false＝不假定可并发");
    assert!(bare.presentation_meta().is_none(), "默认无卡片声明");
    // render 默认＝现有统一渲染（Ok  pretty JSON／Err 错误串两条路径）
    let ok = ToolOutcome::ok(serde_json::json!({"a": 1}));
    assert_eq!(bare.render(&ok), render_outcome(&ok));
    let err = ToolOutcome::err("tool/x", "坏");
    assert_eq!(bare.render(&err), render_outcome(&err));
    // 访问器对未注册名的回落（bare_probe 故意不注册进表）
    let registry = ToolsRegistry::new();
    registry.register(Arc::new(FullProbe)).expect("注册探针");
    assert_eq!(registry.timeout_ms("bare_probe"), None);
    assert!(!registry.is_concurrency_safe("bare_probe"));
    assert!(registry.presentation_meta("bare_probe").is_none());
}

/// 场景三：registry 访问器直取（注册后 per-tool 声明即时可见，渲染/调度侧通道）。
#[test]
fn registry访问器直取元数据() {
    let registry = ToolsRegistry::new();
    registry.register(Arc::new(FullProbe)).expect("注册探针");
    assert_eq!(registry.timeout_ms("full_probe"), Some(99_999));
    assert!(registry.is_concurrency_safe("full_probe"));
    let meta = registry.presentation_meta("full_probe").expect("有卡片声明");
    assert_eq!(meta.title, "探针");
    assert_eq!(meta.icon, "probe");
}

/// 场景四：per-tool render 覆盖（FullProbe 覆盖生效＋grep 平铺示范＋错误回落）。
#[test]
fn per_tool_render覆盖生效() {
    let probe = FullProbe;
    let outcome = ToolOutcome::ok(serde_json::json!({"matches": ["x"], "count": 1}));
    assert_eq!(probe.render(&outcome), "PROBE_RENDER");
    // grep 示范：matches 逐行平铺；canonical 值不变（模型面无损）
    let grep = GrepTool { cwd: Arc::new(CwdService::new(Path::new("."))) };
    let out = ToolOutcome::ok(serde_json::json!({"matches": ["a/1.rs:2:x", "b.rs:9:y"], "count": 2}));
    assert_eq!(grep.render(&out), "a/1.rs:2:x\nb.rs:9:y");
    // 错误路径与无 matches 回落统一渲染（同 render_outcome，行为零意外）
    let err = ToolOutcome::err("tool/regex", "正则非法");
    assert_eq!(grep.render(&err), render_outcome(&err));
    let no_matches = ToolOutcome::ok(serde_json::json!({"count": 0}));
    assert_eq!(grep.render(&no_matches), render_outcome(&no_matches));
    // 空匹配数组：输出「（无匹配）」提示而非空串（评审 P2-4：零匹配在会话/日志可见）
    let empty = ToolOutcome::ok(serde_json::json!({"matches": [], "count": 0}));
    assert_eq!(grep.render(&empty), "（无匹配）");
}

/// 场景五：收敛纪律——output_schema 的 S4 收敛声明在位（源码文本断言）＋五工具示范已填。
#[test]
fn output_schema收敛纪律() {
    // rustdoc 纪律的 CI 化：防「规格悄悄改」（方案 §2.5）——声明文本在位
    let source = include_str!("../src/contract.rs");
    assert!(source.contains("S4 收敛声明"), "output_schema 的 S4 收敛声明缺失");
    assert!(source.contains("新工具必须覆盖本方法"), "「S4 后新工具必填」声明缺失");
    // 我辖区五内置工具已填 output_schema 作示范（bash 归云铠辖区，不代不断言）
    let mine = ["read", "write", "edit", "glob", "grep"];
    let tools = builtin_tools(Arc::from(Path::new(".")), None, None);
    let mut filled = 0;
    for tool in &tools {
        if !mine.contains(&tool.name()) {
            continue; // bash：云铠辖区（默认 None 豁免中）
        }
        let Some(schema) = tool.output_schema() else {
            panic!("{} 未填 output_schema（S4 收敛纪律）", tool.name());
        };
        assert_eq!(schema["type"], "object", "{} 的 output_schema 应为 object", tool.name());
        assert!(schema.get("properties").is_some(), "{} 应有 properties", tool.name());
        filled += 1;
    }
    assert_eq!(filled, 5, "read/write/edit/glob/grep 五工具应全部示范");
}

/// 场景六：hint 字段 serde 加法兼容规格锁（契约 v1→v1.1；云镜 P2-A1）。
/// hint 字段的 serde 行为此前只有 contract.rs rustdoc 留痕，无测试锁——本测试
/// 把兼容两翼锁死：①旧数据（v1 存量日志/网关留存）缺 hint 字段必须可读，回落
/// `None`（serde default）；②新数据（v1.1）带 hint 序列化/反序列化往返无损。
/// 判别力边界（云镜 A1 复核 nit，如实注记）：收紧为 `deny_unknown_fields`
/// 在本场景**不红**——三场景数据均不含未知字段，收紧后仍全绿；该收紧的真实
/// 危害在 v1.2 增字段后旧数据被拒，当前用例锁不到（补未知字段用例属规格扩展
/// 另批评估）。去掉 `#[serde(default)]`（场景①红）或改 hint 形态（场景②红）
/// 在此红——「规格悄悄改」防线的 CI 化，与场景五 rustdoc 断言同纪律。
#[test]
fn hint字段serde加法兼容() {
    // ①旧数据缺 hint：可读，回落 None（v1 存量日志/网关数据零炸裂）
    let old: ToolError = serde_json::from_str(r#"{"code":"tool/x","message":"坏"}"#)
        .expect("缺 hint 字段的旧数据必须可读（serde default 规格锁）");
    assert_eq!(old.code, "tool/x");
    assert_eq!(old.message, "坏");
    assert!(old.hint.is_none(), "缺 hint 回落 None");
    // ②新数据带 hint：序列化 → 反序列化往返无损（指引文本原样保留）
    let fresh = ToolError::new("tool/path-outside-root", "越界").with_hint("改用 write 工具");
    let text = serde_json::to_string(&fresh).expect("带 hint 序列化");
    let back: ToolError = serde_json::from_str(&text).expect("带 hint 往返反序列化");
    assert_eq!(back.code, "tool/path-outside-root");
    assert_eq!(back.hint.as_deref(), Some("改用 write 工具"), "hint 往返必须无损");
    // 显式 null 同回落 None（网关传 hint=null 不炸；Option 语义的边界补锁）
    let null_hint: ToolError =
        serde_json::from_str(r#"{"code":"c","message":"m","hint":null}"#).expect("hint=null 可读");
    assert!(null_hint.hint.is_none());
}