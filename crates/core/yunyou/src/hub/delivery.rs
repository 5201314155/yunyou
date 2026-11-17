//! 投递语义（融合终版 §1.4）：寻址受理/离线缓冲/越权拦截/回执与留痕同点落账。
//!
//! 全部方法在 hub 单锁内调用；锁外动作（watch emit、观测镜像 spawn）由 service.rs
//! 在锁释放后执行——「锁内快照、锁外 spawn」与 events.rs emit 同款纪律（写锁绝不
//! 跨 await，锁内不 spawn）。

use super::registry::Registry;
use super::service::CoreInner;
use super::types::{
    Address, DeliveredMessage, DeliveryStatus, HistoryEntry, MessageHandler, OutgoingMessage,
    SentReceipt,
};
use crate::error::{CordisError, ErrorKind, Result};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// 一次受理的锁内产出，锁外消费（观测通知的快照——锁绝不跨 await）。
pub(super) struct Acceptance {
    /// 投递回执（真账）。
    pub receipt: SentReceipt,
    /// 观测镜像待派发：(目标成员的订阅回调集, 消息)——只有 Online 目标才有。
    pub mirrors: Vec<(Vec<MessageHandler>, DeliveredMessage)>,
    /// watch payload 六位（`message.v1/watch`，尽力而为）。
    pub watch: Vec<Value>,
}

/// unix 毫秒（留痕 at_ms；留痕是观测面，排序键=log_id，时钟只作展示参考）。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 寻址展开：目标名清单。All=除 from 外的在册全体（含 Pending）；Member=单名
/// （存在性在 send 里判——展开只管「投给谁」，不管「投不投得到」）。
fn resolve_targets(registry: &Registry, from: &str, to: &Address) -> Vec<String> {
    match to {
        Address::All => {
            let mut names: Vec<String> =
                registry.by_name.keys().filter(|n| n.as_str() != from).cloned().collect();
            names.sort(); // outcomes 与广播受理序稳定（名字序；投递语义不受其影响）
            names
        }
        Address::Member(name) => vec![name.clone()],
    }
}

impl CoreInner {
    /// 寻址投递（身份派发定案）：from 由调用 fiber 反查得出，签名层面无 from 参数
    /// ——模型自报与工具实现伪造都无从下手，hub 是唯一身份源。
    ///
    /// - 调用 fiber 未绑定成员 → HubUnknownMember（无身份者发不出——防伪造机制化）；
    /// - Member 目标无任何条目 → HubUnknownMember（typo 不静默吞）；
    /// - Online=入信箱+观测镜像（Delivered）；Pending=入信箱缓冲（Buffered，补投就绪）；
    /// - All=除 from 外在册全体；零成员 fail-loud（task-30① lead 预裁翻转
    ///   原「空集照样 Ok」口径）；
    /// - 留痕与受理同点落账（outcomes 即真账）；Ok 当且仅当全部目标受理。
    pub(super) fn send(
        &mut self,
        from_fiber: u64,
        to: Address,
        message: OutgoingMessage,
    ) -> Result<Acceptance> {
        let from = self.identity_or_fail(from_fiber, "send")?;
        if let Address::Member(name) = &to {
            if !self.registry.by_name.contains_key(name) {
                return Err(CordisError::new(
                    ErrorKind::HubUnknownMember,
                    format!(
                        "收件名 \"{name}\" 未在册（未预注册且未上线）：typo 不静默吞，在册名单见 members()"
                    ),
                ));
            }
        }
        let targets = resolve_targets(&self.registry, &from, &to);
        // 零成员广播 fail-loud（task-30①，lead 预裁）：除 from 外零在册=广播无
        // 收件人，静默 Ok(recipients=0) 是「发出去了没人收到」的假成功——原
        // 「空集照样 Ok」口径（测试矩阵⑩）由本裁翻转，用例同步改断言。
        if matches!(to, Address::All) && targets.is_empty() {
            return Err(CordisError::new(
                ErrorKind::HubUnknownMember,
                "广播被拒：除发送者外无在册成员（All 需要≥1 个收件人；先 spawn 成员或在册名单见 members()）",
            ));
        }
        let log_id = self.next_log;
        self.next_log += 1;
        let mut outcomes: Vec<(String, DeliveryStatus)> = Vec::with_capacity(targets.len());
        let mut mirrors: Vec<(Vec<MessageHandler>, DeliveredMessage)> = Vec::new();
        for name in &targets {
            let Some(mailbox) = self.registry.by_name.get_mut(name) else { continue };
            let delivered = DeliveredMessage {
                log_id,
                from: from.clone(),
                kind: message.kind,
                body: message.body.clone(),
            };
            // Online 即时入箱+镜像；Pending 只入箱（缓冲）——两者信箱同构，差别
            // 只在「是否立即可 drain」：Pending 无 fiber，drain 必等重注册。
            let status = if mailbox.fiber.is_some() {
                mirrors.push((mailbox.observers.clone(), delivered.clone()));
                DeliveryStatus::Delivered
            } else {
                DeliveryStatus::Buffered
            };
            mailbox.inbox.push(delivered);
            outcomes.push((name.clone(), status));
        }
        let buffered = outcomes.iter().filter(|(_, s)| *s == DeliveryStatus::Buffered).count();
        let to_label = match &to {
            Address::All => "All".to_string(),
            Address::Member(name) => name.clone(),
        };
        // 观测状态位：含任一缓冲目标即标 buffered（观测面只答「有没有落进 spawn 窗口」）。
        let watch_status =
            if buffered > 0 { DeliveryStatus::Buffered } else { DeliveryStatus::Delivered };
        let watch = vec![
            json!(log_id),
            json!(from),
            json!(to_label),
            json!(message.kind.as_str()),
            json!(message.body),
            json!(watch_status.as_str()),
        ];
        let entry = HistoryEntry {
            log_id,
            at_ms: now_ms(),
            from,
            to: to_label,
            kind: message.kind,
            body: message.body,
            outcomes: outcomes.clone(),
        };
        self.ring.push(entry);
        Ok(Acceptance {
            receipt: SentReceipt { log_id, recipients: outcomes.len(), buffered },
            mirrors,
            watch,
        })
    }

    /// 消费通道（拉）：取净当前 fiber 绑定成员的信箱（追加序=全局 log_id 升序）。
    /// 非成员 fiber → HubUnknownMember（防成员 A 取净 B 的信箱——越权读对偶防线）。
    /// 「取净」=一次性清空：消费位由宿主每轮请求前调用（drain 是宿主机制非模型工具）。
    pub(super) fn drain(&mut self, from_fiber: u64) -> Result<Vec<DeliveredMessage>> {
        let name = self.identity_or_fail(from_fiber, "drain")?;
        let Some(mailbox) = self.registry.by_name.get_mut(&name) else {
            return Err(CordisError::new(
                ErrorKind::Internal,
                format!("成员 \"{name}\" 身份在册而信箱缺失：内核缺陷"),
            ));
        };
        Ok(std::mem::take(&mut mailbox.inbox))
    }

    /// 身份派发唯一入口：fiber id → 成员名；未绑定=HubUnknownMember（注册类与
    /// 消费类的无运载分型在 service.rs 的 require_* 层，此处是「有 fiber 无绑定」）。
    fn identity_or_fail(&self, fiber_id: u64, action: &str) -> Result<String> {
        self.registry.identity_of(fiber_id).ok_or_else(|| {
            CordisError::new(
                ErrorKind::HubUnknownMember,
                format!("当前 fiber 未绑定成员，{action} 被拒：无身份者发不出、取不到他人信箱（防伪造）"),
            )
        })
    }
}
