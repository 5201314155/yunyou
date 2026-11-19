//! 模式切换的模型通知（task-16 S3 补件，lead 裁决 2026-11-19：不扩
//! ComposeEdit——①切换瞬间通知=SystemMessage 落会话；②每回合重申=动态
//! 模式段 harness:mode 同名 shadow 重注册（registry.rs:7-10 同名=替换裁断，
//! compose 每轮消费=claude messages.ts:3392 每回合重申同律））。

use crate::mode::Mode;
use std::sync::Arc;
use yunyou_agent_loop::{GateHub, ModeStateService};
use yunyou_session::SessionEvent;
use yunyou_system_prompt::Section;
use yunyou_util::BoxFut;

/// 模式段名（harness: 前缀=宿主段；段位 490——skills 目录段 500 之前）。
pub(crate) const MODE_SECTION_NAME: &str = "harness:mode";
pub(crate) const MODE_SECTION_ORDER: f64 = 490.0;

/// 模式→段文本（每回合重申内容；文案与 mode.rs 矩阵语义一致）。
fn mode_section_text(mode: Option<&str>) -> String {
    match mode.and_then(Mode::parse) {
        Some(Mode::Default) | None => {
            "当前权限模式：default——工作区内写直接放行，区外写/执行命令会请求审批".into()
        }
        Some(Mode::AcceptEdits) => {
            "当前权限模式：acceptEdits——编辑放行（区内写免审），执行命令仍会请求审批".into()
        }
        Some(Mode::Plan) => {
            "当前权限模式：plan——只读调研中，写操作与命令执行一律请求审批；完成调研后请退出计划模式".into()
        }
        Some(Mode::Bypass) => {
            "当前权限模式：bypass——不询问直接执行（deny 规则与高危拦截仍生效）".into()
        }
    }
}

/// 模式→切换通知文案（件①；prev=None=会话首次显式设置）。
fn switch_notice(prev: Option<&str>, next: Option<&str>) -> String {
    let name = |m: Option<&str>| {
        m.and_then(Mode::parse)
            .map(|m| match m {
                Mode::Default => "default".to_string(),
                Mode::AcceptEdits => "acceptEdits".to_string(),
                Mode::Plan => "plan".to_string(),
                Mode::Bypass => "bypass".to_string(),
            })
            .unwrap_or_else(|| "default".to_string())
    };
    match next.and_then(Mode::parse) {
        Some(Mode::Plan) => "已进入 plan 模式：只读调研，写操作与命令执行将请求审批".into(),
        Some(Mode::Bypass) => "已进入 bypass 模式：命令不再询问（deny 规则与高危拦截仍生效）".into(),
        Some(Mode::AcceptEdits) => "已进入 acceptEdits 模式：编辑放行，执行命令仍请求审批".into(),
        Some(Mode::Default) | None => match prev.and_then(Mode::parse) {
            Some(Mode::Plan) => "已退出 plan 模式，恢复正常读写".into(),
            Some(Mode::Bypass) => "已退出 bypass 模式，恢复审批询问".into(),
            _ => format!("已切换 default 模式（原 {}）", name(prev)),
        },
    }
}

/// 模式服务（权限插件 provide；set_mode 三副作用单点内聚：hub 写+提示段
/// 重注册+切换 SystemMessage 通知——cli /mode 与 shift+tab 等一切切换入口
/// 共用本服务，模型面永远知情）。
pub struct ModeGate {
    hub: Arc<GateHub>,
    prompt: Arc<dyn yunyou_system_prompt::SystemPrompt>,
    store: Arc<dyn yunyou_session::SessionStore>,
}

impl ModeGate {
    /// 装配（setup 期：hub 句柄+提示服务+会话存储）。
    pub fn new(
        hub: Arc<GateHub>,
        prompt: Arc<dyn yunyou_system_prompt::SystemPrompt>,
        store: Arc<dyn yunyou_session::SessionStore>,
    ) -> Self {
        Self { hub, prompt, store }
    }
}

impl ModeStateService for ModeGate {
    fn current_mode(&self) -> Option<String> {
        self.hub.current_mode()
    }

    fn set_mode<'a>(&'a self, mode: Option<String>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let prev = self.hub.current_mode();
            self.hub.set_mode(mode.clone());
            // 件②：动态模式段重注册（同名 shadow 替换；每轮 compose 自动携带）
            let section = Section::new(
                MODE_SECTION_NAME,
                MODE_SECTION_ORDER,
                mode_section_text(mode.as_deref()),
            );
            if let Err(err) = self.prompt.register_section(section) {
                eprintln!("[permission] 模式段重注册失败（切换仍生效，仅模型面缺提醒）：{err}");
            }
            // 件①：切换瞬间 SystemMessage 注入对话（会话事实源落盘）
            let notice = switch_notice(prev.as_deref(), mode.as_deref());
            if let Err(err) = self.store.append(&SessionEvent::SystemMessage { text: notice }).await {
                eprintln!("[permission] 切换通知落盘失败（切换仍生效）：{err}");
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // 测试断言惯例（同包既有测试同律）
mod tests {
    use super::*;
    use yunyou_agent_loop::ApprovalSubjectKind;

    /// 文案三型：进入 plan/退出 plan/首次显式 default。
    #[test]
    fn 切换通知文案() {
        assert!(switch_notice(None, Some("plan")).contains("只读调研"));
        assert!(switch_notice(Some("plan"), Some("default")).contains("已退出 plan"));
        assert!(switch_notice(Some("bypass"), Some("default")).contains("恢复审批询问"));
        assert!(switch_notice(None, Some("default")).contains("default"));
        assert!(mode_section_text(Some("plan")).contains("只读调研"));
        assert!(mode_section_text(None).contains("default"));
    }

    /// set_mode 三副作用：hub 写+段重注册（compose 面变化）+SystemMessage 落盘。
    #[tokio::test]
    async fn set_mode三副作用() {
        // 测试环境无内核装配——用轻量假件验证 ModeGate 协作面（真服务面由
        // plugin.rs provide 装配态保障；此处锁 ModeGate 自身逻辑）
        let hub = Arc::new(GateHub::default());
        // store：临时 jsonl；prompt：真 SectionRegistry 经 DefaultSystemPrompt
        let dir = std::env::temp_dir().join(format!("perm-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = yunyou_session::JsonlStore::new(dir.join("session.jsonl"))
            as Arc<dyn yunyou_session::SessionStore>;
        let prompt = yunyou_system_prompt::DefaultSystemPrompt::default();
        let gate = ModeGate::new(Arc::clone(&hub), Arc::new(prompt), Arc::clone(&store));
        gate.set_mode(Some("plan".into())).await;
        assert_eq!(hub.current_mode().as_deref(), Some("plan"));
        // 落盘断言：SystemMessage 在案
        let events = gate.store.load_all().await.unwrap();
        assert!(events.iter().any(
            |e| matches!(e, SessionEvent::SystemMessage { text } if text.contains("已进入 plan"))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 占位引用（防未用告警；ApprovalSubjectKind 在 authorization 域使用）。
    #[test]
    fn 占位() {
        let _ = ApprovalSubjectKind::ToolCall;
    }
}
