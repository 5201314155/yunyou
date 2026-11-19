//! yunyou/ 前缀归一器（挂账清理批：rpc.rs 贴顶备案兑现拆出）：
//! B2 三域 wire 形定（云织反骨二次上报，lead 裁）——`yunyou/<域>.<动作>` 点形；
//! 斜杠形别名（openapi 首版）+裸名（现态客户端实测）保留，三形态同效。
//! 返回 String（每请求一次小分配；leak 归一=内存泄漏禁）。

/// 方法名前缀归一：三域（team/tasks/schedule）接 `yunyou/` 前缀别名；
/// 其余域不归一（不扩面）；多级/异常形态不归一（fail-visible 兜底，不猜）。
pub(crate) fn normalize_method(method: &str) -> String {
    let Some(rest) = method.strip_prefix("yunyou/") else { return method.to_string() };
    let domain = rest.split(['.', '/']).next().unwrap_or("");
    match domain {
        "team" | "tasks" | "schedule" => {
            if let Some((head, tail)) = rest.split_once('.') {
                if tail.contains('.') || tail.contains('/') {
                    return method.to_string(); // 多级点形不猜
                }
                return match head {
                    "team" => format!("team/{tail}"),
                    "tasks" => format!("tasks/{tail}"),
                    "schedule" => format!("schedule/{tail}"),
                    _ => method.to_string(),
                };
            }
            // 斜杠形别名（无点）：rest 已是规范形（team/roster）直接归一
            if tail_slash_ok(rest) {
                return rest.to_string();
            }
            method.to_string()
        }
        _ => method.to_string(),
    }
}

/// 斜杠形别名合法性（单级动作，无二级分隔）。
fn tail_slash_ok(rest: &str) -> bool {
    let Some((_, tail)) = rest.split_once('/') else { return false };
    !tail.is_empty() && !tail.contains('/') && !tail.contains('.')
}

#[cfg(test)]
#[path = "rpc_normalize_tests.rs"]
mod rpc_normalize_tests;
