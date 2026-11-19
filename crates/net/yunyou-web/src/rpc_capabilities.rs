//! capabilities 域 unary RPC（契约 web.v1 规格.md §9.3；W3 裁定「随团队批接」，
//! 依赖于 W3-W6 逐批齐备——本件为挂账清偿）：`capabilities/list`。
//!
//! # 诚实投影纪律
//! available=对应服务缝在根装配中在册（ctx.get 实测，不静态硬编）；available=false
//! 必带 gate（未开闸缝/未落批点名）——前端据此灰置并说明，不出假开关（规格 §9.3）。

use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;
use yunyou::{ServiceId};
use yunyou_commands::CommandsService;
use yunyou_credentials::CredentialService;
use yunyou_tools::RegistryService;
use yunyou_team::TeamService;

/// capabilities/list（L1）：能力开闸投影。服务在册探测=ctx.get 实测
/// （装配事实，非硬编；typed get 逐缝取用）。
pub(crate) fn capabilities_list(state: &Arc<WebState>) -> Value {
    let root = state.turns.root();
    let tools = root
        .get::<dyn RegistryService>(ServiceId::of::<dyn RegistryService>())
        .is_ok();
    let commands = root
        .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
        .is_ok();
    let team = root
        .get::<dyn TeamService>(ServiceId::of::<dyn TeamService>())
        .is_ok();
    let credentials = root
        .get::<dyn CredentialService>(ServiceId::of::<dyn CredentialService>())
        .is_ok();
    let items = vec![
        // 会话与工具：web 面本体（session 域 [已接] 恒可用）
        entry("session", "会话与对话", "会话/回合/工具事件流", true, None),
        entry("commands", "斜杠命令", "命令目录与执行（D2 落盘同源）", commands, gate_if(commands, "commands 服务未就绪")),
        entry("tools", "内置工具", "文件/搜索/bash/patch 及沙箱执行", tools, gate_if(tools, "工具注册表未就绪")),
        entry("team", "团队协作", "名册/成员消息/共享任务板（写审分离）", team, gate_if(team, "团队插件未启用")),
        entry("scheduler", "定时任务", "At/Interval/Cron 三型（会话内有效）", true, Some("durable 跨重启持久化未闭环（S4）：仅会话内有效，不给跨重启开关")),
        entry("settings", "设置与凭据", "schema 投影/修订写/凭据只写不回显", credentials, gate_if(credentials, "凭据插件未启用")),
        entry("skills", "技能目录", "技能查看与展开回填", false, Some("技能目录下沉批（yunyou-skills）未落：目录投影候共享模块就位")),
        entry("market", "插件市场", "market 插件列表与安装", false, Some("工作区授权（S11 Landlock+seccomp）与 bundle 补丁缝未开闸")),
        entry("plugins.ui", "Web 插件 UI", "ui kind=page 页面插件", false, Some("manifest ui kind=page 二期预留（拒装，manifest 契约 §3）")),
    ];
    json!({"ok": true, "value": {"items": items}})
}

/// 条目成形（available=false 必带 gate；true 则 gate 缺席）。
fn entry(key: &str, title: &str, description: &str, available: bool, gate: Option<&str>) -> Value {
    let mut row = json!({
        "key": key,
        "title": title,
        "description": description,
        "available": available,
    });
    if let Some(gate) = gate {
        row["gate"] = Value::String(gate.to_string());
    }
    row
}

/// available=false 时给 gate；true 时 None（不出假开关也不噪音）。
fn gate_if(available: bool, gate: &'static str) -> Option<&'static str> {
    (!available).then_some(gate)
}

#[cfg(test)]
#[path = "rpc_capabilities_tests.rs"]
mod rpc_capabilities_tests;
