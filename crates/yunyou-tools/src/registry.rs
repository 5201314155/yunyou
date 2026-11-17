//! 工具注册表：名字 → 定义；注册即 effect 撤销（方案 §2.1.4，对标
//! `tools/src/index.ts:127-157` 的 register 返回 disposer）。
//!
//! 撤销器对注册表持 **Weak**：注册表的存活归 fiber 的服务引用计数管，
//! disposer 不延长其生命（避免 Arc 循环）。

use crate::contract::{PresentationMeta, ToolDefinition};
use yunyou::{CordisError, Disposer, ErrorKind};
use yunyou_llm;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// 注册表服务 trait（object-safe；消费方 agent-loop 经 ctx.get 取用）。
pub trait RegistryService: Send + Sync {
    /// 按名取工具。
    fn get(&self, name: &str) -> Option<Arc<dyn ToolDefinition>>;

    /// 注册工具（跨插件注册缝，K-7/任务26 T2）：返回撤销器，调用方登记为
    /// 自己 fiber 的 effect 即「注册即撤销」（方案 §2.1.4）。只读视图实现
    /// （如成员工具面包装）返回 Err 拒绝——注册是写操作，包装视图不越权。
    fn register_tool(self: Arc<Self>, tool: Arc<dyn ToolDefinition>) -> Result<Disposer, CordisError>;
    /// 全部工具的模型可见声明（顺序稳定）。
    fn schemas(&self) -> Vec<yunyou_llm::ToolSchema>;

    /// 按名取工具的超时声明（毫秒；未注册＝`None`；`None`＝用全局 tool_timeout_secs）。
    ///
    /// 契约⑥配套访问器：默认实现直取（不缓存）——工具可注册/注销，直取保证
    /// 与注册表态恒一致；元数据访问低频（调度/渲染决策），O(log n) 足够。
    /// **白名单反向印证：本访问器不在模型面**（模型面只有 schemas()）。
    fn timeout_ms(&self, name: &str) -> Option<u64> {
        self.get(name).and_then(|tool| tool.timeout_ms())
    }

    /// 按名取并发安全声明（未注册按保守 `false`——没声明＝不假定可并发）。
    fn is_concurrency_safe(&self, name: &str) -> bool {
        self.get(name).is_some_and(|tool| tool.is_concurrency_safe())
    }

    /// 按名取卡片呈现元数据（未注册＝`None`；渲染层回落统一卡片）。
    fn presentation_meta(&self, name: &str) -> Option<PresentationMeta> {
        self.get(name).and_then(|tool| tool.presentation_meta())
    }
}

/// 工具注册表服务。
pub struct ToolsRegistry {
    /// BTreeMap 保证 schemas() 输出顺序稳定（提示词可复现）。
    tools: RwLock<BTreeMap<String, Arc<dyn ToolDefinition>>>,
}

impl RegistryService for ToolsRegistry {
    fn get(&self, name: &str) -> Option<Arc<dyn ToolDefinition>> {
        self.tools.read().expect("工具表锁").get(name).cloned()
    }

    fn register_tool(self: Arc<Self>, tool: Arc<dyn ToolDefinition>) -> Result<Disposer, CordisError> {
        // 委托既有固有实现（Weak 撤销器语义零改动）；Arc<Self> 接收器=dyn 兼容。
        // 名字取 register_tool 与固有 register 区分：固有 &Arc<Self> 版被既有
        // 调用点（含 S4 冻结测试）依赖，trait 不遮蔽它
        ToolsRegistry::register(&self, tool)
    }

    fn schemas(&self) -> Vec<yunyou_llm::ToolSchema> {
        self.tools
            .read()
            .expect("工具表锁")
            .values()
            .map(|tool| yunyou_llm::ToolSchema {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters(),
            })
            .collect()
    }
}

impl ToolsRegistry {
    /// 建空注册表（返回共享句柄，供 plugin 注册进内核）。
    pub fn new() -> Arc<Self> {
        Arc::new(Self { tools: RwLock::new(BTreeMap::new()) })
    }

    /// 注册工具，返回撤销器（放进 fiber 的 effect 即随卸载自动注销）。
    ///
    /// # 失败语义
    /// 同名工具已存在 → `Err(ServiceDuplicate)`（模型面的名字必须唯一，
    /// 对标 TS register 的保留名校验）。
    pub fn register(
        self: &Arc<Self>,
        tool: Arc<dyn ToolDefinition>,
    ) -> Result<Disposer, CordisError> {
        let name = tool.name().to_string();
        {
            let mut table = self.tools.write().expect("工具表锁");
            if table.contains_key(&name) {
                return Err(CordisError::new(
                    ErrorKind::ServiceDuplicate,
                    format!("tool/工具 \"{name}\" 已注册"),
                ));
            }
            table.insert(name.clone(), tool);
        }
        let weak = Arc::downgrade(self);
        Ok(Disposer::new(move || {
            let weak = weak.clone();
            Box::pin(async move {
                if let Some(table) = weak.upgrade() {
                    table.tools.write().expect("工具表锁").remove(&name);
                }
            })
        }))
    }
}
