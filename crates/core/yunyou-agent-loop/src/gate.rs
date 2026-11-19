//! 工具执行前审批缝（task-16 S1，设计 lead 批 2026-11-18：三待裁全批+S1 动工）。
//!
//! # 定位与触发时机
//! 缝=工具执行出口：`run_tool` 取得工具之后、`execute_tool` 之前，按序调用
//! 已注册 gate；**第一个非 Pass 定案**（短路）。内核只实现注册/调用序/呈现
//! 分派，**零规则知识**——deny/ask/allow 的判定、规则语法、模式矩阵全部在
//! 策略插件侧（plugins/workflows/permission，S2-S5），主板论同律。
//!
//! # 错误隔离（对齐 [`crate::hooks`] 同律）
//! gate Err / panic / 超时（默认 200ms）→ 该 gate 本轮按 Pass 跳过，工具照跑；
//! 同一 gate 连续失败 [`GATE_FAILURE_TRIP`] 次 → 自动摘除（进程内软态）。
//! **fail-open 是有意语义**（缝=增强不是承重墙）：高危兜底的真源在工具自判层
//! （bash_guard 独立于本缝在 tools 域生效），缝故障不解除工具层防线。
//!
//! # 审批呈现（Ask → 审批对象类型注册缝）
//! Ask 决策经 [`ApprovalUiRegistry`] 按对象类型分派呈现器（v1 两型：工具调用
//! 型/文件审阅型——后者服务 plan 模式 ExitPlan，yun-bo 模式体系稿 §三已裁）；
//! **无呈现器（无头场景）= 自动 Deny（fail-closed）**——问不了人就不放行。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use yunyou::Result;
use yunyou_util::BoxFut;

/// 单 gate 执行超时（对齐 hooks HOOK_TIMEOUT；旋钮候 [permission] 后续卡）。
pub(crate) const GATE_TIMEOUT: Duration = Duration::from_millis(200);
/// 同一 gate 连续失败摘除阈值（对齐 HOOK_FAILURE_TRIP=3 先例）。
pub(crate) const GATE_FAILURE_TRIP: u32 = 3;

/// 工具调用只读投影（gate 判定输入；不给可变引用=gate 不可能改写调用）。
pub struct ToolCallView<'a> {
    /// 工具名（模型面名）。
    pub name: &'a str,
    /// 参数 JSON 原文（未解析；解析与形态守卫在 run_tool 既有链）。
    pub arguments: &'a str,
}

/// gate 判定上下文（v1 最小集；workspace_root/mode 随 S4 扩字段——加法演进）。
#[derive(Debug, Clone, Default)]
pub struct GateContext {
    /// 当前权限模式（None=未设置/未接线；策略插件自解析；词汇对齐模式体系
    /// 骨架附录 SessionMode 四态 kebab——default/accept_edits/plan/bypass）。
    pub mode: Option<String>,
    /// 工作区根（S3/S4：acceptEdits 与 default 的「区内写免审」判定面；
    /// None=root 未知→策略侧按区外保守处理）。
    pub workspace_root: Option<String>,
    /// 沙箱意愿信号（S4 L4 穿越缝：执行面愿进沙箱=Some(true)；None=未知——
    /// 策略侧 auto_allow_sandboxed 开且意愿真时穿越 ask 规则。执行面接线候
    /// 沙箱联动卡，v1 信号位）。
    pub sandbox_willing: Option<bool>,
}

/// gate 决策（三态+Pass；第一个非 Pass 短路）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// 本 gate 无意见，链继续。
    Pass,
    /// 放行（sandboxed=true=策略侧裁定须进沙箱执行；v1 仅标记，执行面接线候沙箱联动卡）。
    Allow { sandboxed: bool },
    /// 问人：经审批呈现器取用户决定（建议清单=「记住本次选择」候选，§五设计）。
    Ask { suggestions: Vec<String> },
    /// 拒绝：message=人话（进 ToolResult 错误文本给模型指路）。
    Deny { message: String },
}

/// 工具执行前 gate（task-16 S1 缝）。
///
/// 注册面=内核 Context 服务表：策略插件 inject
/// [`GateRegistryService`](GateRegistryService) 在 setup 期注册；运行期注册
/// 先后无关（runner 每轮快照）。卸载纪律同 hooks（dispose 自行 unregister，
/// 熔断摘除兜底）。
pub trait PreToolUseGate: Send + Sync {
    /// gate 显示名（诊断/摘除点名用；同名注册 fail-loud）。
    fn name(&self) -> &str;

    /// 执行序（同序按注册先后稳定排序；小序先跑——免疫层 L0 应注册为小序）。
    fn order(&self) -> u32 {
        100
    }

    /// 审批判定：输入=只读调用投影；Err=隔离跳过（见模块头 fail-open 语义）。
    fn check(&self, call: &ToolCallView<'_>, ctx: &GateContext) -> BoxFut<'_, Result<GateDecision>>;
}

// ── 审批对象类型注册缝（v1 两型起步；注册制可扩展） ──────────────

/// 审批对象（present 时由内核组装；类型即呈现器分派键）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalSubjectKind {
    /// 工具调用型（判定链 Ask 的缺省型）。
    ToolCall,
    /// 文件审阅型（plan 模式 ExitPlan：审批对象=计划文件，内嵌 diff/预览）。
    FileReview,
    /// 确认型变体占位（bypass 进入确认；v1 不实现——模式状态机卡客户）。
    Confirmation,
    /// 插件权限型（task-16 修正令 2026-11-19：安装级授权——manifest
    /// [permissions] network/credentials 走 Ask 缝，用户授权制替代恒拒；
    /// 手机权限模型：审批粒度从调用级扩展到安装级）。
    PluginPermission,
}

/// 审批对象载荷（借用形态；present 期间有效）。
pub enum ApprovalSubject<'a> {
    /// 工具调用型：被拦的工具调用。
    ToolCall(&'a ToolCallView<'a>),
    /// 文件审阅型：待审文件路径+文本预览（v1 文本；diff 渲染归呈现器）。
    FileReview { path: &'a str, preview: &'a str },
    /// 插件权限型：安装/升级时插件申请的能力面（requested 项如
    /// "network"/"credentials:openai_key"；purpose=manifest 声明的用途描述）。
    PluginPermission {
        /// 插件 id。
        plugin_id: &'a str,
        /// 申请项清单（逐项可授；空串元素=申请面畸形，呈现器拒）。
        requested: &'a [String],
        /// 用途描述（manifest 声明；呈现给用户）。
        purpose: &'a str,
    },
}

/// 呈现器产出（「记住本次选择」的策略落库在呈现器/策略侧，内核只取决定）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// 用户放行。
    Approved,
    /// 用户拒绝（message=人话，进 ToolResult）。
    Denied { message: String },
}

/// 审批呈现器（按对象类型注册；同型重复注册 fail-loud）。
pub trait ApprovalPresenter: Send + Sync {
    /// 本呈现器管的类型。
    fn subject(&self) -> ApprovalSubjectKind;

    /// 呈现并取用户决定。Err=呈现故障（按 Denied fail-closed 处理+人话指路）。
    fn present(&self, subject: ApprovalSubject<'_>) -> BoxFut<'_, Result<ApprovalDecision>>;
}

// ── 注册表（gate 链 + 呈现器表；隔离/熔断机制同 hooks.rs） ─────────

struct GateEntry {
    gate: Arc<dyn PreToolUseGate>,
}

/// gate 注册表+判定执行器（错误隔离三态+连败摘除，机制与 [`crate::hooks`] 同源）。
#[derive(Default)]
pub struct GateRegistry {
    entries: RwLock<Vec<GateEntry>>,
    /// 连败计数（按 gate 名；成功清零，达阈值摘除）。
    failures: Mutex<HashMap<String, u32>>,
}

impl GateRegistry {
    /// 注册 gate；同名 fail-loud。
    pub fn register(&self, gate: Arc<dyn PreToolUseGate>) -> Result<()> {
        let mut entries = self.entries.write().expect("gate 表锁");
        if entries.iter().any(|e| e.gate.name() == gate.name()) {
            return Err(yunyou::CordisError::new(
                yunyou::ErrorKind::Internal,
                format!("审批 gate「{}」重复注册（同名 fail-loud）", gate.name()),
            ));
        }
        entries.push(GateEntry { gate });
        Ok(())
    }

    /// 注销（按名；返回是否确有注销）。
    pub fn unregister(&self, name: &str) -> bool {
        let mut entries = self.entries.write().expect("gate 表锁");
        let before = entries.len();
        entries.retain(|e| e.gate.name() != name);
        entries.len() != before
    }

    /// 在册 gate 名清单（观测面）。
    pub fn list(&self) -> Vec<String> {
        self.entries.read().expect("gate 表锁").iter().map(|e| e.gate.name().to_string()).collect()
    }

    /// 是否空（run_tool 零开销短路判定）。
    pub fn is_empty(&self) -> bool {
        self.entries.read().expect("gate 表锁").is_empty()
    }

    /// 按序判定：第一个非 Pass 短路；全 Pass=Pass。
    /// 隔离三态：Err/panic/超时→该 gate 按 Pass 跳过+连败计数（达阈值摘除）。
    pub async fn check(&self, call: &ToolCallView<'_>, ctx: &GateContext) -> GateDecision {
        let snapshot: Vec<Arc<dyn PreToolUseGate>> = {
            let entries = self.entries.read().expect("gate 表锁");
            let mut gates: Vec<_> = entries.iter().map(|e| Arc::clone(&e.gate)).collect();
            gates.sort_by_key(|g| g.order());
            gates
        };
        for gate in snapshot {
            let name = gate.name().to_string();
            // panic 边界复用 hooks.rs 同款 catch_panic（单源；None=panic 态走隔离）
            let fut = crate::hooks::catch_panic(gate.check(call, ctx));
            match tokio::time::timeout(GATE_TIMEOUT, fut).await {
                Ok(Some(Ok(decision))) => {
                    self.failures.lock().expect("连败表锁").remove(&name);
                    if !matches!(decision, GateDecision::Pass) {
                        return decision;
                    }
                }
                // panic(None)/Err/超时 → 隔离跳过+连败计数
                _ => self.bump_failure(&name),
            }
        }
        GateDecision::Pass
    }

    /// 连败计数+阈值摘除（对齐 hooks 同名机制）。
    fn bump_failure(&self, name: &str) {
        let mut failures = self.failures.lock().expect("连败表锁");
        let count = failures.entry(name.to_string()).or_insert(0);
        *count += 1;
        if *count >= GATE_FAILURE_TRIP {
            drop(failures);
            self.unregister(name);
        }
    }
}

/// 审批呈现器注册表（按对象类型；同型 fail-loud）。
#[derive(Default)]
pub struct ApprovalUiRegistry {
    presenters: RwLock<HashMap<ApprovalSubjectKind, Arc<dyn ApprovalPresenter>>>,
}

impl ApprovalUiRegistry {
    /// 注册呈现器；同型 fail-loud。
    pub fn register(&self, presenter: Arc<dyn ApprovalPresenter>) -> Result<()> {
        let mut presenters = self.presenters.write().expect("呈现器表锁");
        let kind = presenter.subject();
        if presenters.contains_key(&kind) {
            return Err(yunyou::CordisError::new(
                yunyou::ErrorKind::Internal,
                format!("审批呈现器 {kind:?} 重复注册（同型 fail-loud）"),
            ));
        }
        presenters.insert(kind, presenter);
        Ok(())
    }

    /// 注销（按类型；返回是否确有注销）。
    pub fn unregister(&self, kind: ApprovalSubjectKind) -> bool {
        self.presenters.write().expect("呈现器表锁").remove(&kind).is_some()
    }

    /// 在册类型清单（观测面）。
    pub fn list(&self) -> Vec<ApprovalSubjectKind> {
        self.presenters.read().expect("呈现器表锁").keys().copied().collect()
    }

    /// 按类型取呈现器（Arc 克隆，锁即放——同步上下文探测/自管调用用；
    /// [`Self::present`] 为异步入口，本方法供 setup 等同步路径使用）。
    pub fn presenter_for(&self, kind: ApprovalSubjectKind) -> Option<Arc<dyn ApprovalPresenter>> {
        self.presenters.read().expect("呈现器表锁").get(&kind).map(Arc::clone)
    }

    /// 呈现取决定：无呈现器=None（调用方 fail-closed Deny）。
    pub async fn present(&self, subject: ApprovalSubject<'_>) -> Option<ApprovalDecision> {
        let kind = match &subject {
            ApprovalSubject::ToolCall(_) => ApprovalSubjectKind::ToolCall,
            ApprovalSubject::FileReview { .. } => ApprovalSubjectKind::FileReview,
            ApprovalSubject::PluginPermission { .. } => ApprovalSubjectKind::PluginPermission,
        };
        let presenter = Arc::clone(self.presenters.read().expect("呈现器表锁").get(&kind)?);
        match tokio::time::timeout(GATE_TIMEOUT, presenter.present(subject)).await {
            Ok(Ok(decision)) => Some(decision),
            // Err/panic/超时→呈现故障=问不了人=fail-closed 拒绝
            _ => Some(ApprovalDecision::Denied {
                message: "审批呈现故障（呈现器 Err/超时），按拒绝处理（fail-closed）".into(),
            }),
        }
    }
}

/// gate 链+呈现器表捆绑（runner 单字段持两表；Default=全空=行为与无缝逐字节一致）。
#[derive(Default)]
pub struct GateHub {
    /// 判定链。
    pub gates: GateRegistry,
    /// 审批呈现器表。
    pub approval: ApprovalUiRegistry,
    /// 权限模式会话态（task-16 快刀卡：/mode 运行期切换写此处，run_tool 组装
    /// GateContext 时透传——S3 引擎现成消费；**会话内存态不落盘**，会话结束
    /// 随进程态收回=授权/模式不残留哲学；session-meta 持久化=演进项候裁）。
    pub mode: RwLock<Option<String>>,
}

impl GateHub {
    /// 读当前模式（None=未设=策略侧按 Default 保守处理）。
    pub fn current_mode(&self) -> Option<String> {
        self.mode.read().expect("模式态锁").clone()
    }

    /// 写模式（/mode 命令切换入口；任意字符串由策略侧 Mode::parse 校验——
    /// 未知词在策略侧保守降 Default，此处不拒收=服务面零词汇知识）。
    pub fn set_mode(&self, mode: Option<String>) {
        *self.mode.write().expect("模式态锁") = mode;
    }
}

/// 模式会话态服务面（cli /mode 命令经此切换；对齐 GateRegistryService 形态）。
pub trait ModeStateService: Send + Sync {
    /// 读当前模式（None=未设）。
    fn current_mode(&self) -> Option<String>;
    /// 写模式（None=清回未设）。
    fn set_mode(&self, mode: Option<String>);
}

impl ModeStateService for GateHub {
    fn current_mode(&self) -> Option<String> {
        GateHub::current_mode(self)
    }
    fn set_mode(&self, mode: Option<String>) {
        GateHub::set_mode(self, mode)
    }
}

/// gate 注册服务面（策略插件 inject 注册；对齐 HooksRegistryService 形态）。
pub trait GateRegistryService: Send + Sync {
    /// 注册 gate（同名 fail-loud）。
    fn register(&self, gate: Arc<dyn PreToolUseGate>) -> Result<()>;
    /// 注销 gate（按名）。
    fn unregister(&self, name: &str) -> bool;
    /// 在册清单（观测面）。
    fn list(&self) -> Vec<String>;
}

impl GateRegistryService for GateHub {
    fn register(&self, gate: Arc<dyn PreToolUseGate>) -> Result<()> {
        self.gates.register(gate)
    }
    fn unregister(&self, name: &str) -> bool {
        self.gates.unregister(name)
    }
    fn list(&self) -> Vec<String> {
        self.gates.list()
    }
}

/// 审批呈现器注册服务面（UI 侧/模式插件 inject 注册）。
pub trait ApprovalUiService: Send + Sync {
    /// 注册呈现器（同型 fail-loud）。
    fn register(&self, presenter: Arc<dyn ApprovalPresenter>) -> Result<()>;
    /// 注销（按类型）。
    fn unregister(&self, kind: ApprovalSubjectKind) -> bool;
    /// 在册类型清单。
    fn list(&self) -> Vec<ApprovalSubjectKind>;
}

impl ApprovalUiService for GateHub {
    fn register(&self, presenter: Arc<dyn ApprovalPresenter>) -> Result<()> {
        self.approval.register(presenter)
    }
    fn unregister(&self, kind: ApprovalSubjectKind) -> bool {
        self.approval.unregister(kind)
    }
    fn list(&self) -> Vec<ApprovalSubjectKind> {
        self.approval.list()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 透传 gate（按序/短路测试用）。
    struct StaticGate {
        name: &'static str,
        order: u32,
        decision: GateDecision,
    }
    impl PreToolUseGate for StaticGate {
        fn name(&self) -> &str {
            self.name
        }
        fn order(&self) -> u32 {
            self.order
        }
        fn check(&self, _call: &ToolCallView<'_>, _ctx: &GateContext) -> BoxFut<'_, Result<GateDecision>> {
            Box::pin(async move { Ok(self.decision.clone()) })
        }
    }

    fn view() -> ToolCallView<'static> {
        ToolCallView { name: "bash", arguments: "{\"command\":\"ls\"}" }
    }

    /// 同名注册 fail-loud + 注销 + 清单。
    #[test]
    fn 注册面_同名拒绝与注销() {
        let hub = GateHub::default();
        hub.gates
            .register(Arc::new(StaticGate { name: "g1", order: 1, decision: GateDecision::Pass }))
            .unwrap();
        assert!(hub.gates.register(Arc::new(StaticGate { name: "g1", order: 2, decision: GateDecision::Pass })).is_err());
        assert_eq!(hub.gates.list(), vec!["g1"]);
        assert!(hub.gates.unregister("g1"));
        assert!(!hub.gates.unregister("g1"));
        // 呈现器同型 fail-loud
        struct P;
        impl ApprovalPresenter for P {
            fn subject(&self) -> ApprovalSubjectKind {
                ApprovalSubjectKind::ToolCall
            }
            fn present(&self, _s: ApprovalSubject<'_>) -> BoxFut<'_, Result<ApprovalDecision>> {
                Box::pin(async move { Ok(ApprovalDecision::Approved) })
            }
        }
        hub.approval.register(Arc::new(P)).unwrap();
        assert!(hub.approval.register(Arc::new(P)).is_err());
    }

    /// 模式会话态：set/get 透传（/mode 命令写 GateHub.mode，run_tool 读同源）。
    #[test]
    fn 模式会话态写读() {
        let hub = GateHub::default();
        assert_eq!(hub.current_mode(), None, "未设=None");
        hub.set_mode(Some("plan".into()));
        assert_eq!(hub.current_mode().as_deref(), Some("plan"));
        hub.set_mode(None);
        assert_eq!(hub.current_mode(), None, "清回未设");
    }

    /// 链序：小序先跑+第一个非 Pass 短路（后位 Deny 不得覆盖先位 Allow）。
    #[tokio::test]
    async fn 链序与短路() {
        let hub = GateHub::default();
        hub.gates
            .register(Arc::new(StaticGate { name: "慢", order: 200, decision: GateDecision::Deny { message: "不应到达".into() } }))
            .unwrap();
        hub.gates
            .register(Arc::new(StaticGate { name: "快", order: 10, decision: GateDecision::Allow { sandboxed: false } }))
            .unwrap();
        let decision = hub.gates.check(&view(), &GateContext::default()).await;
        assert_eq!(decision, GateDecision::Allow { sandboxed: false });
    }

    /// 错误隔离：Err gate 按 Pass 跳过，后续 gate 照常定案；连败 3 次摘除。
    #[tokio::test]
    async fn 错误隔离与熔断摘除() {
        struct ErrGate;
        impl PreToolUseGate for ErrGate {
            fn name(&self) -> &str {
                "坏"
            }
            fn check(&self, _c: &ToolCallView<'_>, _x: &GateContext) -> BoxFut<'_, Result<GateDecision>> {
                Box::pin(async move { Err(yunyou::CordisError::new(yunyou::ErrorKind::Internal, "炸")) })
            }
        }
        let hub = GateHub::default();
        hub.gates.register(Arc::new(ErrGate)).unwrap();
        hub.gates
            .register(Arc::new(StaticGate { name: "兜", order: 200, decision: GateDecision::Deny { message: "拦截".into() } }))
            .unwrap();
        for _ in 0..3 {
            let d = hub.gates.check(&view(), &GateContext::default()).await;
            assert_eq!(d, GateDecision::Deny { message: "拦截".into() }, "Err 按 Pass 跳过");
        }
        assert!(!hub.gates.list().contains(&"坏".to_string()), "连败 3 次应摘除");
    }

    /// Ask 呈现分派：有呈现器=取决定；无呈现器=None（调用方 fail-closed）；
    /// 呈现器 Err=fail-closed 拒绝。
    #[tokio::test]
    async fn ask呈现分派与fail_closed() {
        let hub = GateHub::default();
        // 无呈现器：None
        assert!(hub.approval.present(ApprovalSubject::ToolCall(&view())).await.is_none());
        // 挂呈现器：Approved 透传；Deny 透传
        struct Yes;
        impl ApprovalPresenter for Yes {
            fn subject(&self) -> ApprovalSubjectKind {
                ApprovalSubjectKind::ToolCall
            }
            fn present(&self, _s: ApprovalSubject<'_>) -> BoxFut<'_, Result<ApprovalDecision>> {
                Box::pin(async move { Ok(ApprovalDecision::Approved) })
            }
        }
        hub.approval.register(Arc::new(Yes)).unwrap();
        assert_eq!(
            hub.approval.present(ApprovalSubject::ToolCall(&view())).await,
            Some(ApprovalDecision::Approved)
        );
        // FileReview 型无呈现器=None（plan 客户未挂时不误用工具型呈现器）
        assert!(hub
            .approval
            .present(ApprovalSubject::FileReview { path: "plan.md", preview: "# 计划" })
            .await
            .is_none());
    }
}
