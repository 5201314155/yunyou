# 团队插件 MessageHub 契约设计（任务26·T1）

> 作者：云伍｜日期：2026-11-14｜依据：lead 四裁（2026-11-14 13:03：MessageHub 进契约层 message.v1/fork 一期降级/max_concurrency 进插件 config/S2 已入库）＋业主任务26 立项三补＋《团队运行时落地方案》§7＋插件技能纪律
> 定位：**缝在内核（契约层），卡在插件（yunyou-team）**——团队插件只是 MessageHub 的第一个客户端（lead 三补③原文口径）。本文=设计先行，零实现、零 commit。

---

## 1. 定性与切线

1. **为什么进契约层**：消息面的消费方不止团队插件——面板插话（网关投影二期）、成员互联（peer）、AI 自提醒、完工自动汇报共用一条管道；二期多进程化时换传输实现（进程内→IPC），消费方契约不动。判据=「会被第二个消费方消费、或二期要换实现」→ 主板（契约层）。
2. **为什么不复用 waterfall/events**：waterfall 六事件是**策略改写面**（插件技能 §二⑧：不并入 v1 最小集），events 归属分发是**通知面**——两者都不承载「点对点+寻址+留痕」的消息语义；混用会把消息abus成事件，重演「机制硬编码」反模式。
3. **与蓝图关系**：蓝图 §7 明示「跨进程协作主体走 SQLite/文件，一期无 WS」——本契约是进程内最小面，词汇与蓝图 tasks/消息互通同构，二期多进程落《团队运行时落地方案》§7 的文件/IPC 协议时**换实现不换契约**。

## 2. 契约 crate 与服务键

- 新建 `crates/yunyou-message`（契约 crate，零网络零状态，同 yunyou-llm 形态）：

```rust
/// 消息（一期进程内形态；serde 类型即跨进程 schema 的种子，二期按
/// 《团队运行时落地方案》§7 升格 JSON Schema 生成物）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubMessage {
    pub from: String,          // 发送者名（成员名或 "user"/"system"）
    pub to: String,            // 寻址目标（成员名；"lead"=主 agent）
    pub kind: MessageKind,     // User | System | Report（完工自动汇报）
    pub body: String,
    pub sent_at: i64,          // unix ms（留痕与回看排序）
}

pub trait MessageHub: Send + Sync {
    /// 注册成员收件箱（重名拒绝 ServiceDuplicate）；disposer 注销=逆序清理
    /// 自动摘除（注册即 effect，内核既有语义，零新机制）。
    fn register(&self, member: &str) -> Result<Disposer>;
    /// 寻址投递：目标在场→入其收件箱；离线（未注册）→保留箱缓存，注册时补投
    /// （防「先发后拉」竞态——spawn_teammate 拉起窗口内 lead 的首条消息不丢）。
    fn send(&self, msg: HubMessage) -> Result<()>;
    /// 拉取收件箱（一期拉模式：member 的 AgentLoop 每轮请求前取净，注入 system 段；
    /// 推模式=事件回调，随 S6 事件面按需加，一期不预支）。
    fn drain(&self, member: &str) -> Vec<HubMessage>;
    /// 留痕只读（/team 回看与面板历史；上限环形缓冲防涨）。
    fn history(&self, limit: usize) -> Vec<HubMessage>;
}
```

- 服务键 `ServiceId::of::<dyn MessageHub>()`；provides 投影 `"dyn MessageHub"`；contracts=["message.v1"]。
- **谁 provide**：T2 的 yunyou-team 插件（MessageHubPlugin）。一期 MessageHub 与团队插件同批交付（一个提供方）；契约先行=二期「面板插件/网关插件」直接消费同一服务键，零改造。

## 3. 语义规则（钉死进 trait 文档与 T3 门禁用例）

1. **peer 互通**：send 不限制 from/to 关系——成员互联与 lead↔member 同管道（业主交互规格②「成员间可互发」）；from 由**宿主侧**填写（消费方工具实现从 fiber 归属取发送者身份），不信模型自报（防伪造发送者）。
2. **完工自动汇报是机制不是习惯**：member fiber 的 run_turn 终结（Ok/Err/取消）由团队插件的事件监听自动 send Report 到 lead（「做完了会给你发消息」=内核事件面驱动）。
3. **留痕环形缓冲**：默认 1000 条，超出丢最旧（防长跑涨内存）；完整历史走会话 JSONL（各成员自己的落盘），hub 留痕只是观测面。
4. **取消贯穿**：drain/send 是同步短临界区（Mutex 收件箱，锁内不 await）；不引入长阻塞（内核锁纪律，家规反面清单「锁内取引用再传」同律）。

## 4. T2/T3 接口位（本文不展开，防越界设计）

- **T2（yunyou-team 插件主体）**：teammate=独立 fiber（自己的 Session+AgentLoop）+人设走契约③ section；工具族 spawn_teammate/send_message/team_task_*（S4 规格）；`max_concurrency` 进 `[plugins.yunyou-team.config]`（lead 裁决）；fork 一期降级=「注入 lead 对话摘要」为 system 段文本，真 fork 随二期 session 投影。
- **T3（观测面+状态机门禁）**：`/team` 列表+`/team watch` 订阅 TurnObserver 事件流；任务板 TeamBoard 契约（消息面之外的第二契约，V1 状态机 pending→claimed→submitted→approved/rejected/blocked，**「状态流转只由审核方写、拒绝自审自过」的断言在 T3 钉死为门禁用例**——lead 确认为本设计最值钱一条）。

## 5. 一期不做（防膨胀清单）

跨进程传输（IPC/WS）、消息持久化（SQLite 表）、消息确认/重试、广播组、优先级队列、消息加解密——全部留二期按蓝图 §7 演进；一期契约面不堵塞（字段 serde(default)+skip_serializing_if 双向兼容，与 session 树字段同律）。

## 6. 验收口径（T1 交付时逐条对照）

1. 契约 crate 编译绿+契约文档（本文件）与 trait 同批；2. 消息 schema serde 往返单测；3. 寻址投递/离线补投/重名拒绝三用例；4. 留痕环形上限用例；5. deps-gate：message crate 不依赖内核组件（契约层形态）；6. 行数红线（单文件 ≤300）。

> 自检：本文只定义契约与语义，无实现代码；裁决点均已 lead 四裁覆盖，无新增待裁项。
