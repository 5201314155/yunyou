//! 契约 message.v1 数据形态（融合终版 §1.3 冻结原样，本文件不做任何字段增删）。
//!
//! 全类型 serde derive=二期 IPC schema 种子；`serde(default)`/`skip_serializing_if`
//! 兼容纪律（云伍 §5 吸收）只落在「可缺省而不失语义」的字段上。词汇单源=任务26
//! 台账原文；worker/租约/SQLite 等蓝图二期词不进本面。

use crate::error::Result;
use crate::events::EventOwner;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use yunyou_util::BoxFut;

/// 观测键（冻结面 #9）：每次受理后向进程总线 emit（五分发，尽力而为）。
///
/// payload 六位序冻结：`[log_id, from, to, kind, body, 状态]`。状态位取
/// `"delivered"`/`"buffered"`：一次受理含任一缓冲目标即标 buffered——观测面只需
/// 回答「这条有没有落进 spawn 窗口」，逐人明细看留痕 outcomes（真账）。
pub const WATCH_KEY: &str = "message.v1/watch";

/// 寻址目标（冻结面 #3）：`All`=在册全体（含 Pending，缓冲）且不含 from 自己。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Address {
    /// 在册全体广播。
    All,
    /// 点对点：按成员名寻址。
    Member(String),
}

/// 消息类别（冻结面 #4，三值冻结）：区分成员说话/系统通知/完工汇报，三值防
/// v2 churn（lead 合并令 c）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    /// 成员间正文。
    Text,
    /// 系统通知。
    System,
    /// 完工自动汇报（机制语义：成员 run_turn 终结时由成员 fiber 自身发送，
    /// 谁终结谁发送——别处代发会被身份派发拦截）。
    Report,
}

impl MessageKind {
    /// 观测 payload 与诊断用的类别名（与 serde repr 同串）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::System => "system",
            Self::Report => "report",
        }
    }
}

/// 出站面（冻结面 #5）：**无 from 字段**——发送者身份由 hub 按调用 fiber 派发
/// （防伪造定案），模型自报与插件伪造在此面无从下手。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// 消息类别。
    pub kind: MessageKind,
    /// 正文文本（对齐 send_message 工具面）。
    pub body: String,
}

/// 名册消息侧投影（冻结面 #2）：Pending 无归属（fiber=None）=spawn 窗口可见性。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberInfo {
    /// 在册成员名。
    pub name: String,
    /// 是否 Online（绑定存活 fiber）。
    pub online: bool,
    /// Online 时的归属 fiber；Pending=None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fiber: Option<EventOwner>,
}

/// 收件面（冻结面 #6）：不暴露 to=最小知识面（收到的即发给自己的）；
/// log_id 全局单调=消费方排序键（收件方按 log_id 排序自理）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredMessage {
    /// 全局投递序号（单调递增）。
    pub log_id: u64,
    /// 发送者成员名（hub 派发，非自报）。
    pub from: String,
    /// 消息类别。
    pub kind: MessageKind,
    /// 正文。
    pub body: String,
}

/// 观测订阅回调（推，尽力而为）：handler 失败记日志不上抛——消费保证在 drain。
pub type MessageHandler =
    Arc<dyn Fn(&DeliveredMessage) -> BoxFut<'static, Result<()>> + Send + Sync>;

/// 投递回执（真账）：Ok 当且仅当全部目标受理入箱/入缓冲（不假报成功）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentReceipt {
    /// 本条消息的全局 log_id。
    pub log_id: u64,
    /// 受理目标总数（Online+Pending）。
    pub recipients: usize,
    /// 其中缓冲（Pending，等待补投）数。
    pub buffered: usize,
}

/// 留痕查询（冻结面 #7）：取最近 limit 条（log_id 升序返回；limit=0 得空）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryQuery {
    /// 返回条数上限。
    #[serde(default)]
    pub limit: usize,
}

/// 送达状态（冻结面 #7 真账）：区分即时送达与缓冲补投——补投可调试性的对账面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryStatus {
    /// 目标 Online，即时入箱待取。
    Delivered,
    /// 目标 Pending，缓冲等待重注册后首次 drain 取净。
    Buffered,
}

impl DeliveryStatus {
    /// 观测 payload 状态位字符串（与 serde repr 同串）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Buffered => "buffered",
        }
    }
}

/// 留痕条目（冻结面 #7）：outcomes 即真账；to=`"All"` 或成员名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// 全局投递序号（与收件面/回执同键）。
    pub log_id: u64,
    /// 受理时刻（unix 毫秒；留痕为观测面，排序键=log_id 非时钟）。
    pub at_ms: u64,
    /// 发送者成员名。
    pub from: String,
    /// 寻址目标：`"All"` 或成员名。
    pub to: String,
    /// 消息类别。
    pub kind: MessageKind,
    /// 正文。
    pub body: String,
    /// 逐目标受理结果（成员名, 送达状态）。
    #[serde(default)]
    pub outcomes: Vec<(String, DeliveryStatus)>,
}
