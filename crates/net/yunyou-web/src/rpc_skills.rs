//! skills 域 unary RPC（skills 下沉批，契约 web.v1 规格.md §9.2+域细则 §3）：
//! `skills/list`（技能目录）+ `skills/get`（正文，发送须用户显式动作）。
//! 纯函数面=yunyou-skills crate（零内核依赖下沉，web/cli 平行消费）。
//!
//! # source 诚实纪律（云枢注记，域细则 §3-1）
//! source 三值中 builtin/plugin 层 yunyou 现态无实料目录——v1 投影恒 "project"
//! （双槽扫描=项目根 .yunyou/.dsh），不虚构其他层。

use crate::contract::{self, code, err_envelope};
use crate::server::WebState;
use serde_json::{json, Value};
use std::sync::Arc;

/// 技能槽根（项目根=进程 cwd，与 cli 扫描同源）。
fn skill_root() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// skills/list（L1）：双槽扫描目录（id/name/description/source="project"）。
pub(crate) fn skills_list(_state: &Arc<WebState>) -> Value {
    let items: Vec<Value> = yunyou_skills::scan_skill_catalog(&skill_root())
        .iter()
        .map(|entry| {
            json!({
                "id": entry.name,
                "name": entry.name,
                "description": entry.description,
                "source": "project",
            })
        })
        .collect();
    json!({"ok": true, "value": {"items": items}})
}

/// skills/get（L1，payload {id!}）：正文（发送须用户显式动作——契约 §9.2-3，
/// 不是可执行 RPC）。
pub(crate) fn skills_get(_state: &Arc<WebState>, payload: &Value) -> Value {
    let request = contract::request_of(payload);
    let Some(id) = request.get("id").and_then(Value::as_str) else {
        return err_envelope(code::BAD_REQUEST, "get 缺 id（skills/list 查目录）");
    };
    match yunyou_skills::read_skill_body(&skill_root(), id) {
        Some((description, body)) => json!({"ok": true, "value": {
            "id": id,
            "name": id,
            "description": description,
            "body": body,
        }}),
        None => err_envelope(
            code::BAD_REQUEST,
            &format!("技能 {id} 不存在或不可目录化（无一级标题/非法名；skills/list 查目录）"),
        ),
    }
}

#[cfg(test)]
#[path = "rpc_skills_tests.rs"]
mod rpc_skills_tests;
