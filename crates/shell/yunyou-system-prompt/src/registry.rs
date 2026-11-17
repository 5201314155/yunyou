//! 段注册表：注册即 effect、卸载即回收（契约③，对标官方 `PromptLayer.sections`
//! NamedEntries + `systemPrompt.section()`，system-prompt/src/index.ts:400-492）。
//!
//! # 同名语义（本 crate 裁断，与官方的差异及理由）
//! 官方「同层重名抛错」，是靠 **ScopedLayers 分层**（scoped 层 shadow global 层）实现的
//! （:400-419）；yunyou 一期只有单一全局层、无 scope 键（ScopedLayers 归 P1），
//! 因此**同名注册＝替换（shadow 栈）**：后注册者顶掉先注册者成为生效文本，
//! 而每个 disposer 仍精确对应自己那一次注册——卸下后来者，先前那层重新生效。
//! 这正是官方人设槽（`deployment:persona-prefix`）赖以「preset 被同名替换而非重复」
//! 的机制本体（:189-200 注释）。分层落地后本语义自然收敛为「同层替换、跨层 shadow」。
//!
//! # 与内核 effect 模式的对齐
//! [`SectionRegistry::register_section`] 返回内核 [`yunyou::Disposer`]，插件在 `setup()`
//! 里照内核模式接成 effect 即可（注册即 effect、fiber 卸载逆序清理、卸载不留残尸）：
//!
//! ```ignore
//! let disposer = registry.register_section(Section::new("my:policy", 500.0, "……"))?;
//! ctx.effect("systemPrompt.section", move || Ok(Some(disposer)))?;
//! ```

use crate::error::SystemPromptError;
use crate::interpolate::is_valid_variable_name;
use crate::section::Section;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use yunyou::Disposer;

/// 注册表内部状态（std Mutex，临界区内无 await，遵守内核锁纪律）。
#[derive(Default, Debug)]
struct RegistryInner {
    next_id: u64,
    /// (注册 id, 段) 按注册序保存；id 是 disposer 的精确摘除依据。
    records: Vec<(u64, Section)>,
    /// 变量表（同名替换，与段一致）。
    variables: HashMap<String, String>,
}

/// 段注册表（`Arc` 持有：disposer 与装配都要能拿到它）。
#[derive(Default, Debug)]
pub struct SectionRegistry {
    inner: Mutex<RegistryInner>,
}

impl SectionRegistry {
    /// 注册一个提示段，返回内核 Disposer（注册即 effect，显式 dispose 或 fiber 卸载皆可注销）。
    ///
    /// 装载期校验（对标官方 `section()` 入口检查，:479-492）：段名非空、段序有限、
    /// 文本无畸形变量引用；任一不过即 [`SystemPromptError`]，**不落表**。
    pub fn register_section(
        self: &Arc<Self>,
        section: Section,
    ) -> Result<Disposer, SystemPromptError> {
        section.validate()?;
        let id = {
            let mut inner = self.inner.lock().expect("段注册表锁未被毒化");
            let id = inner.next_id;
            inner.next_id += 1;
            inner.records.push((id, section));
            id
        };
        let registry = Arc::clone(self);
        Ok(Disposer::new(move || {
            Box::pin(async move { registry.remove(id) })
        }))
    }

    /// 登记一个提示变量（同名替换；变量名合法性对标官方 `VARIABLE_NAME`）。
    ///
    /// 变量无 disposer：变量是「本轮应为模型提供的事实」，替换即刷新，不占段位。
    pub fn register_variable(
        &self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), SystemPromptError> {
        let name = name.into();
        if !is_valid_variable_name(&name) {
            return Err(SystemPromptError::InvalidVariableName { name });
        }
        self.inner
            .lock()
            .expect("段注册表锁未被毒化")
            .variables
            .insert(name, value.into());
        Ok(())
    }

    /// 记录快照（注册序，供装配期做同名 shadow 归并）。
    pub(crate) fn records(&self) -> Vec<Section> {
        self.inner
            .lock()
            .expect("段注册表锁未被毒化")
            .records
            .iter()
            .map(|(_, section)| section.clone())
            .collect()
    }

    /// 变量表快照（装配期取用）。
    pub(crate) fn variables(&self) -> HashMap<String, String> {
        self.inner.lock().expect("段注册表锁未被毒化").variables.clone()
    }

    /// 按注册 id 注销（幂等：已注销或从未存在都安静返回）。
    fn remove(&self, id: u64) {
        self.inner
            .lock()
            .expect("段注册表锁未被毒化")
            .records
            .retain(|(record_id, _)| *record_id != id);
    }
}