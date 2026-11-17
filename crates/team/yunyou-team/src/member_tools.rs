//! 成员工具面：包装全局注册表——deny 三件（lead 专属派工权）+覆写三件（身份化：
//! 发消息=成员自己 fiber 身份、认领/完成=自己名下）。包装而非复制：全局注册表
//! 的存量工具（read/bash/glob/grep/…）零拷贝直通，新工具自动可见（无清单漂移）。

use crate::tools::{SendTool, TaskClaimTool, TaskSubmitTool};
use yunyou::WeakContext;
use yunyou_llm::ToolSchema;
use yunyou_tools::contract::ToolDefinition;
use yunyou_tools::RegistryService;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// 成员不可见的 lead 专属工具（AI 自拉人/建任务/改派=派工权，五点规格①+业主
/// 架构「主 agent 派发/管理/踢人」；成员对任务分配的异议走消息面向 lead 提请）。
const LEAD_ONLY: [&str; 5] = [
    "spawn_teammate",
    "team_task_create",
    "team_task_reassign",
    "team_kick",
    // 审查权=派工权（写审分离 K-8）：审查裁决 lead 专属，成员异议走消息面
    "team_task_review",
];

/// 成员注册表（object-safe 代理；schemas/get 都过 deny+override 闸）。
pub(crate) struct MemberRegistry {
    global: Arc<dyn RegistryService>,
    deny: HashSet<&'static str>,
    overrides: BTreeMap<String, Arc<dyn ToolDefinition>>,
}

impl RegistryService for MemberRegistry {
    fn get(&self, name: &str) -> Option<Arc<dyn ToolDefinition>> {
        if self.deny.contains(name) {
            return None;
        }
        if let Some(tool) = self.overrides.get(name) {
            return Some(Arc::clone(tool));
        }
        self.global.get(name)
    }

    fn register_tool(self: Arc<Self>, _tool: Arc<dyn ToolDefinition>) -> Result<yunyou::Disposer, yunyou::CordisError> {
        // 成员工具面=只读视图（deny+override 包装），注册是写操作不越权；
        // 成员不需要也不能注册新工具（工具面收敛在 lead 侧，防面膨胀）
        Err(yunyou::CordisError::new(
            yunyou::ErrorKind::ServiceDuplicate,
            "成员工具面是只读视图，不接受工具注册",
        ))
    }

    fn schemas(&self) -> Vec<ToolSchema> {
        self.global
            .schemas()
            .into_iter()
            .filter(|schema| !self.deny.contains(schema.name.as_str()))
            .map(|schema| match self.overrides.get(&schema.name) {
                Some(tool) => ToolSchema {
                    name: schema.name,
                    description: tool.description().to_string(),
                    parameters: tool.parameters(),
                },
                None => schema,
            })
            .collect()
    }
}

/// 构造成员工具面（member.rs setup 调用）：团队服务经 ctx.get 取用（服务在
/// 默认作用域共享），身份化覆写绑定成员自己的 fiber 弱句柄。
pub fn member_registry(
    ctx: &yunyou::Context,
    member_name: &str,
    global: Arc<dyn RegistryService>,
) -> yunyou::Result<Arc<dyn RegistryService>> {
    let service = ctx.get::<dyn crate::task_types::TeamService>(yunyou::ServiceId::of::<
        dyn crate::task_types::TeamService,
    >())?;
    let member_ctx: WeakContext = ctx.downgrade();
    let mut overrides: BTreeMap<String, Arc<dyn ToolDefinition>> = BTreeMap::new();
    overrides.insert(
        "send_message".into(),
        Arc::new(SendTool {
            service: Arc::clone(&service),
            actor: Some(member_name.to_string()),
            member_ctx: Some(member_ctx),
        }),
    );
    overrides.insert(
        "team_task_claim".into(),
        Arc::new(TaskClaimTool { actor: Some(member_name.to_string()), service: Arc::clone(&service) }),
    );
    overrides.insert(
        "team_task_submit".into(),
        Arc::new(TaskSubmitTool { actor: Some(member_name.to_string()), service }),
    );
    Ok(Arc::new(MemberRegistry { global, deny: LEAD_ONLY.into_iter().collect(), overrides }))
}
