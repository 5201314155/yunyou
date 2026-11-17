//! 工具输出 8KB 降级预览（任务30 三期②）。
//!
//! 职责：canonical value 序列化超 8KB 时，事件侧降级文案从「详情见会话
//! 文件」改为「首 3 行预览 + 已折叠 N 行」——旧文案对终端用户无操作意义
//! （会话文件路径不进终端、用户无从点开，业主实锤 nit）；预览先给出结果
//! 头部最关键信息，折叠计数说明剩余量级。信息无损性不受影响：完整版本
//! 始终落会话 JSONL，降级只作用于事件通道。
//!
//! 主文本定位：取 value 内**最长的字符串叶子**——内置工具的输出主体均为
//! 单一长字符串字段（bash 的 output、read 的 content、grep 的各匹配项等），
//! 最长叶子即输出主体，无需按工具名映射（新增工具零接线）。

use serde_json::Value;
use yunyou_session::PREVIEW_LINES;

// 预览直出行数单源化（task-22 尾件，lead 裁方案甲）：原本文件私有常量上移
// yunyou-session（与 cli render detail 同消费方，跨 crate 无法字面共享的旧
// 注记随常量退役），值锁测试在 session lib.rs。

/// 8KB 超限降级文案：`[输出超 8KB，已折叠]` + 首 3 行 + `… 已折叠 N 行`。
pub(crate) fn fold_preview(value: &Value) -> String {
    let text = longest_leaf(value).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::from("[输出超 8KB，已折叠]");
    for line in lines.iter().take(PREVIEW_LINES) {
        out.push('\n');
        out.push_str(line);
    }
    if lines.len() > PREVIEW_LINES {
        out.push_str(&format!("\n… 已折叠 {} 行", lines.len() - PREVIEW_LINES));
    }
    out
}

/// 最长字符串叶子（递归；对象/数组按声明序取最长——输出主体是唯一长串）。
fn longest_leaf(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s),
        Value::Array(items) => items.iter().filter_map(longest_leaf).max_by_key(|s| s.len()),
        Value::Object(map) => map.values().filter_map(longest_leaf).max_by_key(|s| s.len()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// bash 型：output 长文本 → 首 10 行预览 + 折叠计数（任务30 四期 D-A 拉齐）。
    #[test]
    fn bash_型_首十行加折叠计数() {
        let output = (1..=13).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n");
        let value = json!({"exit_code": 0, "output": output});
        let text = fold_preview(&value);
        let expect_head: String = (1..=10).map(|i| format!("\nl{i}")).collect();
        assert_eq!(text, format!("[输出超 8KB，已折叠]{expect_head}\n… 已折叠 3 行"), "{text:?}");
        assert!(!text.contains("会话文件"), "旧文案不得残留：{text:?}");
    }

    /// read 型：content 恰 10 行 → 无折叠计数行（全部可见）。
    #[test]
    fn read_型_恰好十行无折叠行() {
        let content = (1..=10).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
        let value = json!({"content": content});
        let expect: String = (1..=10).map(|i| format!("\nL{i}")).collect();
        assert_eq!(fold_preview(&value), format!("[输出超 8KB，已折叠]{expect}"));
    }

    /// 纯字符串 value（降级形态嵌套：降级结果本身）→ 直接预览。
    #[test]
    fn 纯字符串value() {
        let lines = (1..=12).map(|i| format!("第{i}行")).collect::<Vec<_>>().join("\n");
        let expect_head: String = (1..=10).map(|i| format!("\n第{i}行")).collect();
        assert_eq!(fold_preview(&json!(lines)),
            format!("[输出超 8KB，已折叠]{expect_head}\n… 已折叠 2 行"));
    }

    /// 无字符串叶子（数值/空对象）→ 只有头行不伪造预览。
    #[test]
    fn 无字符串叶子_只有头行() {
        assert_eq!(fold_preview(&json!({"count": 42})), "[输出超 8KB，已折叠]");
        assert_eq!(fold_preview(&json!({})), "[输出超 8KB，已折叠]");
    }

    /// 数组匹配项：最长项为预览主体（grep 多行匹配拼接场景的兜底）。
    #[test]
    fn 数组取最长叶子() {
        let value = json!({"matches": ["短", "很长很长很长很长\n的第二行"], "count": 2});
        let text = fold_preview(&value);
        assert!(text.contains("的第二行"), "应取最长叶子：{text:?}");
    }
}
