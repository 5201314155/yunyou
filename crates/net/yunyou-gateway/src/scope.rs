//! App 会话 scope 边界（网关落地方案 §7.2；业主硬约束③：暴露面 ⊆ 令牌 scope，
//! 两道闸串联——中间件先验 scope 再选 IPC 消息）。

/// 三 scope（方案 §7.2 冻结集）：chat=对话全能力；panel:read=任务只读投影；
/// panel:write=命令入队（消费权仍在编排层）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 对话能力（session.open/chat.send/chat.events）。
    Chat,
    /// 面板只读（panel.tasks.read）。
    PanelRead,
    /// 面板写（panel.command.submit，只入队）。
    PanelWrite,
}

impl Scope {
    /// 串行化形态（tokens.scope_csv 的逗号分隔词汇，§3.1 存储契约）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::PanelRead => "panel:read",
            Self::PanelWrite => "panel:write",
        }
    }

    /// 从逗号分隔串解析（未知项跳过=加法演进兼容；全未知=空集=无能力令牌）。
    pub fn parse_csv(csv: &str) -> Vec<Self> {
        csv.split(',').filter_map(|item| match item.trim() {
            "chat" => Some(Self::Chat),
            "panel:read" => Some(Self::PanelRead),
            "panel:write" => Some(Self::PanelWrite),
            _ => None,
        }).collect()
    }

    /// scope 检查：令牌 scope 集是否覆盖所需能力（中间件两道闸的第一道）。
    pub fn covers(scopes: &[Self], needed: Self) -> bool {
        scopes.contains(&needed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// csv 解析与未知项跳过（加法演进兼容：未来 scope 不炸旧解析）。
    #[test]
    fn scope解析与未知项跳过() {
        let scopes = Scope::parse_csv("chat,panel:write");
        assert_eq!(scopes, vec![Scope::Chat, Scope::PanelWrite]);
        assert!(Scope::parse_csv("chat,future:thing").contains(&Scope::Chat));
        assert!(Scope::parse_csv("future:thing").is_empty(), "全未知=空集");
    }

    /// 覆盖检查（两道闸第一道的语义锁）。
    #[test]
    fn scope覆盖检查() {
        let scopes = Scope::parse_csv("chat");
        assert!(Scope::covers(&scopes, Scope::Chat));
        assert!(!Scope::covers(&scopes, Scope::PanelRead), "chat 令牌不覆盖 panel:read");
        assert!(!Scope::covers(&scopes, Scope::PanelWrite));
    }
}
