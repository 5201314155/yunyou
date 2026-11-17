# MessageHub 契约设计（任务26 T1·融合终版 v1.1）

> 作者：云缀（任务26 T1 设计席，只读+设计零实现零 commit）｜评审：lead（已批基座 2026-11-14 13:5x）→ 融合令（lead 14:3x）→ 本文终版｜**T1 实现批的最终依据**
>
> **【云枢 retro-review 适用】**——新契约面（message.v1）。云枢席重建后按 §4 冻结面逐字段过目；冻结点=lead 融合令+本终版落盘（2026-11-14）。
>
> **传承线**：v1.0（云缀，199 行）lead 已批为基座（五方法骨架/EffectHandle 生命周期/outcomes 真账/内核 hub.rs root provide/案乙论证）→ lead 合并令：云伍版四项语义（a 防伪造/b 拉模式+离线补投/c 三值/d 环形 1000）并入，冲突取舍逐条标注（§3）→ 本文=融合终版。云伍原文《团队插件MessageHub契约设计.md》作附录素材（其 T2/T3 接口位与验收清单已吸收进 §5/§6）。
>
> 输入：业主蓝图《小Agent团队架构.md》（补注四主板论）；《团队运行时落地方案.md》（任务5，已过审）；《插件化改造方案.md》（契约 v1 冻结范式 §2.5/§8 D7）；`crates/yunyou/src/events/`（行号 2026-11-14 实测）；任务26 四裁（台账 13:0x）；云伍设计原文。
> 词汇单源声明：v1 词汇以任务26 台账原文为单源（teammate/lead/member/名册/任务板/消息池/发布订阅+寻址+留痕）；蓝图二期词汇（worker/租约/SQLite/launcher）不进 v1 契约面，仅 §6 出现并标注「二期词汇」。

## 0. 结论速览

1. **架构基座（lead 已批，维持）**：投递本体=内核内平行新表（成员表+收件表+留痕缓冲），机制四处全复用（effect 生命周期/EventOwner 身份/ErrorKind 家族/五分发观测）——「零新机制的检验口径=检验机制而非数据结构」（lead 批为设计律级表述），论证见 §2。
2. **trait 七方法（融合后）**：`open_mailbox`（预注册信箱）/`register`（成员上线）/`members`/`subscribe`（观测推）/`send`（寻址投递，from 由 hub 派发）/`drain`（消费拉）/`history`（留痕查询）——全同步短临界区，§1。
3. **防伪造定案（合并 a）**：send/drain 的身份一律由 hub 按调用 fiber 从成员表派发，签名层面无 from 参数——模型与插件实现都无法伪造发送者；v1.0 的「进程内互信」留位就此转定案（§3 表 a）。
4. **双通道定案（合并 b）**：drain 拉模式=消费通道（AgentLoop 每轮请求前取净注入 system 段，经契约③ section 缝）；subscribe 推模式=观测面保留（尽力而为）。信箱状态机 Pending↔Online 承载离线补投：send 到非 Online 在册名=缓冲（回执计 buffered），未在册名=HubUnknownMember fail-loud（typo 不静默吞）——spawn 窗口不丢消息，§1.4。
5. **MessageKind 三值 Text/System/Report（合并 c）**+**留痕环形 1000（合并 d，常量在实现、机制语义冻结）**。
6. 错误形态三变体不变（`HubDuplicateMember`/`HubUnknownMember`/`HubInvalidName`，全库 grep 实测 ErrorKind 无穷尽 match=加法演进）；冻结面十项+留位不预造八项（§4）；T1 独立验收六条+测试矩阵十二例（§5）；无【待裁】事项（融合冲突已由 lead 合并令覆盖，§3 逐条标注裁决依据）。

## 1. 服务 trait 最小面（注册成员/寻址投递/留痕+信箱双通道）

### 1.1 契约定位与边界

- 裁①定性：message.v1 是契约层新契约，与 12 条内核契约同级挂 manifest `contracts` 词汇表（`validate_manifest` 的 name.vN 格式已支持）。
- 三补③原文（台账 97 行）：「消息池=内核级 MessageHub 服务（**发布订阅+寻址+留痕**，面板插话/成员互联/AI 自提醒同一管道），团队插件只是第一个客户端——缝在内核卡在插件」。
- 与名册服务的边界：名册三件套之一（role/capabilities/max_concurrency/cost_quota）归 T2 名册服务（裁③）；MessageHub 只持「可寻址名字+归属+信箱」，members() 是名册的消息侧投影。
- **drain 是宿主机制不是模型工具**：取净注入由 T2 在每轮请求前系统级完成（模型无感）；send_message 才是模型面工具（S4 规格，无 from 参数——防伪造见 §1.4）。

### 1.2 trait 签名（T1 实现批按此落盘；全同步——同步性裁决见 §3 表 e）

```rust
/// 契约 message.v1：进程内消息枢纽（发布订阅 + 寻址 + 留痕 + 信箱补投）。
/// 词汇单源：任务26 台账原文；蓝图二期词汇不进本契约面。
pub trait MessageHub: Send + Sync + 'static {
    /// 预注册信箱（Pending 态）：spawn_teammate 拉起窗口的消息落点。
    /// EffectHandle 挂调用 fiber——spawn 失败可提前 dispose 回收名字；
    /// 重名（任何态）/空白名 fail-loud。
    fn open_mailbox(&self, name: &str) -> Result<EffectHandle>;

    /// 成员上线：绑定当前 fiber。Pending→Online（补投就绪）；无条目→直接
    /// Online（自发成员，如 lead）；已 Online→HubDuplicateMember。
    /// EffectHandle 语义：fiber 死亡→条目降回 Pending（信箱与未取净消息保留，
    /// 同名重注册即续投）——非删除。
    fn register(&self, name: &str) -> Result<EffectHandle>;

    /// 名册消息侧投影（online 标志+归属；Pending 无归属=spawn 窗口可见性）。
    fn members(&self) -> Vec<MemberInfo>;

    /// 观测面订阅（推，尽力而为）：当前 fiber 绑定成员的入站镜像；
    /// handler 失败记日志不上抛（消费保证在 drain，§3 表 e）。
    fn subscribe(&self, handler: MessageHandler) -> Result<EffectHandle>;

    /// 寻址投递：身份由 hub 按调用 fiber 派发（防伪造定案）；Online=入信箱
    /// +观测镜像，Pending=信箱缓冲，未在册=HubUnknownMember。
    /// 同步短临界区（锁内不 await）；广播 All 不含 from 自己。
    fn send(&self, to: Address, message: OutgoingMessage) -> Result<SentReceipt>;

    /// 消费通道（拉）：取净当前 fiber 绑定成员的信箱（log_id 升序）；
    /// 非成员 fiber=HubUnknownMember（防越权读他人信箱）。
    fn drain(&self) -> Result<Vec<DeliveredMessage>>;

    /// 留痕查询：环形 1000 丢最旧；完整历史=各成员会话 JSONL（会话域，云伍 §3.3）。
    fn history(&self, query: HistoryQuery) -> Result<Vec<HistoryEntry>>;
}
```

object-safe 核验：无泛型类型参数、全同步返回——比 v1.0 更简（推模式降级后 await 语义消失）。

### 1.3 数据形态（serde derive 全覆盖=二期 IPC schema 种子，云伍 §2/§5 兼容纪律吸收）

```rust
pub enum Address { All, Member(String) }          // All=在册全体（含 Pending，缓冲）
pub enum MessageKind { Text, System, Report }     // 三值冻结；Report=完工自动汇报机制语义
pub struct OutgoingMessage { pub kind: MessageKind, pub body: String }
pub struct MemberInfo { pub name: String, pub online: bool, pub fiber: Option<EventOwner> }
pub struct DeliveredMessage { pub log_id: u64, pub from: String,
    pub kind: MessageKind, pub body: String }     // 收件面不暴露 to=最小知识面
pub type MessageHandler =
    Arc<dyn Fn(&DeliveredMessage) -> BoxFut<'static, Result<()>> + Send + Sync>;
pub struct SentReceipt { pub log_id: u64, pub recipients: usize, pub buffered: usize }
pub struct HistoryQuery { pub limit: usize }
pub enum DeliveryStatus { Delivered, Buffered }   // 真账区分即时送达与缓冲补投
pub struct HistoryEntry { pub log_id: u64, pub at_ms: u64, pub from: String,
    pub to: String /* "All" 或成员名 */, pub kind: MessageKind, pub body: String,
    pub outcomes: Vec<(String, DeliveryStatus)> }
```

### 1.4 错误形态、fail-loud 与投递语义

| 方法 | 失败 | 错误形态 | fail-loud 落点与消费方动作 |
| --- | --- | --- | --- |
| open_mailbox | 同名在任何态 | `HubDuplicateMember` | 立即 Err，报错格式对齐 ServiceDuplicate（service.rs:162-168） |
| open_mailbox | 空名/全空白 | `HubInvalidName` | 立即 Err（空名=永不可达地址，注册时堵） |
| register | 已 Online（本 fiber 或他人） | `HubDuplicateMember` | 立即 Err——重名绑定=路由欺骗，绝不静默接管 |
| send | 调用 fiber 未绑定成员 | `HubUnknownMember` | 立即 Err，消息不产生——**防伪造的机制化**：无身份者发不出 |
| send | to 名无任何条目 | `HubUnknownMember` | 立即 Err；send_message 工具据此报「投不出去+在册名单」（typo 不静默吞，lead 合并令 b） |
| drain | 调用 fiber 未绑定成员 | `HubUnknownMember` | 立即 Err（防成员 A 取净 B 的信箱——越权读对偶防线） |
| history | 存储异常 | `Internal` 兜底 | 一期内存表不应出现，出现即内核缺陷 |

**fail-loud 总纲三不**（维持 v1.0 已批表述）：不静默降级（对齐官方 plugin-manager evaluatePluginCompatibility「版本不符拒装」先例，插件化改造方案 §2.5 已引）；不吞错（反面教材=events.rs:83 fire-and-forget 丢错误——消息消费面不学它，观测面例外见下）；不假报成功（send 返回 Ok 当且仅当全部目标受理入箱/入缓冲）。

**身份派发（合并 a 定案）**：send/drain 不接受名字参数，hub 按调用 fiber 查成员表派发身份——模型自报无效（工具无 from 参数），T2 工具实现也无法伪造（hub 是唯一身份源）；发送点必须在成员 fiber 上下文。**Report 联动细则（合并 c 细节）**：完工自动汇报=成员 run_turn 终结时**由成员 fiber 自身**发 Report 给 lead（T2 在成员 fiber 内挂收尾钩子）；若 T2 在 lead fiber 监听 status 代发，身份被解析为 lead=语义错误，被本机制拦截、集成测试必暴露——防伪造倒逼正确架构。

**信箱状态机与补投（合并 b 细节）**：`Pending（信箱开、无 fiber）⇄ Online（绑定 fiber）`。open_mailbox→Pending；register→Online（补投就绪）；fiber 死亡→Pending（信箱与未取净消息保留）；同名重注册→续投。send 到 Pending 名=缓冲（回执计 buffered，观测镜像照发、状态标 buffered）；注册后**首次 drain 即得全部缓冲**（云伍「注册时补投」的等价落法：缓冲对 drain 可见即=补投，注册时主动推送无意义——成员尚未开始消费）。预注册=open_mailbox，名字在 spawn_teammate 时已知，故 typo 名不在预注册集=无条目=HubUnknownMember——离线补投与防 typo 两全（lead 合并令 b 原文要求）。

**投递/锁语义**（云伍 §3.4 吸收）：send/drain 为同步短临界区（Mutex，锁内快照、锁外 spawn 观测通知——events.rs:82-93 emit 同款纪律「写锁绝不跨 await」，service.rs 注释同律）；drain 返回按 log_id 升序（全局发送序）；并发 send 下同收件人处理序不承诺=全局 log 序（收件方按 log_id 排序自理）；留痕与受理同点落账（outcomes 即真账）。

### 1.5 provide 落点与消费范式（lead 基座枚举维持，§3 表 f/g）

- 实现体落 `crates/yunyou/src/hub.rs`（trait+InProcessHub：成员表+收件表+留痕环形缓冲），RootState 挂载（与 EventTable 同位，events.rs:71 同构）。
- 内核 root 装配以 `ServiceId::of::<dyn MessageHub>()` provide（root fiber 常在，EffectInactive 免疫）——缝由内核供：team 插件禁用不得杀缝（三补③原文），面板/AI 自提醒等后续消费方不依赖 team 卡在位。
- 消费范式：团队插件 `inject()` 声明 `dyn MessageHub`+manifest `contracts: ["message.v1"]`，`ctx.get` 取用（compaction 消费 llm 同款范式）；**严禁进 assemble.rs**（主板论第 6 条）。
- 观测镜像：每次受理后向进程总线 emit `message.v1/watch`（五分发 emit，尽力而为），payload 六位 [log_id, from, to, kind, body, 状态]；/team watch 的消息流数据面（云伍 T3 的 TurnObserver 事件流是另一路观测源，两者不合并——语义不同）。

## 2. 与 scoped events 归属分发底座的关系论证（v1.0 已批基座，原文保留）

### 2.1 底座现状（scope.rs 183 行实测）

ScopedTable（scope.rs:43-45）是进程级单例 EventTable 上**叠加**的归属元数据通道：`事件键 -> [(监听器, 归属 fiber id)]`；EventOwner={fiber_id, fiber_name}（scope.rs:22-28）；分发过滤在**快照阶段**完成，「跨 fiber 监听器连 spawn 都不发生」（scope.rs:76-78）；payload 由内核补前两位 [fiber id, fiber 插件名]（scope.rs:89-91）；`on_owned` 随 fiber 卸载自动注销（scope.rs:152-163）；`emit_owned` 无归属 fiber 时空转（scope.rs:174-176）。设计目标明写「只服务契约⑧事件面（volatile-update 与 reference-updated）」（scope.rs:5-8）。

### 2.2 案甲（复用归属分发当投递通道）四点不适配

| # | 不适配点 | 证据 |
| --- | --- | --- |
| 1 | **送达不可观测**：emit fire-and-forget，收件回调缺失/失败 hub 无感，send 无法对「已投未达」fail-loud——与「错误形态+fail-loud 语义」直接冲突 | scope.rs:94-99 `let _ = listener(&args).await` 丢弃结果 |
| 2 | **错误丢弃**：消息内容投丢了发送方不知道 | events.rs:83 注释原文「错误按 TS fire-and-forget 语义丢弃」 |
| 3 | **键无隔离**：消息与业务事件共用一张 scoped 表，过滤维度只有 fiber_id，契约上无法阻止第三方向同键发无关 payload，信封校验无从谈起 | events.rs:73-79 三表结构；scope.rs 全文无按来源校验 |
| 4 | **留痕脱节**：emit 后 hub 不知投递是否发生，留痕只能记「发」不能记「达」，outcomes 真账形态不成立 | 同 1 |

另注：emit_owned 的 owner 语义是「事件归属」（谁的事件），不是「收件寻址」（投给谁）——Volatile::update 特意自带 owner 元数据「以免换了个 ctx 就把 owner 发错」（scope.rs:170-172 注释），owner 是发送方身份而非路由键，两者语义不同构。

### 2.3 案乙（平行新表+机制复用）的四项复用清单

| 机制 | 复用点 | 证据 |
| --- | --- | --- |
| 生命周期 | register/subscribe/open_mailbox 返回 EffectHandle 挂 fiber effect：离线降级/观测注销自动完成——零新生命周期机制 | ctx.provide 同构 service.rs:141-152；ctx.on_owned 同构 scope.rs:146-165 |
| 身份 | MemberInfo.fiber=EventOwner 原型直用（pub 导出，events.rs:29） | scope.rs:22-28 |
| 错误 | 三新变体进既有 ErrorKind 家族 | error.rs:11-44；全库无 match ErrorKind 穷尽点（grep 实测） |
| 观测 | 面板/REPL watch 走既有五分发 emit，不加第七种分发 | events.rs:82-93 |

### 2.4 结论与「零新机制」的检验口径（lead 批为设计律级表述）

**案乙定案**。检验口径：**问「有没有重复造既有机制的轮子」，不问「有没有新数据结构」**——裁①已定性 message.v1 是新契约，新契约自带其语义所需的最小数据结构（成员表+收件表+信箱+留痕缓冲）不是重复造轮子；反过来，若把投递塞进 emit_owned，需要叠加送达确认、按来源校验、留痕钩子——那才是对冻结面（16 条五分发回归）的破坏性改造。案甲省的 ~150 行实现行数，代价是契约面永远说不清「消息和事件差在哪」。

## 3. 融合冲突取舍表（lead 合并令四项+落地冲突五项，逐条标注裁决依据）

| # | 冲突点 | v1.0 云缀版（已批基座） | 云伍版 | 终版裁决与依据 |
| --- | --- | --- | --- | --- |
| a | 发送者身份 | from 显式传参+校验在册（互信边界留位） | 宿主侧填写防伪造 | **云伍案升级定案**：签名去 from，hub 按调用 fiber 派发（§1.4）——比云伍原案更强：不止工具实现不信模型，hub 缝上连实现也无法伪造。依据=lead 合并令 a |
| b1 | 消费模式 | subscribe 推为主 | drain 拉为主 | **双通道**：drain=消费（每轮请求前取净注入 system 段，经契约③ section 缝），subscribe=观测保留（尽力而为）。依据=lead 合并令 b |
| b2 | 离线窗口 | 无（Online 严格寻址） | 信箱缓存+注册补投 | **信箱状态机 Pending⇄Online**+预注册 open_mailbox（EffectHandle 挂调用 fiber=spawn 失败回收锚点）；typo 无条目仍 fail-loud。依据=lead 合并令 b，细节落自云缀（lead 令「两全细节你落」） |
| b3 | drain 寻址 | —（新增方法） | drain(member: &str) 按名 | **fiber 派发去名参数**：按名 drain=成员 A 可取净 B 信箱（越权读），防伪造在消费端的对偶。依据=lead 令细节落自云缀 |
| c | MessageKind | 两值 Text/System | 三值 User/System/Report | **三值 Text/System/Report**（lead 裁 Text 命名）；Report 发送点=成员 fiber 上下文（谁终结谁发送，§1.4）。依据=lead 合并令 c |
| d | 留痕容量 | 环形 1000（留位待定） | 环形 1000 默认 | **定案 1000**：机制语义冻结（有界环形丢最旧），常量在实现不进 config（触发=首个需调容量的部署）；完整历史=会话 JSONL（云伍 §3.3 边界吸收）。依据=lead 合并令 d |
| e | 同步性 | send/history BoxFut 异步（为 await handler） | 全同步短临界区 | **全同步**：推模式降级为尽力观测后 await 语义消失，保异步=预造；锁内不 await 吸收（云伍 §3.4）；二期异步化=v2 走版本化（§6）。依据=融合推演（lead 令未覆盖，本行取舍自证） |
| f | provide 主体 | 内核 root provide | T2 MessageHubPlugin provide | **内核 root provide 维持**：team 禁用不得杀缝（三补③原文）；后续消费方不依赖 team 卡在位。依据=lead 合并令基座枚举「内核 hub.rs root provide」 |
| g | crate 形态 | crates/yunyou/src/hub.rs | 独立 crates/yunyou-message | **hub.rs 内核内**：trait 需要 EffectHandle/EventOwner/ErrorKind=内核类型，独立 crate 反向依赖内核=伪独立；hub.rs 不依赖任何组件 crate 即契约层形态（云伍验收 5 的 deps-gate 意图由本形态等价满足）。依据=lead 基座枚举+依赖推演 |
| h | 注册返回类型 | EffectHandle | Disposer | **同一机制两种叫法**（内核名=EffectHandle，注册即 effect）；词汇统一用内核名。依据=词汇单源 |
| i | 消息结构 | 三面分立（出站/送达/留痕） | 单一 HubMessage | **三面分立保留**（收件面不暴露 to=最小知识面；出站面无 from=身份派发）+全类型 serde derive 与 serde(default)/skip_serializing_if 兼容纪律吸收（二期种子）。依据=融合 |
| j | 重名错误 | HubDuplicateMember（新 kind） | ServiceDuplicate 复用 | **HubDuplicateMember**：成员表≠服务表，命名空间分离，可编程匹配精度。依据=v1.0 基座维持（lead 已批 §1.4） |

## 4. 契约 v1 冻结面字段表

### 4.1 一期冻结（本终版+lead 融合令即冻结点；破坏性变更=v2+兼容期，插件化改造方案 §2.5 纪律）

| # | 冻结项 | 内容 | 冻结理由 |
| --- | --- | --- | --- |
| 1 | trait 七方法签名 | §1.2 原样（全同步） | T2 团队插件编译依赖面 |
| 2 | MemberInfo | {name, online, fiber: Option\<EventOwner\>} | 寻址+归属+spawn 窗口可见性最小集 |
| 3 | Address | All（在册全体含 Pending，不含 from）/ Member(String) | 路由词汇；二期重解释点（§6） |
| 4 | MessageKind | Text / System / Report 三值 | 区分成员说话/系统通知/完工汇报；三值防 v2 churn（lead 合并令 c） |
| 5 | OutgoingMessage | {kind, body: String} | 正文=文本，对齐 send_message 工具面 |
| 6 | DeliveredMessage | {log_id, from, kind, body} | 收件回调所见；log_id 单调=排序键 |
| 7 | 留痕面 | HistoryEntry 七字段（outcomes 含 DeliveryStatus）/ HistoryQuery{limit} | Delivered/Buffered 真账是补投可调试性的对账面 |
| 8 | 错误形态 | 三变体+EffectInactive/Internal 复用映射（§1.4 表） | 可编程匹配面；加变体不破存量（无 match ErrorKind 穷尽点） |
| 9 | 观测键 | `message.v1/watch` 键名+payload 六位序（含状态位） | /team watch 消息流数据面 |
| 10 | 留痕机制语义 | 有界环形丢最旧+「完整历史在会话域」边界 | 容量常量在实现（不进 config），机制承诺进契约 |

### 4.2 留位不预造（防过度设计抓手③；先例=插件化改造方案 §8 D7「按需激活」、契约⑫；云伍 §5 一期不做清单已去重吸收）

| # | 留位项 | 为什么现在不做 | 触发条件 |
| --- | --- | --- | --- |
| 1 | hub 留痕持久化（跨重启查询） | hub 留痕=观测面（1000 环形）；完整历史=各成员会话 JSONL（云伍 §3.3 定案边界） | 首个「hub 级跨重启回看」消费方立项 |
| 2 | MessageKind 第四值 | 三值已覆盖现有消费方（文本/系统/汇报） | 二期网关媒体/结构化卡片面（样板：dsh-go/media 契约 v1 meta.extra 承载位，插件化改造方案 §3-⑪ 已引） |
| 3 | 收件过滤器（按 from/kind 订阅） | 一期成员量≤十人级，全收无负担 | 成员量或消息频度使「全收」成负担的实测证据 |
| 4 | 消息优先级/插队 | 面板插话一期=REPL 观测，无打断需求 | 二期网关 admin 面立项（云伍 §5 同项） |
| 5 | 回执分级/确认重试（送达/已读 ack） | 一期同步入箱即受理，无异步 ack 必要 | 二期跨进程 at-least-once 投递立项（云伍 §5「消息确认/重试」同项） |
| 6 | 代发/委托（on-behalf-of） | **防伪造定案的反向需求**：以他人身份发送须走显式委托语义，v1 无消费方 | 二期网关 admin 面（v1.0 留位「from 强绑定」的继任形态） |
| 7 | 组播/频道 | All=全体在册（单层团队广播）≠分组组播（云伍 §5「广播组」同项，术语澄清） | 首个「按 role 群发」消费方 |
| 8 | 跨进程传输/持久化表/加解密/稳定 id | 云伍 §5 原文吸收，归蓝图 §7 二期演进 | 见 §6 衔接 |

## 5. 一期验收口径（T1 独立验收=什么算完）

1. **文档面**：本终版为 T1 实现批最终依据（lead 融合令+云枢 retro-review 补审）。
2. **代码面**（T1 实现批，排期=四裁④在途批收口后）：`crates/yunyou/src/hub.rs`（trait+InProcessHub，单文件 ≤300 行）+error.rs 三变体+lib.rs 导出+RootState 挂载+root provide。
3. **测试矩阵十二例**：①重名 open_mailbox fail-loud；②空白名 fail-loud；③register 已 Online 名 fail-loud（他人绑定）；④无 fiber 注册/订阅报 EffectInactive；⑤send 未绑定成员 fiber=HubUnknownMember（防伪造拦截）；⑥send 未在册名=HubUnknownMember（typo 防静默吞）；⑦spawn 窗口缓冲→注册→首次 drain 取净（补投）；⑧fiber 死亡降级 Pending→重注册→残留消息续投；⑨Pending 名 send 回执计 buffered+留痕 outcomes=Buffered；⑩广播 All 不含 from+空集 Ok；⑪drain 非成员 fiber=HubUnknownMember（越权读拦截）；⑫留痕环形 1000 丢最旧+manifest contracts=["message.v1"] 过 validate_manifest。
4. **范式门禁**：`ctx.get::<dyn MessageHub>` 可取；assemble.rs 零触碰；events.rs/scope.rs 语义零改动（16 条五分发回归原样绿）；hub.rs 不依赖任何组件 crate（契约层形态，云伍验收 5 意图）。
5. **门禁无管道直跑**：cargo test -p yunyou 0 败+cargo clippy -p yunyou --all-targets -D warnings 零警告，贴退出码。
6. **词汇自证**：member/lead/名册/寻址/留痕/信箱与任务26 台账原文一一对应；grep 无 worker/租约/SQLite 等二期词混入 v1 面（hub.rs 注释与错误文案同查）。

## 6. 二期衔接说明（多进程 IPC 时契约不动、实现换血的边界）

1. **换血的正确切面**：二期不是把 v1 改造成 IPC——跨进程协作主体走 SQLite/文件（落地方案 §7 原文「一期无 WS」），进程内成员继续由 v1 服务。「换实现不换契约」的兑现=实现内部机制可换（信箱直取→通道化→代理转发）；**v1 全同步签名的二期代价**：若跨进程契约需要异步签名，走 message.v2 版本化（破坏性变更+兼容期），不静默改（插件化改造方案 §2.5 纪律）。
2. **远程成员走二期新契约**：远程 worker 消息面归蓝图二期（任务图+花名册，落地方案 §4）；若收敛进 message 契约族，发 message.v2 加法演进（MemberInfo 增稳定 id，MemberName↔worker_id 对齐）；版本校验对齐官方 plugin-manager evaluatePluginCompatibility 先例：版本不符拒装，不静默降级。
3. **生命周期语义替换点（必须写进二期契约文档）**：进程内「fiber 死亡降级 Pending+信箱保留」↔跨进程「租约过期出册」（二期词汇，蓝图七-3）——两套机制并存，v1 的 EffectHandle 承诺范围**明确限定=进程内**，防二期把 effect 注销误当跨进程下线信号（lead 已批认定点，维持）。
4. **Address::All 语义迁移注记**：「在册」定义从进程内条目（Pending/Online）变为租约有效——契约词汇不变，语义按运行态重解释；v1 文档只声明「在册=进程内在册」。
5. **信封序列化**：全冻结类型 serde derive+serde(default)/skip_serializing_if（云伍 §5 兼容纪律）=v2 IPC schema 种子；JSON Schema 生成按落地方案 §7 四件套纪律，触发=首个跨进程消费方立项（一期不做，同契约⑪「按需激活」口径）。

## 7. 自检对照表（任务书五项+合并令逐条）

| 要求 | 本文落点 |
| --- | --- |
| 1. trait 最小面（签名级+错误形态+fail-loud） | §1.2 签名/§1.3 数据/§1.4 错误表+三不总纲+身份派发+信箱状态机 |
| 2. 与 scoped events 底座关系论证（零新机制优先） | §2：现状→案甲四点不适配（行号）→案乙四项复用→检验口径（lead 批设计律级） |
| 3. v1 冻结面（冻结/留位不预造） | §4.1 十项/§4.2 八项（触发条件+先例） |
| 4. 一期验收口径 | §5 六条+测试矩阵十二例+词汇自证 |
| 5. 二期衔接（IPC 换血边界） | §6 五条（含 v1 同步签名的 v2 代价声明——新增诚实条款） |
| 合并令 a-d 融合+冲突取舍逐条标注 | §3 十二行取舍表（a-d 依 lead 令；e-j 五项落地冲突自证取舍）；§0-§6 相应落点已改 |
| 纪律：零实现零 commit；retro-review 标签；行数≤400；台账自记 | 全文仅本文档+台账行落盘；头部已标；实测 wc ≤400 |

> 交付声明：本批零代码、零 commit、crates/ 零触碰；云伍原文保持附录素材地位未改动。交 lead 轮询收取。
