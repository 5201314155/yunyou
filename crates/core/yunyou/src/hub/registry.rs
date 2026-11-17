//! 成员表与信箱状态机（融合终版 §1.4：`Pending（信箱开、无 fiber）⇄ Online（绑定 fiber）`）。
//!
//! 职责=注册表：开箱/上线/降级/开箱回收/名册投影/身份反查/观测登记。所有方法
//! 在 hub 的单一 `Mutex<CoreInner>` 锁内调用（同步短临界区，锁内绝不 await——
//! 「锁外 spawn 观测通知」的纪律落点在 service.rs）。

use super::types::{MemberInfo, MessageHandler};
use crate::error::{CordisError, ErrorKind, Result};
use crate::events::EventOwner;
use std::collections::HashMap;
use std::sync::Arc;

/// 成员名校验（HubInvalidName，冻结错误表）：
/// ①空名/全空白=永不可达地址，注册时堵；
/// ②**字符集白名单 `[a-z0-9-_]`＋长度上限 64（kebab-case 惯例）**。
///
/// # 取舍翻转（v1 只钉空白 → 白名单，task-18）
/// v1 曾裁「除空白外不做字符集收紧——多堵=预造」。翻转依据=云垒实测票：
/// 成员名经 `teammate-{name}.jsonl` 拼装流向**会话文件路径**（team service
/// 的 spawn 链），`../`、绝对路径等穿越形态实测可越目录落盘——名字不再是
/// 纯内存键，而是路径分量。白名单字符集把 `/`、`.`、`\`、空白、大写整体
/// 排除在词法层，**穿越类按字符集根除而非逐形态打补丁**（`..`/`../`/绝对
/// 路径全数落不进合法字符集）。名字仍按原样入册不归一（trim 改名会制造
/// 「注册名≠寻址名」暗差异的既有取舍不变，但白名单下空白名已在①被拒）。
///
/// pub 导出理由：spawn_teammate 工具面（yunyou-team）须在 ToolOutcome 层
/// 给模型可操作的人话拒绝——同一规则单一源，两道闸共用本函数防漂移。
pub fn validate_member_name(name: &str) -> Result<String> {
    if name.trim().is_empty() {
        return Err(CordisError::new(
            ErrorKind::HubInvalidName,
            "成员名不得为空或全空白（空名=永不可达地址，注册时堵）",
        ));
    }
    let legal = name.len() <= MEMBER_NAME_MAX_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    // 保留广播地址（task-30 打回-轻①，云垒升级）：kebab 白名单现在就放行
    // "all"——注册成功后任何点发 to="all" 即广播全员=跨成员消息误投面；
    // schema description 是模型面软约束非强制闸，注册时一行收死。
    if legal && name == "all" {
        return Err(CordisError::new(
            ErrorKind::HubInvalidName,
            "成员名 \"all\" 为保留广播地址（send_message 的 to=all 广播在册全体），\
             取该名会与广播语义冲突，请换名",
        ));
    }
    if !legal {
        // 回显截断：拒绝消息进 tool_result，超长名不得撑爆消息面（截断不影响
        // 自纠——规则文案已说明合法形态）。
        let shown: String = name.chars().take(48).collect();
        let ellipsis = if name.chars().count() > 48 { "…" } else { "" };
        return Err(CordisError::new(
            ErrorKind::HubInvalidName,
            format!(
                "成员名非法：「{shown}{ellipsis}」——只允许小写字母/数字/中划线/下划线，\
                 长度上限 {MEMBER_NAME_MAX_LEN}（kebab-case 惯例，如 code-reviewer）；\
                 成员名会成为会话文件名的一部分，路径与空白字符一律不收"
            ),
        ));
    }
    Ok(name.to_string())
}

/// 成员名长度上限（kebab-case 惯例；官方同类标识量级内取整）。
pub const MEMBER_NAME_MAX_LEN: usize = 64;

/// 信箱条目：一个在册名字一份，承载状态机三件套（归属/未取净消息/观测订阅）。
pub(super) struct Mailbox {
    /// 在册成员名（键的冗余持有：投影与诊断免回查键）。
    pub name: String,
    /// Online 时的归属 fiber；None=Pending（状态机的「态」即此 Option）。
    pub fiber: Option<EventOwner>,
    /// 未取净消息：按全局 log_id 升序追加（受理都在同一锁内完成，追加序=全局发送序）。
    pub inbox: Vec<super::types::DeliveredMessage>,
    /// 观测订阅（推，尽力而为）：本成员的入站镜像回调；fiber 死亡随降级清空。
    pub observers: Vec<MessageHandler>,
}

impl Mailbox {
    /// 名册投影：online 标志+归属（MemberInfo.fiber 的直接来源）。
    fn info(&self) -> MemberInfo {
        MemberInfo {
            name: self.name.clone(),
            online: self.fiber.is_some(),
            fiber: self.fiber.clone(),
        }
    }
}

/// 成员表：名字 → 信箱（主索引）+ Online fiber id → 名字（身份反查）。
pub(super) struct Registry {
    /// 主索引（含 Pending 预注册条目——spawn 窗口的可见落点）。
    pub by_name: HashMap<String, Mailbox>,
    /// 身份反查：Online fiber id → 成员名（防伪造的唯一身份源，send/drain 用）。
    pub by_fiber: HashMap<u64, String>,
}

impl Registry {
    pub(super) fn new() -> Self {
        Self { by_name: HashMap::new(), by_fiber: HashMap::new() }
    }

    /// 预注册信箱（Pending 态）。重名（任何态）fail-loud：open_mailbox 的语义是
    /// 「占一个尚未上线的地址位」，重名即歧义，绝不静默改名。
    pub(super) fn open_mailbox(&mut self, name: &str) -> Result<()> {
        if self.by_name.contains_key(name) {
            return Err(CordisError::new(
                ErrorKind::HubDuplicateMember,
                format!("成员名 \"{name}\" 已在册（任何态重名均拒；spawn 失败请先 dispose 回收名字）"),
            ));
        }
        self.by_name.insert(
            name.to_string(),
            Mailbox { name: name.to_string(), fiber: None, inbox: Vec::new(), observers: Vec::new() },
        );
        Ok(())
    }

    /// 成员上线：Pending→Online（补投就绪）；无条目→直接 Online（自发成员，如
    /// lead）。两类重名绑定同报 HubDuplicateMember：目标名已 Online（本 fiber 或
    /// 他人）=路由欺骗绝不静默接管；本 fiber 已绑他名=一 fiber 一成员名（不堵则
    /// drain 寻址歧义——语义加严项，云枢 retro 项③留痕，lead 2026-11-15 批）。
    pub(super) fn register(&mut self, name: &str, owner: EventOwner) -> Result<()> {
        if let Some(bound) = self.by_fiber.get(&owner.fiber_id) {
            return Err(CordisError::new(
                ErrorKind::HubDuplicateMember,
                format!("当前 fiber 已绑定成员 \"{bound}\"：一 fiber 一成员名，防身份混淆"),
            ));
        }
        match self.by_name.get_mut(name) {
            Some(mailbox) => {
                if mailbox.fiber.is_some() {
                    return Err(CordisError::new(
                        ErrorKind::HubDuplicateMember,
                        format!("成员 \"{name}\" 已 Online：重名绑定=路由欺骗，拒绝接管"),
                    ));
                }
                mailbox.fiber = Some(owner.clone()); // Pending→Online，认领预注册信箱
            }
            None => {
                self.by_name.insert(
                    name.to_string(),
                    Mailbox {
                        name: name.to_string(),
                        fiber: Some(owner.clone()),
                        inbox: Vec::new(),
                        observers: Vec::new(),
                    },
                );
            }
        }
        self.by_fiber.insert(owner.fiber_id, name.to_string());
        Ok(())
    }

    /// fiber 死亡降级：Online→Pending——信箱与未取净消息保留（非删除，同名重注册
    /// 即续投）。只处理 fiber_id 匹配的条目：防「旧 disposer 迟到误降新绑定」
    /// （fiber id 永不复用，同 id 匹配即唯一归属证据）。
    pub(super) fn demote(&mut self, name: &str, fiber_id: u64) {
        let Some(mailbox) = self.by_name.get_mut(name) else { return };
        if !mailbox.fiber.as_ref().is_some_and(|owner| owner.fiber_id == fiber_id) {
            return;
        }
        mailbox.fiber = None;
        mailbox.observers.clear(); // 订阅 effect 同挂该 fiber，此处为兜底清空
        self.by_fiber.remove(&fiber_id);
    }

    /// 开箱回收（open_mailbox 的 disposer）：仅当条目仍 Pending（未被成员认领）
    /// 才摘除。已被认领（Online）的信箱归成员所有——开箱方（spawn 发起者）死亡
    /// 不得连带删除活成员的信箱。
    pub(super) fn reclaim_if_pending(&mut self, name: &str) {
        if self.by_name.get(name).is_some_and(|mailbox| mailbox.fiber.is_none()) {
            self.by_name.remove(name);
        }
    }

    /// 名册消息侧投影（按名字排序：HashMap 遍历序随机，排序保输出稳定）。
    pub(super) fn members(&self) -> Vec<MemberInfo> {
        let mut infos: Vec<MemberInfo> = self.by_name.values().map(Mailbox::info).collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    /// 身份反查：Online fiber → 成员名（send/drain 身份派发的唯一入口）。
    pub(super) fn identity_of(&self, fiber_id: u64) -> Option<String> {
        self.by_fiber.get(&fiber_id).cloned()
    }

    /// 登记观测订阅（调用方身份必须已绑定成员；名字由身份反查得出）。
    pub(super) fn attach_observer(&mut self, name: &str, handler: MessageHandler) -> Result<()> {
        let Some(mailbox) = self.by_name.get_mut(name) else {
            // 身份反查在册而信箱缺失=表内不一致，只能是内核缺陷（demote 同步摘双方）
            return Err(CordisError::new(
                ErrorKind::Internal,
                format!("成员 \"{name}\" 身份在册而信箱缺失：内核缺陷"),
            ));
        };
        mailbox.observers.push(handler);
        Ok(())
    }

    /// 摘除观测订阅（按指针相等；subscribe 的 effect disposer 专用）。
    pub(super) fn detach_observer(&mut self, name: &str, handler: &MessageHandler) {
        if let Some(mailbox) = self.by_name.get_mut(name) {
            mailbox.observers.retain(|h| !Arc::ptr_eq(h, handler));
        }
    }
}
