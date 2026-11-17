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
        crate::adapter::parse_error_reason(content)
    }
}

/// 内置工具统一实现（T1-a 过渡形态）：委托既有实现（humanize/parse），
/// 行为零变更——六内置与未知工具在旧路径本就同代码，T1-b 再拆各自结构体。
struct BuiltinRenderer;

impl ToolRenderer for BuiltinRenderer {
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
    if is_builtin(name) {
        &BuiltinRenderer
    } else {
        &FallbackRenderer
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
    /// 直返 fallback），错误摘要=parse 的 error_reason 同款。
    #[test]
    fn 兜底_未知工具摘要_错误原因() {
        let (summary, has_exit) =
            renderer("未知工具").humanized_summary("未知工具", Some("x=1"), &json!({}), None, "原文摘要");
        assert_eq!(summary, "原文摘要");
        assert!(!has_exit);
        assert_eq!(
            renderer("未知工具").error_summary(&json!({"code": "E1", "message": "文件没找到\n第二行"})),
            Some("文件没找到".to_string())
        );
        assert_eq!(renderer("未知工具").error_summary(&json!({"output": "ok"})), None);
    }
}
