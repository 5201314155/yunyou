//! 工具契约：定义 trait、执行上下文与结果形态
//! （对标 contracts.ts:30-46 的 execute + output，一期裁剪见模块注释）。

use serde::{Deserialize, Serialize};
use yunyou_util::BoxFut;
use tokio_util::sync::CancellationToken;

/// 一次工具执行的运行上下文。
///
/// 对标 TS 的 `ToolRunContext`（call_id + 取消信号）；一期不含权限审批
/// 与上下文推迟（deferred context），出现需求再扩。
#[derive(Clone, Debug)]
pub struct ToolExec {
    /// 本次调用 id（会话 tool_call / tool_result 事件的关联键）。
    pub call_id: String,
    /// 取消令牌（通常来自所属 fiber；工具长任务必须观察它，方案 §2.3）。
    pub cancel: CancellationToken,
    /// 工作区上下文槽位（契约⑨，方案 §3-⑨ 一期既定动作；P1-11a 免返工预留）：
    /// 一期只落位不读不校验（策略对象落地前恒 `None`），工具侧零感知——
    /// 不读该字段即无行为变化；P1-11a 落地时由执行器补传策略面。
    pub workspace: Option<WorkspaceContext>,
}

/// 工作区上下文（契约⑨ `sandboxPolicy` 策略对象，P1-11a 策略面落地充实）。
///
/// `policy` 为 Arc 快照（构造一次贯穿 L2 授权与 L3 enforcer，防两处解析
/// 漂移——设计 §3.1）；`workspace: None` 语义不变=沙箱未接（降级直跑
/// +meta 标注），既有 None 构造点零破坏。
#[derive(Clone, Debug)]
pub struct WorkspaceContext {
    /// 策略快照（构造在 yunyou-sandbox 插件 setup，消费在 bash helper 链）。
    pub policy: std::sync::Arc<crate::sandbox::WorkspacePolicy>,
}

/// 工具业务失败（进入会话的 tool_result，is_error=true）。
///
/// # hint 指路面（P0 三缺陷之三；契约加法演进 v1 → v1.1）
/// `hint` 是给模型的下一步行动指引：中文、可执行、不含内部路径。加法向后
/// 兼容两翼：serde default 使旧日志/网关数据缺 hint 仍可读；`None`＝无指引，
/// 既有调用点零改动。渲染统一走 [`render_error`]。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    /// 稳定错误码（如 `tool/path-outside-root`、`tool/timeout`）。
    pub code: String,
    /// 中文可读信息。
    pub message: String,
    /// 下一步指引（可空；存在时随错误文本进入会话，模型据此择路不再瞎猜）。
    #[serde(default)]
    pub hint: Option<String>,
}

impl ToolError {
    /// 构造工具错误（无指引；既有调用点语义不变）。
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), hint: None }
    }

    /// 挂指引（builder：只增字段不改既有语义）。
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// 工具执行结果：成功携带**无损 JSON 值**（canonical value，对标
/// contracts.ts:38「return only its canonical lossless-JSON value」）。
#[derive(Debug, Clone)]
pub enum ToolOutcome {
    /// 成功：canonical JSON 值（渲染统一由执行器完成，见 lib.rs 差异说明）。
    Ok(serde_json::Value),
    /// 业务失败。
    Err(ToolError),
}

impl ToolOutcome {
    /// 快捷构造成功值。
    pub fn ok(value: serde_json::Value) -> Self {
        Self::Ok(value)
    }

    /// 快捷构造错误。
    pub fn err(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Err(ToolError::new(code, message))
    }

    /// 快捷构造带指引的错误（P0-3 指路面：错误不只说「错在哪」，还说「下一步怎么走」）。
    pub fn err_h(
        code: impl Into<String>,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self::Err(ToolError::new(code, message).with_hint(hint))
    }
}

/// 工具卡片的呈现元数据（契约⑥「meta 投影成卡片」的最小集）。
///
/// 静态纯声明：标题 / 摘要模板 / 图标位——**不含运行期值**，渲染层在执行时
/// 喂 args/outcome 做插值；因此 meta 本体可安全进 manifest `ui` 缝、诊断面
/// 与两端渲染（CLI/未来 App，方案 §4.1 输出渲染钩形态）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationMeta {
    /// 卡片标题（如「读取文件」）。
    pub title: String,
    /// 摘要模板（如「读取 path 的内容」；插值由渲染层完成，模板保持静态）。
    pub summary: String,
    /// 图标位（渲染层映射的图标键/字符占位，如「file」）。
    pub icon: String,
}

impl PresentationMeta {
    /// 构造呈现元数据（三字段均为渲染层输入，不带运行期值）。
    pub fn new(title: impl Into<String>, summary: impl Into<String>, icon: impl Into<String>) -> Self {
        Self { title: title.into(), summary: summary.into(), icon: icon.into() }
    }
}

/// 工具定义 trait（对标 TS `ToolDefinition` 的服务面裁剪）。
///
/// # 契约（评审线一强制）
/// - object-safe：无泛型方法、无 `Self` 返回值；异步返回 [`BoxFut`]；
/// - `execute` 收到的 `args` 是模型给出的 JSON 参数（工具自行校验/收窄，
///   对标 contracts.ts「the tool validates/narrows its own input」）；
/// - 输出契约：[`ToolDefinition::output_schema`] 可选声明 canonical value 的
///   JSON Schema（一期执行器不强制校验，保留门禁扩展位——差异已报备）；
/// - 实现必须 `Send + Sync` 且 `execute` 返回 `'static` future。
pub trait ToolDefinition: Send + Sync {
    /// 工具名（模型可见；建议 snake_case）。
    fn name(&self) -> &str;

    /// 工具描述（模型可见，注入系统提示词）。
    fn description(&self) -> &str;

    /// 参数 JSON Schema（draft 2020-12；object 类型）。
    fn parameters(&self) -> serde_json::Value;

    /// canonical 输出声明。
    ///
    /// **S4 收敛声明（契约⑥，规格冻结 v1 组成部分）**：默认 `None` 仅作存量
    /// 豁免——bash 等一期工具暂缺声明；**S4 起新工具必须覆盖本方法**（破坏
    /// 规格冻结纪律＝评审打回项，断言测试锁死：tests/工具契约规格冻结.rs）。
    /// 默认实现保留而非删除的理由：构造入口/注册期直接拒绝 `None` 会当场打死
    /// 存量 bash 工具（其升级归 bash 语义独立卡）， Rust 亦无「强制覆盖」
    /// 编译机制；收敛路径＝本 deprecation 声明＋内置五工具示范（read/write/
    /// edit/glob/grep 已填）＋测试纪律三事，**破坏性收紧随契约 v2 连同 bash
    /// 一并做**（届时 register 可拒绝 `None`）。
    fn output_schema(&self) -> Option<serde_json::Value> {
        None
    }

    /// 单次调用超时（毫秒；`None` = 用全局 `tool_timeout_secs`）。
    ///
    /// **白名单纪律（契约 v1·评审线一强制）**：本方法绝不进模型面——
    /// `RegistryService::schemas()` 只投 name/description/parameters 三元组
    /// （断言测试锁死，tests/ 工具契约规格冻结.rs）；对标官方
    /// `contracts.ts:63-70` 的 timeoutMs 白名单。
    fn timeout_ms(&self) -> Option<u64> {
        None
    }

    /// 是否可并发执行（默认 `false` = 保守串行；读类工具显式选入 `true`）。
    ///
    /// 对标官方 `contracts.ts:71-84` 的 isConcurrencySafe 显式选入语义：
    /// 工具作者声明前调用方不得假定其并发安全。**同 timeout_ms：绝不进模型面。**
    fn is_concurrency_safe(&self) -> bool {
        false
    }

    /// 把一次执行结果投影为人/卡片可见文本（契约⑥渲染钩）。
    ///
    /// 默认实现＝现有统一渲染 [`render_outcome`]（零行为变更）；per-tool 可覆盖
    /// 定制卡片/文本（对标官方 `contracts.ts:85-102` presentResult 纯投影）。
    /// 注意：模型面的 canonical 值不受本钩影响——无 lossless 值直通见
    /// [`ToolOutcome::Ok`]。
    fn render(&self, outcome: &ToolOutcome) -> String {
        render_outcome(outcome)
    }

    /// 工具卡片的呈现元数据声明（契约⑥；默认 `None`＝无声明，渲染层回落统一卡片）。
    ///
    /// 静态纯声明（标题/摘要模板/图标位），渲染前即可知——**不做成
    /// `ToolOutcome` 的伴随字段**：挂 Definition 与 `timeout_ms`/
    /// `is_concurrency_safe` 同构（默认 None＝零声明，存量工具零改动）；
    /// 塞进结果值会把展示负载污染 canonical 数据通路与会话日志，违反
    /// 「事件轻、重载荷不进模型请求」与「会话日志禁 append 新 type」。
    fn presentation_meta(&self) -> Option<PresentationMeta> {
        None
    }

    /// 执行一次调用。取消语义见 [`ToolExec::cancel`]。
    fn execute(&self, args: serde_json::Value, exec: ToolExec) -> BoxFut<'static, ToolOutcome>;
}

/// 渲染错误为会话可见文本（hint 存在时追加「提示：」行——模型据此择路）。
pub fn render_error(err: &ToolError) -> String {
    match &err.hint {
        Some(hint) => format!("[错误 {}] {}\n提示：{}", err.code, err.message, hint),
        None => format!("[错误 {}] {}", err.code, err.message),
    }
}

/// 渲染 canonical value 为模型/用户可见文本（统一渲染；`ToolDefinition::render`
/// 的默认实现直调本函数——per-tool 定制覆盖点见该 trait 方法）。
pub fn render_outcome(outcome: &ToolOutcome) -> String {
    match outcome {
        ToolOutcome::Ok(value) => serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into()),
        ToolOutcome::Err(err) => render_error(err),
    }
}
