//! 插件声明面（PluginManifest，插件化方案 §2.2）。
//!
//! manifest 一期是「代码内声明」形态（裁决 D1：代码是真源）：[`crate::registry::Plugin`]
//! 的 `manifest()` 返回本结构，装载前由 [`validate_manifest`] 做静态校验（S2 接入装配
//! 序列，本批仅提供入口与测试）；schemars 生成落盘的文件形态留给 market 级二期。
//!
//! ## 服务键字符串投影规则（本批建立，方案 §2.2「与 ServiceId 投影规则一致」）
//!
//! 服务键在内核里是 [`ServiceId`]（trait object 的 `TypeId` 类型键），manifest 侧是
//! 字符串；`TypeId` 运行期不携带名字，故投影在「类型在手上」的调用点完成：
//! [`projection_of::<P>()`] 把 `type_name::<P>()` 规范化为 `"dyn <TraitName>"`（取尾段）
//! 并登记进进程级投影表。组件 manifest 的 `inject`/`provides` 字符串**一律经
//! [`projection_of`] 生成**（与 `ServiceId::of::<P>()` 同一类型参数），校验器对 trait
//! `inject()` 的键查表反查后做集合比对——trait 侧加键而 manifest 未同步（或反向）均被抓。

use crate::registry::Plugin;
use crate::service::ServiceId;
use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

/// 插件两级（方案 §2.2 两级表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginLevel {
    /// 编译期注册、随二进制分发；禁用仅影响启用态，不可卸载。
    Bundled,
    /// 二期子进程插件协议（JSON-RPC over stdio）分发的独立包，可卸载。
    Market,
}

/// UI 注册缝条目（方案 §4）：条目 schema v1 已随 S8 冻结（settings/command 合法、
/// page 预留），实现与校验见 [`crate::manifest_ui`]（本 re-export 保持 `yunyou::manifest::UiEntry`
/// 与 `yunyou::UiEntry` 原路径可用）。
pub use crate::manifest_ui::UiEntry;

/// 插件声明面（一期最小集，方案 §2.2）。
///
/// # 契约（评审线一）
/// - `id` 全局唯一：bundled 用 crate 名，market 用包名；
/// - `inject` 与 [`Plugin::inject`] 严格同源（同一类型参数清单，经 [`projection_of`] 投影）；
/// - `version` 用组件自身 `env!("CARGO_PKG_VERSION")`（semver）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginManifest {
    /// 插件全局唯一标识：bundled 用 crate 名（如 "yunyou-tools"），market 用包名。
    pub id: String,
    /// 插件自身版本（semver；组件覆盖时取自身 crate 的 CARGO_PKG_VERSION）。
    pub version: String,
    /// 插件两级：bundled 编译期注册、market 二期子进程协议（方案 §2.2 两级表）。
    pub level: PluginLevel,
    /// 对齐的插槽规格版本（方案 §2.5 契约版本化）：含本插件 provide 与 consume 的
    /// 契约，格式 `name.vN`（如 "tools.v1"）；装载前校验非空且格式合法。
    pub contracts: Vec<String>,
    /// provide 的服务键字符串投影（面板/诊断展示用；经 [`projection_of`] 生成）。
    pub provides: Vec<String>,
    /// 与 [`Plugin::inject`] 同源的冗余依赖声明（服务键投影），供装载前静态校验一致性。
    pub inject: Vec<String>,
    /// UI 注册缝贡献（方案 §4）：一期空数组预留位，条目仅类型位（实现随 S8）。
    pub ui: Vec<UiEntry>,
    /// 是否默认启用（bundled 默认 true；yunyou.toml [plugins] 段可覆盖，方案 §2.4）。
    pub auto_enable: bool,
}

impl PluginManifest {
    /// 最小 manifest（`Plugin::manifest` 默认实现用）：level=bundled、auto_enable=true、其余空。
    pub(crate) fn minimal(id: String, version: String) -> Self {
        Self {
            id,
            version,
            level: PluginLevel::Bundled,
            contracts: Vec::new(),
            provides: Vec::new(),
            inject: Vec::new(),
            ui: Vec::new(),
            auto_enable: true,
        }
    }
}

/// 进程级服务键投影表：`TypeId → 投影名`（由 [`projection_of`] 调用点登记）。
/// 全局静态表属内核元数据（非业务可变状态）：登记幂等、只增不改；锁临界区仅一次
/// HashMap 写、无 await，与 root fiber 表同类的进程级登记面。
fn projection_table() -> &'static RwLock<HashMap<TypeId, String>> {
    static TABLE: OnceLock<RwLock<HashMap<TypeId, String>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 服务键字符串投影的唯一合法入口：从类型投影 `"dyn <TraitName>"` 并登记进投影表。
/// 组件 manifest 的 `inject`/`provides` 必须经本函数生成（与 `ServiceId::of::<P>()`
/// 同一类型参数）；装载前校验凭投影表把 trait `inject()` 的 [`ServiceId`] 反查为
/// 字符串后比对。
pub fn projection_of<P: ?Sized + 'static>() -> String {
    let name = normalize_type_name(std::any::type_name::<P>());
    #[allow(clippy::expect_used)] // 中毒 tripwire：边界后触发=内核缺陷（S-ISO-1 A 锁类，std 锁保留裁决）
    let mut table = projection_table().write().expect("投影表锁");
    table.entry(TypeId::of::<P>()).or_insert(name.clone());
    name
}

/// 投影规范化：`"dyn yunyou_llm::Llm"` → `"dyn Llm"`（取尾段；非 trait object 类型退化为尾段）。
fn normalize_type_name(raw: &str) -> String {
    let body = raw.strip_prefix("dyn ").unwrap_or(raw);
    let short = body.rsplit("::").next().unwrap_or(body);
    format!("dyn {short}")
}

/// 查投影表：[`ServiceId`] → 投影名。Typed 键要求登记过（即 manifest 侧声明过）；
/// Named 弱类型字符串键自带名字、直接返回原名。
pub(crate) fn projection_of_id(id: &ServiceId) -> Option<String> {
    #[allow(clippy::expect_used)] // 中毒 tripwire（S-ISO-1 A 锁类，std 锁保留裁决）：边界后触发=内核缺陷
    match id {
        ServiceId::Named(name) => Some(name.to_string()),
        ServiceId::Typed(tid) => projection_table().read().expect("投影表锁").get(tid).cloned(),
    }
}

/// 装载前静态校验（方案 §2.2：宿主启动时先读全部 manifest 做静态校验，再进入装配）。
///
/// 校验三条（contracts 与宿主提供面的版本兼容比对随 S2 装配接入，本批不涉及）：
/// ① `inject` 冗余声明与 trait `inject()` 双向一致（服务键投影集合相等）；
/// ② `contracts` 非空且格式合法（`name.vN`）；③ `inject`/`provides` 服务键拼写
/// 与投影规则一致（`"dyn <TraitName>"`）。返回中文违例清单（空 = 通过）；
/// S2 起装配序列在装载前调用，违例即拒装并逐条报出。
pub fn validate_manifest(plugin: &dyn Plugin, manifest: &PluginManifest) -> Vec<String> {
    let mut issues = Vec::new();
    // ① inject 双向一致：trait 侧键查投影表 → 与 manifest 侧字符串集合比对
    //   （需 plugin 实例；落盘 manifest 文件侧无实例，不在此函数复用面内）
    let declared: HashSet<String> = manifest.inject.iter().cloned().collect();
    let mut actual: HashSet<String> = HashSet::new();
    for key in plugin.inject() {
        match projection_of_id(&key) {
            Some(name) => { actual.insert(name); }
            None => issues.push(format!(
                "inject 服务键 {} 无投影登记：manifest 侧必须经 projection_of 声明",
                key.display()
            )),
        }
    }
    for name in &actual {
        if !declared.contains(name) {
            issues.push(format!("trait inject() 声明了 {name} 但 manifest.inject 缺失（双向一致违例）"));
        }
    }
    for name in &declared {
        if !actual.contains(name) {
            issues.push(format!("manifest.inject 声明了 {name} 但 trait inject() 未声明（双向一致违例）"));
        }
    }
    // ②③④ 纯静态子集（无需 plugin 实例）：落盘 manifest 文件（装载器 L1，
    // yunyou-config）与代码内 manifest() 同口径复用——单源防两份校验漂移
    issues.extend(validate_manifest_static(manifest));
    issues
}

/// 静态子集校验（②③④，装载器 L1 抽取）：contracts 非空且格式合法（name.vN）、
/// 服务键拼写符合投影规则（`dyn <TraitName>`）、ui 条目（S8 冻结面）。不需要
/// [`Plugin`] 实例——代码内 manifest 校验（[`validate_manifest`] 的②③④段）与
/// 落盘 manifest 文件解析（yunyou-config `manifest_file`）共用本单源；①inject
/// 双向比对需实例，留 [`validate_manifest`]。
pub fn validate_manifest_static(manifest: &PluginManifest) -> Vec<String> {
    let mut issues = Vec::new();
    // ② contracts 非空且格式合法（name.vN）
    if manifest.contracts.is_empty() {
        issues.push("contracts 不得为空：需声明对齐的插槽规格版本（如 tools.v1，方案 §2.5）".into());
    }
    for spec in &manifest.contracts {
        if !is_contract_format(spec) {
            issues.push(format!("contracts 项 {spec:?} 格式非法：应为 name.vN（v+纯数字）"));
        }
    }
    // ③ 服务键拼写与投影规则一致（dyn <TraitName>）
    for key in manifest.inject.iter().chain(&manifest.provides) {
        if !is_service_key_format(key) {
            issues.push(format!("服务键 {key:?} 不符投影规则：应为 \"dyn <TraitName>\"（projection_of 投影形态）"));
        }
    }
    // ④ ui 条目校验（S8 冻结面，方案 §4）：page 预留拒装、section/name 非空
    issues.extend(crate::manifest_ui::validate_ui_entries(&manifest.ui));
    issues
}

/// contracts 格式：`name.vN`——name 非空（可含 `.`/`-` 做层级与连字），N 为正整数。
fn is_contract_format(spec: &str) -> bool {
    let Some((name, ver)) = spec.rsplit_once('.') else { return false };
    if name.is_empty() || !ver.starts_with('v') || ver.len() < 2 { return false }
    match name.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && ver[1..].chars().all(|c| c.is_ascii_digit())
}

/// 服务键拼写格式：`"dyn <TraitName>"`——投影规则的字符串形态（尾段标识符，不含路径）。
fn is_service_key_format(key: &str) -> bool {
    let Some(body) = key.strip_prefix("dyn ") else { return false };
    let mut chars = body.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigValue, Context, FiberOutput, Result};
    use std::sync::Arc;
    use yunyou_util::BoxFut;

    /// 测试用服务契约（验证投影取尾段）。
    trait TestSvc: Send + Sync {}
    /// 测试用深路径契约（验证规范化取尾段，显式登记）。
    trait DeepModuleInnerSvc: Send + Sync {}
    /// 测试用漂移契约：只在 trait inject() 出现、绝不经 projection_of 登记
    /// （模拟「trait 加了依赖、manifest 没同步」的漂移）。
    trait DriftSvc: Send + Sync {}

    /// 测试插件构造宏：类型清单只写一次，trait inject 与 manifest 同源。
    macro_rules! test_plugin {
        ($name:ident, $id:expr $(, $key:expr)* $(,)?) => {
            struct $name;
            impl Plugin for $name {
                fn name(&self) -> &str { $id }
                fn inject(&self) -> Vec<ServiceId> { vec![$($key),*] }
                fn setup(self: Arc<Self>, _ctx: Context, _cfg: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
                    Box::pin(async { Ok(FiberOutput) })
                }
            }
        };
    }

    test_plugin!(DummyPlugin, "dummy");
    test_plugin!(GoodPlugin, "good", ServiceId::of::<dyn TestSvc>());
    // trait 侧声明了未登记键（模拟「trait 加了依赖、manifest 没同步」的漂移）
    test_plugin!(DriftPlugin, "drift", ServiceId::of::<dyn DriftSvc>());

    fn good_manifest(inject: Vec<String>) -> PluginManifest {
        PluginManifest {
            id: "good".into(),
            version: "0.1.0".into(),
            level: PluginLevel::Bundled,
            contracts: vec!["svc.v1".into()],
            provides: vec![projection_of::<dyn TestSvc>()],
            inject,
            ui: vec![],
            auto_enable: true,
        }
    }
    #[test]
    fn 默认实现推导最小manifest() {
        let m = DummyPlugin.manifest();
        assert_eq!(m.id, "dummy");
        assert_eq!(m.level, PluginLevel::Bundled);
        assert!(m.auto_enable && !m.version.is_empty());
        assert!(m.contracts.is_empty() && m.provides.is_empty() && m.inject.is_empty() && m.ui.is_empty());
    }

    #[test]
    fn 投影登记并规范化尾段() {
        assert_eq!(projection_of::<dyn TestSvc>(), "dyn TestSvc");
        assert_eq!(projection_of::<dyn DeepModuleInnerSvc>(), "dyn DeepModuleInnerSvc");
        assert_eq!(projection_of_id(&ServiceId::named("legacy")).as_deref(), Some("legacy"));
    }

    #[test]
    fn 校验正例全通过() {
        let m = good_manifest(vec![projection_of::<dyn TestSvc>()]);
        assert!(validate_manifest(&GoodPlugin, &m).is_empty());
    }

    #[test]
    fn 校验反例逐条报() {
        let base = || good_manifest(vec![projection_of::<dyn TestSvc>()]);
        // 反例 a：trait 有键、manifest.inject 空 → 缺失
        let issues = validate_manifest(&GoodPlugin, &good_manifest(vec![]));
        assert!(issues.iter().any(|s| s.contains("缺失")), "{issues:?}");
        // 反例 b：manifest.inject 多余项 → 未声明
        let m = good_manifest(vec![projection_of::<dyn TestSvc>(), "dyn Ghost".into()]);
        let issues = validate_manifest(&GoodPlugin, &m);
        assert!(issues.iter().any(|s| s.contains("未声明")), "{issues:?}");
        // 反例 c：contracts 空；反例 d：contracts 格式错
        let mut m = base();
        m.contracts.clear();
        assert!(validate_manifest(&GoodPlugin, &m).iter().any(|s| s.contains("contracts")));
        let mut m = base();
        m.contracts = vec!["svc".into()];
        assert!(validate_manifest(&GoodPlugin, &m).iter().any(|s| s.contains("格式非法")));
        // 反例 e：provides 拼写不符投影规则；反例 f：trait 键未登记（漂移键查表失败）
        let mut m = base();
        m.provides.push("TestSvc".into());
        assert!(validate_manifest(&GoodPlugin, &m).iter().any(|s| s.contains("投影规则")));
        let issues = validate_manifest(&DriftPlugin, &good_manifest(vec![]));
        assert!(issues.iter().any(|s| s.contains("无投影登记")), "{issues:?}");
        // 反例 g：ui 条目 kind=page 一期拒装（S8 冻结面经 validate_manifest 联动）
        let mut m = base();
        m.ui = vec![crate::manifest_ui::UiEntry::Page {}];
        assert!(validate_manifest(&GoodPlugin, &m).iter().any(|s| s.contains("page")), "{issues:?}");
    }

    #[test]
    fn 格式校验正反用例() {
        // contracts：name.vN（name 非空标识符，可含点/连字；v+纯数字）
        for good in ["tools.v1", "system-prompt.v1", "a_b.v12", "cordis.loader.v1"] {
            assert!(is_contract_format(good), "{good}");
        }
        for bad in ["", "tools", "tools.v", "tools.V1", ".v1", "1tool.v1", "tools.v1.x"] {
            assert!(!is_contract_format(bad), "{bad}");
        }
        // 服务键拼写：dyn <TraitName>（投影尾段标识符，不含路径）
        for good in ["dyn Llm", "dyn RegistryService", "dyn _T"] {
            assert!(is_service_key_format(good), "{good}");
        }
        for bad in ["Llm", "dyn", "dyn ", "dyn llm::Llm", "dyn 1T", "dyn T-2"] {
            assert!(!is_service_key_format(bad), "{bad}");
        }
    }
}
