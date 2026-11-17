//! # yunyou-credentials —— 凭据服务（契约⑤）＋ settings 投影服务（契约④）
//!
//! ## 为什么 settings 投影并入本 crate（任务书交由实现席裁决，注释即理由）
//!
//! 1. **语义同源**：投影的 `secret` 密码框标注与凭据的 `role('secret')` 结构性脱敏
//!    （[`redact::redact_secrets`]）共享同一 schema 词汇（`role: "secret"`）与同一
//!    walk 实现——拆成两个 crate 必然单向依赖、徙增接线面与漂移风险；
//! 2. **一期零 UI 面**（方案 §4.1「零界面代码」）：投影只落数据面，独立
//!    `yunyou-settings` crate 一期是空架子（违反「不为封装而封装」）；
//! 3. **可逆**：服务键（[`SettingsProjection`] / [`CredentialService`]）与 manifest
//!    契约声明先冻结，二期 UI 面成熟时把 settings 模块平移到独立 crate 不破契约。
//!
//! ## 契约⑤：密钥永不过插件的手（插件化方案 §3-⑤/§7 风险 7）
//!
//! 配置面（yunyou.toml / 会话 / 日志）只持**凭据引用**（[`reference`] 模块，`$NAME`
//! 形式）；真实密钥只存在宿主托管存储（[`store::HostCredentialStore`]，文件 600
//! 权限）与进程环境。[`CredentialService::resolve`] 返回**消费器句柄**
//! [`SecretHandle`] 而非裸值：值不进任何可序列化/可打印的 API 面（不派生
//! Debug/Clone/Serialize），只能经 [`SecretHandle::with_value`] 在宿主侧消费点
//! （一期=LLM 请求头构造处）使用——插件代码即使拿到句柄，也无法把值写进日志、
//! 配置或会话（API 形状保证 + 审查线纪律；方案 §7 风险 7 的 Rust 一期形态）。
//!
//! ## 官方对照（`packages/credentials/credentials/src/index.ts`，2026-11-14 实读）
//!
//! - `resolve` 每操作一次解析、不缓存跨操作（改凭据下轮即生效，无需重启）；
//! - 空值 = 未配置，处处一致（resolve 跳过、describe 报 configured=false）；
//! - `describe` 只返回 configured/source 等事实面，永不含完整值；
//! - `set` 拒绝空值（用 `unset`）；`unset` 缺失引用 = no-op；
//! - 解析顺序：宿主托管存储优先，进程环境变量兜底（官方 ambient fallback）。
//!
//! 事件面：`reference-updated` 事件类型已在内核 events 预留（S6 交付注记），
//! 本期不发送（发布时机随消费方出现再定，避免死事件）。

pub mod migrate;
pub mod plugin;
pub mod redact;
pub mod reference;
pub mod settings;
pub mod store;

use std::fmt;

pub use migrate::{migrate_plaintext_key, MigrationReport};
pub use plugin::{CredentialsConfig, CredentialsPlugin};
pub use redact::{redact_secrets, RedactedSecret, RedactedValue};
pub use reference::{is_reference, reference_name, validate_reference, LLM_API_KEY_REF};
pub use settings::{
    project_form, FormDescription, FormField, FormFieldType, SettingsProjection, SettingsProjector,
};
pub use store::HostCredentialStore;
use yunyou::Result;
use yunyou_util::BoxFut;

/// TOML 解析错误脱敏摘要（挑刺报告 20261117 F-2）：只透出 `message()` 与行号，
/// **严禁**透传错误的 Display 全文——其中含出错行原文，明文 key 会随错误信息
/// 进日志（迁移窗口期 credentials.toml/yunyou.toml 内常有真实密钥）。
///
/// 行号从 span 起始字节偏移对源文本数换行符推得；span 缺失报第 1 行（缺省锚点）。
pub(crate) fn toml_error_summary(text: &str, err: &toml::de::Error) -> String {
    let line = err
        .span()
        .map(|span| {
            let end = span.start.min(text.len());
            text.as_bytes()[..end].iter().filter(|b| **b == b'\n').count() + 1
        })
        .unwrap_or(1);
    format!("第 {line} 行附近：{}", err.message())
}

/// 凭据服务（契约⑤）：配置面持引用，本服务与宿主消费点之间流动的才是值。
///
/// # 防泄漏三律（实现者必须遵守，审查线一打回项）
/// 1. `resolve` 的返回值是 [`SecretHandle`] 消费器，**严禁**任何绕过句柄取值的
///    API（如返回 `&str`/`String` 的方法）；
/// 2. `describe` 永不返回完整值（configured/source 等事实面即可）；
/// 3. `set` 拒绝空值（官方铁律：空值=未配置，处处一致）。
pub trait CredentialService: Send + Sync + 'static {
    /// 解析一个凭据引用为一次性消费句柄。
    ///
    /// 每操作解析一次（调用方不得缓存句柄跨操作——那是「改凭据不重启不生效」的
    /// 官方语义根基）。未配置（存储与环境均无值，或存储值为空串）→ `Ok(None)`；
    /// 引用语法非法 → `Err`（fail-loud，见 [`reference::validate_reference`]）。
    fn resolve(&self, reference: &str) -> BoxFut<'static, Result<Option<SecretHandle>>>;

    /// 描述一个凭据引用（配置面用）：是否已配置 + 供给层，永不含值。
    fn describe(&self, reference: &str) -> BoxFut<'static, Result<CredentialInfo>>;

    /// 把值写入宿主托管层（存量明文迁移器的唯一写入口；运行期不暴露给插件）。
    /// 空值拒绝（用 [`CredentialService::unset`]）；写后落盘（600 权限原子写）。
    fn set(&self, reference: &str, value: &str) -> BoxFut<'static, Result<()>>;

    /// 移除宿主托管层的引用（缺失 = no-op，对齐官方 unset 语义）。
    fn unset(&self, reference: &str) -> BoxFut<'static, Result<()>>;
}

/// 引用的配置面事实（永不含值；官方 CredentialInfo 一期裁剪版）。
///
/// `writable`/`masked`（官方 CredentialInfo 另两字段）一期不做：一期宿主托管层
/// 恒可写、掩码预览无消费面（注释即裁决，二期随配置 UI 面补）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialInfo {
    /// 解析该引用当前是否会返回值（存储与环境均无值/值为空 → false）。
    pub configured: bool,
    /// 当前供给层：`"store"`（宿主托管文件）或 `"env"`（进程环境变量兜底）。
    pub source: Option<String>,
}

/// 一次性密钥消费句柄（方案 §7 风险 7 的 API 面防线）。
///
/// - **不可 Clone / 不可序列化 / Debug 全脱敏**：值进不了日志与会话；
/// - 值的唯一出口是 [`SecretHandle::with_value`]：宿主侧消费点以闭包借用的形式
///   使用（一期=LLM 请求头构造处，见 rustdoc 顶注），用完即弃；
/// - `Drop` 释放内部字符串（replace 空串）。一期口径：任务书红线是「key 零入 git
///   零入日志」，内存页清零（zeroize 级防护）不在一期威胁模型内，二期评估。
pub struct SecretHandle {
    value: String,
}

impl SecretHandle {
    /// 宿主侧消费器：值只经本出口流向 LLM 请求头等宿主消费点。
    ///
    /// 闭包拿到 `&str` 借用；**纪律**：闭包内不得把值 clone/写盘/写日志
    /// （rustdoc 契约+审查线一打回项）。
    pub fn with_value<R>(&self, consume: impl FnOnce(&str) -> R) -> R {
        consume(&self.value)
    }

    /// crate 内构造口（store 实现与测试用；对外经 [`CredentialService::resolve`] 获取）。
    pub(crate) fn new(value: String) -> Self {
        Self { value }
    }
}

impl fmt::Debug for SecretHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 全脱敏：Debug 输出绝不携带值（「key 零入日志」的结构性保证，测试有断言）
        f.write_str("SecretHandle([secret 已脱敏])")
    }
}

impl Drop for SecretHandle {
    fn drop(&mut self) {
        // 释放即清引用（不承诺内存页清零，见类型级 rustdoc 一期口径）
        self.value = String::new();
    }
}
