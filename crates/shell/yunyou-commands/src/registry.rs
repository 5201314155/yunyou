//! 命令注册表默认实现：`RwLock<BTreeMap>` 持全部命令，顺序稳定（设计 §二）。
//!
//! - key=精确名：BTreeMap 的字典序即 [`CommandsService::list`] 的稳定输出序；
//! - 别名不建索引：[`CommandsService::find`] 先查一次精确名，未命中再按字典序
//!   遍历各命令的 aliases——命令量级小（<100），O(n) 遍历换来单表维护
//!   （双表同步的写分歧风险不引入）；「别名不遮蔽任何精确名」由
//!   「先查全表精确名、再遍历别名」的顺序直接保证；
//! - 锁纪律（内核 §2.3 同款）：读写锁绝不跨 await——register/除名均在锁内
//!   完成纯内存操作，disposer 的异步壳只包摘表一步。

use crate::{CommandDefinition, CommandDescriptor, CommandsService};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock, Weak};
use yunyou::{CordisError, Disposer, ErrorKind, Result};

/// 注册表条目：定义本体（handler 随定义存取）。
struct Entry {
    def: CommandDefinition,
}

/// 命令注册表（[`CommandsService`] 默认实现）。
///
/// 内部状态经 `Arc` 共享：Clone 廉价（共享同一注册表，服务句柄语义）；
/// disposer 只持弱引用，注册表实例析构后 disposer 变为 no-op（不复活、不悬挂）。
#[derive(Default, Clone)]
pub struct CommandRegistry {
    entries: Arc<RwLock<BTreeMap<String, Entry>>>,
}

impl CommandRegistry {
    /// 建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 定义 → 只读目录投影（剥离 handler）。
    fn descriptor(def: &CommandDefinition) -> CommandDescriptor {
        CommandDescriptor {
            name: def.name.clone(),
            aliases: def.aliases.clone(),
            description: def.description.clone(),
            kind: def.kind,
            accepts_args: def.accepts_args,
        }
    }
}

impl CommandsService for CommandRegistry {
    fn register(&self, def: CommandDefinition) -> Result<Disposer> {
        let name = def.name.clone();
        let mut entries = self.entries.write().expect("命令表锁");
        if entries.contains_key(&name) {
            return Err(CordisError::new(
                ErrorKind::ServiceDuplicate,
                format!("命令 \"/{name}\" 已注册，拒绝静默覆盖"),
            ));
        }
        // 别名×别名冲突检测（task-17②）：同名 fail-loud 纪律补齐别名面——
        // 此前仅查主名，**别名撞别名**按 find 的字典序遍历**静默**胜出（实测
        // 技能 alias「退出」可劫持壳命令 /quit 的别名「退出」）。只拦别名×
        // 别名：别名×主名的两个方向由设计解析规则「别名不遮蔽任何精确名」
        // （精确名优先）定义共存语义，不属遮蔽缝（既有测试即依赖该共存）。
        for alias in &def.aliases {
            if entries.values().any(|entry| entry.def.aliases.iter().any(|a| a == alias)) {
                return Err(CordisError::new(
                    ErrorKind::ServiceDuplicate,
                    format!("命令 \"/{name}\" 的别名 \"{alias}\" 与既有命令别名冲突，拒绝静默遮蔽"),
                ));
            }
        }
        entries.insert(name.clone(), Entry { def });
        drop(entries); // 写锁不跨出本函数；disposer 构造不再持锁
        let table: Weak<RwLock<BTreeMap<String, Entry>>> = Arc::downgrade(&self.entries);
        Ok(Disposer::new(move || {
            let table = table.clone();
            Box::pin(async move {
                if let Some(table) = table.upgrade() {
                    table.write().expect("命令表锁").remove(&name);
                }
            })
        }))
    }

    fn list(&self) -> Vec<CommandDescriptor> {
        let entries = self.entries.read().expect("命令表锁");
        entries.values().map(|entry| Self::descriptor(&entry.def)).collect()
    }

    fn find(&self, token: &str) -> Option<CommandDescriptor> {
        let entries = self.entries.read().expect("命令表锁");
        if let Some(entry) = entries.get(token) {
            return Some(Self::descriptor(&entry.def));
        }
        entries
            .values()
            .find(|entry| entry.def.aliases.iter().any(|alias| alias == token))
            .map(|entry| Self::descriptor(&entry.def))
    }

    fn handler_of(&self, token: &str) -> Option<crate::CommandHandler> {
        let entries = self.entries.read().expect("命令表锁");
        if let Some(entry) = entries.get(token) {
            return Some(entry.def.handler.clone());
        }
        entries
            .values()
            .find(|entry| entry.def.aliases.iter().any(|alias| alias == token))
            .map(|entry| entry.def.handler.clone())
    }
}
