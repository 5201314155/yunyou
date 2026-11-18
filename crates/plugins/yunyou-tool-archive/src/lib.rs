//! 工具结果剥离策略插件（task-11②，lead 三点裁 2026-11-18）：组装前钩子缝
//! （yunyou-agent-loop::hooks）的第一个策略卡——主板开缝、策略是卡，机制零硬编码。
//!
//! # 语义（简报三点裁决落定）
//! 触发时机：每轮请求组装后、发送前（PreComposeHook）；候选=已消费（其后有
//! assistant 回应）且出了新鲜窗（最近 [`ToolArchiveConfig::fresh_window`] 条不剥）
//! 的 role=Tool 消息；**每轮最多一刀、最旧优先**（长会话逐轮瘦身，单轮请求体
//! 变化可预期）；原文落 side-store `tool_results/<call_id>.txt`（首次写、幂等
//! 不覆写），模型要重读就再调一次工具（重读可回）。
//!
//! # 缓存纪律三条
//! ①每轮最多一刀最旧优先（edits 长度恒 ≤1）；②stub 文本稳定（同 call_id 恒同
//! 文本，幂等判据=存档标记前缀）；③严禁轮中改写（钩子只在轮边界触发——hooks
//! 缝设计锁：retry 不重组装不重触发；side-store 幂等不覆写）。
//!
//! # store_dir 来源（裁「装配投影」方案）
//! 缺省从会话存储推导（SessionStore::path 的父目录），config `store_dir` 显式
//! 覆盖；两者皆无（纯 SDK/外部环境无投影）=**禁用+warn，诚实不猜路径**。

mod plugin;

pub use plugin::ToolArchivePlugin;

use yunyou::Result;
use yunyou_agent_loop::{ComposeEdit, PreComposeHook, PreComposeRequest};
use yunyou_util::BoxFut;
use std::path::{Path, PathBuf};

/// 存档 stub 前缀（幂等判据：以此开头的结果视为已存档，不再二次处理）。
pub(crate) const STUB_PREFIX: &str = "[工具结果已存档至 ";

/// 合成 stub 文本（同 call_id 恒同文本=稳定；带原字符数供模型判断是否重读）。
pub(crate) fn stub_text(call_id: &str, original_chars: usize) -> String {
    format!(
        "{STUB_PREFIX}tool_results/{call_id}.txt（原 {original_chars} 字）。如需重读请重新调用该工具]"
    )
}

/// side-store 落盘（幂等）：首次写原文，已存在跳过（防轮中改写/竞态覆写）。
/// 失败上抛由调用方弃刀（不阻塞轮次）。
pub(crate) fn archive_to_disk(store_dir: &Path, call_id: &str, original: &str) -> std::io::Result<()> {
    let dir = store_dir.join("tool_results");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{call_id}.txt"));
    if target.exists() {
        return Ok(()); // 幂等：已存档不重写
    }
    std::fs::write(target, original)
}

/// 剥离策略钩子（工具结果 → 存档 stub）。
pub(crate) struct ToolArchiveHook {
    /// side-store 根目录（config 显式或会话目录投影）。
    store_dir: PathBuf,
    /// 新鲜窗：最近 N 条 tool 结果不剥（模型很可能马上引用刚拿到的结果）。
    fresh_window: usize,
    /// 跨轮已剥记忆（task-11② 自查发现的关键态）：stub 只存在于请求投影
    /// （会话事实源永不改写），下一轮重新组装时钩子看到的又是原文——没有
    /// 本记忆则「最旧优先」每轮幂等重剥同一刀，后续候选永无机会。进程重启
    /// 后记忆清空=重新剥最旧者（存档幂等，无害）。跨实例撞同 call_id 亦无害
    /// （side-store 幂等+stub 文本同源）。
    cut_done: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl ToolArchiveHook {
    /// 组装期解析的最终配置（store_dir 已定、fresh_window 已裁）。
    pub(crate) fn new(store_dir: PathBuf, fresh_window: usize) -> Self {
        Self { store_dir, fresh_window, cut_done: Default::default() }
    }
}

/// 单条 tool 结果候选（扫描中间态）。
struct ToolResultCandidate {
    message_index: usize,
    call_id: String,
    content: String,
}

impl PreComposeHook for ToolArchiveHook {
    fn name(&self) -> &str {
        "tool-archive"
    }

    /// 剥离是较后策略（默认钩子 order=100 之后）。
    fn order(&self) -> u32 {
        200
    }

    fn on_pre_compose(
        &self,
        request: &PreComposeRequest<'_>,
    ) -> BoxFut<'_, Result<Vec<ComposeEdit>>> {
        // 借用数据在装箱前全部取出（hooks 测试教训：async move 块零捕获，
        // 双入参生命周期归一歧义即消失）；同步 IO 在 poll 内完成（小文件、
        // 幂等，超 200ms 由 hooks 缝超时兜底=弃刀不阻塞轮次）
        let candidates: Vec<String> = request.messages.iter().filter(|m| m.tool_call_id.is_some()).map(|m| format!("{}|{}", m.tool_call_id.clone().unwrap_or_default(), m.content.starts_with(STUB_PREFIX))).collect();
        let cut = self.pick_cut(request.messages);
        let cut = cut.filter(|candidate| {
            // 双保险：pick_cut 已排除记忆集；此处再核（多钩子场景下防御）
            !self.cut_done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&candidate.call_id)
        });
        let edit = cut.and_then(|candidate| {
            let original_chars = candidate.content.chars().count();
            match archive_to_disk(&self.store_dir, &candidate.call_id, &candidate.content) {
                Ok(()) => {
                    self.cut_done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(candidate.call_id.clone());
                    Some(ComposeEdit::StubToolResult {
                        message_index: candidate.message_index,
                        stub_text: stub_text(&candidate.call_id, original_chars),
                    })
                }
                Err(err) => {
                    tracing::warn!(
                        "[tool-archive] 存档落盘失败（{:?}），本轮弃刀不阻塞",
                        err
                    );
                    None
                }
            }
        });
        Box::pin(async move { Ok(edit.into_iter().collect()) })
    }
}

impl ToolArchiveHook {
    /// 触发时机判定（简报 §二规则，纯函数可测）：返回本轮唯一一刀的候选；
    /// None=本轮无刀（无候选/全在新鲜窗/未消费/已存档/落盘判据不满足前置）。
    fn pick_cut(&self, messages: &[yunyou_llm::Message]) -> Option<ToolResultCandidate> {
        // ①候选收集：role=Tool 且非 stub 形态（stub 幂等）
        let mut candidates: Vec<ToolResultCandidate> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.tool_call_id.is_some())
            .filter(|(_, m)| !m.content.starts_with(STUB_PREFIX))
            .map(|(i, m)| ToolResultCandidate {
                message_index: i,
                call_id: m.tool_call_id.clone().unwrap_or_default(),
                content: m.content.clone(),
            })
            .collect();
        // ②新鲜窗：最近 N 条不剥
        if self.fresh_window > 0 {
            let keep = candidates.len().saturating_sub(self.fresh_window);
            candidates.truncate(keep);
        }
        if candidates.is_empty() {
            return None;
        }
        // ③已消费判定：该结果之后存在 assistant 回应（模型已见过并作答）
        let last_assistant = messages
            .iter()
            .rposition(|m| m.tool_calls.is_empty() && m.tool_call_id.is_none());
        candidates.retain(|c| {
            last_assistant.is_some_and(|pos| pos > c.message_index)
        });
        // ④跨轮已剥记忆排除（stub 不落事实源，靠本态防同一刀重复选中）+
        // 每轮最多一刀、最旧优先
        let cut_done = self.cut_done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        candidates.retain(|c| !cut_done.contains(&c.call_id));
        candidates.into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use yunyou_llm::{Message, Role};

    fn msg_tool(call_id: &str, content: &str) -> Message {
        Message::tool_result(call_id, content)
    }

    /// 剥离三前提缺一不可：新鲜窗保护/未消费不剥/已存档幂等。
    #[test]
    fn 触发判定三前提() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let hook = ToolArchiveHook::new(PathBuf::from("/tmp/ta-test"), 2);
        let messages = vec![
            msg_tool("c1", "旧结果"),
            Message::text(Role::Assistant, "看到了"),
            msg_tool("c2", "较新结果"),
            Message::text(Role::Assistant, "又看到了"),
            msg_tool("c3", "最新结果"),
        ];
        // 新鲜窗=2：c3 不入候选；c2 已消费+出窗 → 应剥 c2（最旧出窗者）……
        // 但 c1 更旧且已消费——最旧优先应剥 c1
        let cut = hook.pick_cut(&messages).ok_or("应有候选")?;
        assert_eq!(cut.call_id, "c1", "最旧优先");
        // 未消费：assistant 回应缺席（最后一条就是 tool 结果本身）
        let unconsumed = vec![
            msg_tool("c1", "结果"),
            Message::text(Role::Assistant, "回应"),
            msg_tool("c2", "刚拿到的"),
        ];
        assert!(hook.pick_cut(&unconsumed).is_none(), "无后续回应的候选不剥");
        // 已存档幂等：c1 是 stub 形态则跳过（否则最旧优先会再剥它）
        let archived = vec![
            msg_tool("c1", &stub_text("c1", 3)),
            Message::text(Role::Assistant, "回应"),
            msg_tool("c2", "次旧"),
            Message::text(Role::Assistant, "回应2"),
            msg_tool("c3", "较新"),
            Message::text(Role::Assistant, "回应3"),
            msg_tool("c4", "最新"),
        ];
        let cut = hook.pick_cut(&archived).ok_or("c2 应成候选")?;
        assert_eq!(cut.call_id, "c2", "stub 形态不二次处理；窗内 c3/c4 不剥");
        Ok(())
    }

    /// side-store 幂等：首次写入原文；二次调用不覆写（改内容也不生效）。
    #[test]
    fn 存档幂等不覆写() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("ya-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        archive_to_disk(&dir, "call-x", "原文甲")?;
        archive_to_disk(&dir, "call-x", "原文乙")?;
        let saved = std::fs::read_to_string(dir.join("tool_results/call-x.txt"))?;
        assert_eq!(saved, "原文甲", "已存在不覆写");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// stub 文本稳定：同 call_id 恒同文本（逐字节可复现）+ 指路 + 原体积。
    #[test]
    fn stub文本稳定() {
        let a = stub_text("call-1", 120);
        let b = stub_text("call-1", 120);
        assert_eq!(a, b, "同 call_id 恒同文本");
        assert!(a.starts_with(STUB_PREFIX));
        assert!(a.contains("tool_results/call-1.txt"));
        assert!(a.contains("120"));
        assert!(a.contains("重新调用"));
    }

    /// Arc<dyn PreComposeHook> 形态可用（注册面契约冒烟）。
    #[test]
    fn 钩子可注册形态() {
        let hook: Arc<dyn PreComposeHook> = Arc::new(ToolArchiveHook::new(PathBuf::from("/tmp"), 2));
        assert_eq!(hook.name(), "tool-archive");
        assert_eq!(hook.order(), 200, "剥离是较后策略");
    }
}
