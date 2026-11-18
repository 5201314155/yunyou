//! 工具渲染契约 trait + 注册表（TUI 大改设计稿 件一 T1-a，lead 批 2026-11-17）。
//!
//! 对标 claude code Tool.ts 渲染契约（〈挖掘〉§5.1 Tool.ts:529-711）与
//! 挖矿-TUI施工规格 A1-A2：**工具向 TUI 注册渲染函数，而非 TUI 认识每个
//! 工具**。本批只立缝（零行为变更）：注册表按工具名分发到既有实现
//! （humanize 动词摘要 / adapter parse 详情解析），逻辑搬家入各工具实现
//! 留 T1-b；五状态钩子（queued/rejected/progress）随 T1-c 增补。
//!
//! 契约原则（写进本文件防回退）：模型序列化与 UI 渲染彻底分离——本 trait
//! 只吃 canonical value 出渲染文本，不碰模型可见面。

use super::event::DetailKind;

/// 工具渲染器契约（每工具一个实现；缺省方法有通用兜底语义）。
pub(crate) trait ToolRenderer {
    /// 完成卡人话摘要（对标 renderToolResultMessage 缺省形态）：
    /// 返回 (摘要文本, has_exit)——has_exit 声明句内已含退出码段，
    /// 调用方（adapter）只对未声明者追加「· 退出码 N」（防撞车双补，
    /// 任务30 二期③ 既有契约）。
    fn humanized_summary(
        &self,
        name: &str,
        args_summary: Option<&str>,
        content: &serde_json::Value,
        exit_code: Option<i64>,
        fallback: &str,
    ) -> (String, bool);

    /// 结果详情块（对标 renderToolResultMessage 可缺省；None=无详情）。
    fn result_detail(&self, name: &str, content: &serde_json::Value, is_error: bool) -> Option<DetailKind>;

    /// 失败原因人话摘要（对标 renderToolUseErrorMessage：错误摘短句，
    /// 非原始错误倾倒）；None=调用方不追加。
    fn error_summary(&self, content: &serde_json::Value) -> Option<String> {
        error_reason_plain(content)
    }

    /// spinner 进行时动词短语（对标 getActivityDescription，〈挖掘〉§5.1：
    /// 「Reading src/foo.ts」进行时形态）。默认=摘要原样（未知工具兜底，
    /// 零行为）；六内置各自人话（批 4 T1-b）。
    fn activity(&self, name: &str, args_summary: Option<&str>, fallback: &str) -> String {
        let _ = (name, args_summary);
        fallback.to_string()
    }
}

/// 失败原因人话（批 4 T1-b）：**message 首行优先**（消息自带上下文细节，
/// 短句映射会丢「哪个文件」类信息——adapter 既有测试锁契约）；canonical
/// 错误码映射仅在 message 缺失/空时兜底（稳定码→人话短句），全缺退 None。
fn error_reason_plain(content: &serde_json::Value) -> Option<String> {
    let first_line = |msg: &str| {
        Some(msg.lines().next().unwrap_or(msg).trim().to_string()).filter(|s| !s.is_empty())
    };
    if let Some(msg) = content.get("message").and_then(|v| v.as_str()) {
        return first_line(msg);
    }
    let mapped = match content.get("code").and_then(|v| v.as_str()) {
        Some("tool/no-match") => Some("没找到要替换的文本"),
        Some("tool/ambiguous") => Some("目标文本出现多次，拒绝替换"),
        Some("tool/path-outside-root") => Some("路径越出工作区"),
        Some("tool/not-found") => Some("文件不存在"),
        Some("tool/args") => Some("参数缺失"),
        _ => None,
    };
    if let Some(plain) = mapped {
        return Some(plain.to_string());
    }
    // render_error 文本形态（`[错误 {code}] {message}`）沿用 parse 同款
    let text = content.as_str()?;
    let first = text.lines().next()?.strip_prefix("[错误 ")?;
    let m = first.split_once("] ")?.1.trim();
    (!m.is_empty()).then(|| m.to_string())
}

/// 六内置共享底座（批 4 拆分：摘要/详情仍委托 humanize/parse 单源零漂移，
/// 每工具只自定义 activity 动词短语）。
macro_rules! builtin_renderer {
    ($name:ident, $activity:expr) => {
        struct $name;
        impl ToolRenderer for $name {
            fn humanized_summary(&self, name: &str, args_summary: Option<&str>, content: &serde_json::Value, exit_code: Option<i64>, fallback: &str) -> (String, bool) {
                crate::humanize::humanize(name, args_summary, content, exit_code, fallback)
            }
            fn result_detail(&self, name: &str, content: &serde_json::Value, is_error: bool) -> Option<DetailKind> {
                crate::adapter::parse_detail_for(name, content, is_error)
            }
            fn activity(&self, name: &str, args_summary: Option<&str>, fallback: &str) -> String {
                let _ = name;
                ($activity)(args_summary, fallback)
            }
        }
    };
}

/// activity 动词组装壳：字段值提取成功才包动词（humanize::extract 单源）；
/// 提取失败退 fallback 原文——宁原文不编造（humanize 同哲学）。
fn verb_of(verb: &str, field: &str, args_summary: Option<&str>, fallback: &str) -> String {
    match args_summary.and_then(|args| crate::humanize::extract(args, field)) {
        Some(subject) if !subject.is_empty() => format!("{verb} {subject}"),
        _ => fallback.to_string(),
    }
}

builtin_renderer!(BashRenderer, |args, fallback| verb_of("运行", "cmd", args, fallback));
builtin_renderer!(ReadRenderer, |args, fallback| verb_of("读取", "path", args, fallback));
builtin_renderer!(EditRenderer, |args, fallback| verb_of("修改", "path", args, fallback));
builtin_renderer!(WriteRenderer, |args, fallback| verb_of("写入", "path", args, fallback));
builtin_renderer!(GlobRenderer, |args, fallback| verb_of("搜索", "pattern", args, fallback));
builtin_renderer!(GrepRenderer, |args, fallback| verb_of("搜索", "pattern", args, fallback));

/// 未知工具兜底（对标 Tool.ts 未知工具通用兜底）：与内置同实现——现
/// humanize/parse 对未知名本就退键值摘要（Fallback 语义已内建），本批
/// 显式给 Fallback 注册位，T1-b 拆分时未知工具恒走它、不随内置漂移。
struct FallbackRenderer;

impl ToolRenderer for FallbackRenderer {
    fn humanized_summary(
        &self,
        name: &str,
        args_summary: Option<&str>,
        content: &serde_json::Value,
        exit_code: Option<i64>,
        fallback: &str,
    ) -> (String, bool) {
        crate::humanize::humanize(name, args_summary, content, exit_code, fallback)
    }

    fn result_detail(&self, name: &str, content: &serde_json::Value, is_error: bool) -> Option<DetailKind> {
        crate::adapter::parse_detail_for(name, content, is_error)
    }
}

/// 六内置工具名单（与 humanize 动词表/parse 分流表同域维护；新工具注册
/// 先在此挂名）。未知名一律 FallbackRenderer。
const BUILTIN_TOOLS: [&str; 6] = ["bash", "read", "edit", "write", "glob", "grep"];

/// 注册表查找（唯一分发点）：按工具名取渲染器。静态表零分配零锁。
/// （身份判定用 [`is_builtin`]——ZST 静态地址不保证唯一，禁 ptr::eq 判型。）
pub(crate) fn renderer(name: &str) -> &'static dyn ToolRenderer {
    match name {
        "bash" => &BashRenderer,
        "read" => &ReadRenderer,
        "edit" => &EditRenderer,
        "write" => &WriteRenderer,
        "glob" => &GlobRenderer,
        "grep" => &GrepRenderer,
        _ => &FallbackRenderer,
    }
}

/// 内置/兜底身份判定（测试与 T1-b 拆分守门用）。
pub(crate) fn is_builtin(name: &str) -> bool {
    BUILTIN_TOOLS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 注册表分发性质锁：六内置命中 Builtin；未知名（含 webfetch/未来插件）
    /// 命中 Fallback——两者现行为一致（委托同实现），但分发身份必须分明，
    /// T1-b 拆分后才不会把内置漂移带进兜底。
    #[test]
    fn 注册表_内置命中_未知走兜底() {
        for name in BUILTIN_TOOLS {
            assert!(is_builtin(name), "{name}");
        }
        assert!(!is_builtin("webfetch"));
        assert!(!is_builtin("未来插件"));
    }

    /// 兜底语义锁（零行为）：未知工具摘要退 fallback 原文（humanize 对未知名
    /// 直返 fallback）、activity 同退原文；错误摘要=人话映射+message 首行兜底。
    #[test]
    fn 兜底_未知工具摘要_错误原因() {
        let (summary, has_exit) =
            renderer("未知工具").humanized_summary("未知工具", Some("x=1"), &json!({}), None, "原文摘要");
        assert_eq!(summary, "原文摘要");
        assert!(!has_exit);
        assert_eq!(renderer("未知工具").activity("未知工具", Some("x=1"), "原文摘要"), "原文摘要", "未知工具 activity 兜底原样");
        assert_eq!(
            renderer("未知工具").error_summary(&json!({"code": "E1", "message": "文件没找到\n第二行"})),
            Some("文件没找到".to_string())
        );
        assert_eq!(renderer("未知工具").error_summary(&json!({"output": "ok"})), None);
    }

    /// 批 4 锁：activity 动词短语六内置人话（字段提取失败退原文 fallback）。
    #[test]
    fn activity_六内置动词短语() {
        assert_eq!(renderer("bash").activity("bash", Some("{\"cmd\":\"cargo test -p yunyou-cli\"}"), "原始"), "运行 cargo test -p yunyou-cli");
        assert_eq!(renderer("read").activity("read", Some("{\"path\":\"src/lib.rs\"}"), "原始"), "读取 src/lib.rs");
        assert_eq!(renderer("edit").activity("edit", Some("{\"path\":\"a.rs\"}"), "原始"), "修改 a.rs");
        assert_eq!(renderer("write").activity("write", Some("{\"path\":\"b.rs\"}"), "原始"), "写入 b.rs");
        assert_eq!(renderer("grep").activity("grep", Some("{\"pattern\":\"foo\"}"), "原始"), "搜索 foo");
        assert_eq!(renderer("glob").activity("glob", Some("{\"pattern\":\"*.rs\"}"), "原始"), "搜索 *.rs");
        // 提取失败（截断最坏位）退原文，不编造
        assert_eq!(renderer("bash").activity("bash", None, "原始摘要"), "原始摘要");
    }

    /// 批 4 锁：错误摘要=message 首行优先（细节不丢，adapter 契约同款）；
    /// message 缺失时错误码映射兜底。
    #[test]
    fn 错误码_人话映射() {
        assert_eq!(renderer("edit").error_summary(&json!({"code": "tool/no-match", "message": "old 文本在文件中出现 0 次"})), Some("old 文本在文件中出现 0 次".to_string()), "message 首行优先");
        assert_eq!(renderer("edit").error_summary(&json!({"code": "tool/no-match"})), Some("没找到要替换的文本".to_string()), "message 缺失→码映射兜底");
        assert_eq!(renderer("bash").error_summary(&json!({"code": "tool/io", "message": "读取失败：/x（No such file）\nhint"})), Some("读取失败：/x（No such file）".to_string()));
    }
}
