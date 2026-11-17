//! 变量插值 `{{var}}`（对标官方 `interpolate()`，system-prompt/src/index.ts:347-386）。
//!
//! # 语义（本 crate 的裁断，偏离处已注明理由）
//! - **畸形引用抛错**：`{{}}`、`{{ name }}`、`{{Name}}`、花括号嵌套等一律
//!   [`crate::error::SystemPromptError::MalformedVariable`]（与官方一致）；
//!   起点是**注册期**（`Section::validate`）——装载期 fail loud，
//!   比运行期才炸更早也更可定位。
//! - **未注册变量显式降级**：官方对未注册变量抛错（`unknown prompt variable`，:372-375）；
//!   yunyou 判为**空串 + `tracing::warn!`**——compose 在每轮热路径上且签名不可失败
//!   （消费方 `SystemPrompt::compose -> String` 在辖区外），一个轮次变量缺失不应让
//!   整份提示词装配失败；缺失必须留告警痕迹，不许静默吞掉。
//! - 孤立的一个 `{{`（后文再无 `}}`）是**字面散文**，原样保留（对标官方 :357-363）。
//! - 替换值不再二次扫描（对标官方 :376 之后直接推进 cursor）。

use std::collections::HashMap;

/// 合法变量名（对标官方 `VARIABLE_NAME = /^[a-z][a-z0-9_]*$/`，:203）。
pub fn is_valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(head) if head.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 扫描到的一个完整 `{{name}}` 组（字节下标，`{{`/`}}` 均为 ASCII，切片安全）。
#[derive(Debug, Clone, Copy)]
struct Group {
    /// 组起点（`{{` 的偏移）。
    start: usize,
    /// 变量名起点。
    name_start: usize,
    /// 变量名止点（`}}` 的起点）。
    name_end: usize,
    /// 组终点（`}}` 之后）。
    end: usize,
}

/// 从 `from` 起找下一个完整 `{{name}}` 组；没有（或无 `}}`）则 `None`。
fn next_group(text: &str, from: usize) -> Option<Group> {
    let open = text.get(from..)?.find("{{")? + from;
    let name_start = open + 2;
    let close = text.get(name_start..)?.find("}}")? + name_start;
    Some(Group { start: open, name_start, name_end: close, end: close + 2 })
}

/// 畸形引用扫描：只查「引用写坏没写坏」，不解析变量值（注册期校验用）。
///
/// 返回畸形片段的原文（如 `{{}}`）以便报错可读。
pub fn scan_malformed(text: &str) -> Result<(), String> {
    let mut from = 0;
    while let Some(group) = next_group(text, from) {
        let name = &text[group.name_start..group.name_end];
        if name.contains('{') || name.contains('}') || !is_valid_variable_name(name) {
            return Err(text[group.start..group.end].to_string());
        }
        from = group.end;
    }
    Ok(())
}

/// 插值：合法组按变量表替换，未注册变量＝空串 + 告警，畸形组按字面保留 + 错误日志
/// （畸形文本应由注册期拦截，走到这里说明调用方绕过了校验，故记 error 而非静默）。
pub fn interpolate(text: &str, variables: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut last = 0;
    let mut from = 0;
    while let Some(group) = next_group(text, from) {
        let name = &text[group.name_start..group.name_end];
        if !is_valid_variable_name(name) {
            tracing::error!("插值遇到未经注册期校验的文本（畸形变量名 {name:?}），按字面保留");
            from = group.end;
            continue;
        }
        out.push_str(&text[last..group.start]);
        match variables.get(name) {
            Some(value) => out.push_str(value),
            None => tracing::warn!(
                "提示变量 {name} 未注册，按空串处理（官方语义为抛错；yunyou 显式降级，见模块文档）"
            ),
        }
        last = group.end;
        from = group.end;
    }
    out.push_str(&text[last..]);
    out
}