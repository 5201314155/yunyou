# web.v1 流通道与事件契约（websocket.md）

> **契约 ID**：`web.v1`（流面分册）｜**状态**：**已冻结 v1.0（2026-11-19，lead 批）**
> **依据**：`crates/net/yunyou-web/src/gateway.rs`/`wire.rs`/`bridge.rs` 实测；上游消费端铁律（session-controller/client transport）按 wire.rs 注释记录的实测纪律冻结。
> 总则（信封/错误/演进/secret/权限）见 `规格.md`；本文只定 mux 流与事件词汇。

## 1. 通道与帧协议

- 入口：`GET /api/remote.mux`（WS 升级，token 鉴权先行，无匿名通道）。
- 帧形态：JSON 文本帧。上行四词汇，下行四词汇：

```jsonc
// 上行
{ "type": "open",   "streamId": "s1", "endpoint": "session/follow", "payload": { ... } }
{ "type": "item",   "streamId": "s1", "value": { ... } }   // 本版无上行消费方：即刻忽略
{ "type": "end",    "streamId": "s1" }                     // 同上
{ "type": "cancel", "streamId": "s1" }                     // 同上
// 下行
{ "type": "item",  "streamId": "s1", "value": { ... } }
{ "type": "end",   "streamId": "s1" }
{ "type": "error", "streamId": "s1", "error": { "code": "...", "message": "...", "details": {} } }
```

- 未知上行 `type`：回 `gateway/bad-frame` error 帧（fail-visible）。
- 未识别 endpoint：error（`gateway/not-implemented`）+ end 帧。
- 端点名兼容斜杠形（`session/follow`）与点形（`session.follow`），斜杠为规范形。
- 积压丢帧是约定行为：客户端按 seq/revision 跳号自重连对齐。

## 2. 流端点

| endpoint | 方向 | 基线（打开即回） | live | 状态 |
| --- | --- | --- | --- | --- |
| `session/follow` | 服务端→客户端 | `{type:"snapshot",header:{version:1,id,createdAt,isSeeded},projections,cursor,records[],hasMore,assistantStream}` | 会话事件帧 + assistantStream 帧 | [已接] |
| `session/control` | 服务端→客户端 | `{type:"baseline",value:{projections:{<sessionId>:{asOfSeq,values}}}}` | `{type:"projection",sessionId,seq,changes:[{key,val}]}` | [已接] |
| `workspace/follow` | 服务端→客户端 | `{type:"baseline",value:{items:[{workspaceId,path,title,sessionIds,createdAt,updatedAt}],archivedSessionIds:[],pinnedSessionIds:[]}}` | 单工作区投影，本版无变更推流 | [已接] |
| `$events` | 服务端→客户端 | `{type:"ready",clientId,host:{home}}` | waterfall 事件（域事件下行），通道已建、词汇随域分批 | [已接通道/词汇分批] |

## 3. session/follow 事件词汇（[已接]，形状冻结）

### 3.1 持久事件（进快照 records， seq 致密 +1；surface 事件带 `surfaceOp:"append"`）

每帧形态：`{type:"item",streamId,value:{type:"event",event:{type,seq,time,data[,surfaceOp]}}}`。

| event.type | data 关键字段 | surface |
| --- | --- | --- |
| `user/message` | `{id,role:"user",content:[{type:"text",text}],source:{kind:"user",rpcId?}}` | ✓ |
| `turn/start` / `step/start` | `{turn}` / `{turn,step}` | — |
| `assistant/message` | `{turn,step,message:{id,role:"assistant",content:[{type:"text"|"reasoning",…}],source:{kind:"model",provider:"yunyou",model}},stream:[]}` | ✓ |
| `tool/call` | `{turn,step,callId,name,arguments}`（arguments=120 字摘要，全量属演进项） | — |
| `tool/result` | `{turn,step,message:{id,role:"tool",content[],source:{kind:"tool",callId},toolCallId,isError}}` | ✓ |
| `step/end` | `{turn,step}` | — |
| `turn/end` | `{turn,reason:{kind:"completed"}|{kind:"error",error:{message,code}}}` | — |
| `system/message` | `{turn,step,message:{id,role:"system",content[],source:{kind:"system"}}}` | ✓ |

铁律：`user/message.data.source.rpcId` 必带（prompt requestId echo 对账，缺失=乐观 echo 永不退场）；assistant/message 的 `end` 帧 committed seq 必须指向该落定事件。

### 3.2 assistantStream 暂态帧（不进 records）

`{type:"item",streamId,value:{type:"assistant-stream",frame:{…}}}`，frame 三形态：

- `start`：`{attemptId,revision,startedAfterSeq,turn,step}`；
- `chunk`：`{attemptId,revision,index,time,chunk:{type:"text-delta"|"reasoning-delta",index,text}}`；
- `end`：`{attemptId,revision,index,outcome:{kind:"committed",eventType,seq}|{kind:"abandoned"}}`。

铁律：`revision` 逐帧 +1（跳号=客户端重连信号）；chunk `index` 致密 0,1,2…；快照打开方必带 `assistantStream` 基线（`{revision, activeAttempt?}`），缺失=客户端抛错。

### 3.3 快照语义

- records=重放环（512 封顶，丢最旧；完整流在会话 JSONL）；`hasMore:false` 为诚实现态。
- `cursor` = 已分配最大 seq（未铸会话= -1 计数基线，客户端按致密链消费）。
- `header.id` = 请求 address.sessionId（或当前会话；未铸= "default"）。

### 3.4 projection 帧（session/control live）

`changes` 键集：`title`（首条用户消息派生，首行截 40 字）、`sessionListMetadata`（`{blank,lastPromptAt}`）。`seq` = 触发事件的 seq，消费端 higher-seq-wins。

## 4. 事件下行与加法演进（[冻结待接]）

1. `$events` 流承载跨域事件下行，词汇按域分批接入；第一批（任务/团队/定时/配置）：
   - `tasks/changed` `{taskId,state,owner?}`——任务板任意迁移后推；
   - `team/roster-changed` `{name,online,busy,task?}`——名册观测变化推；
   - `schedule/fired` `{id,message,nextFireMs?}`——到期投递回执（真投递缝随 proto v1.1，未开闸前不发）；
   - `config/reloaded` `{revision}`——配置写入生效推（配合 revision 作废缓存）；
   - `system/restarted` `{}`——进程重启（revision 作废信号）。
2. 事件 data 均为**增量提示**，消费端收到后拉对应投影（tasks/list、team/roster 等），事件本身不携带全量——避免双真源。
3. 加法演进纪律同 规格.md §6：未知 event type 必须可忽略。

## 5. 连接生命周期

1. 升级即鉴权；`Hello`（模型名）属旧 `/ws` 词汇，v2 mux 面**不再使用**——页面元数据走 `system/info`。
2. 客户端重连：带 cursor 打开 `session/follow` → 快照 + 增量；revision/seq 跳号即整流重开。
3. 服务端写失败=连接已死，收场；不做半写重试。
4. `/quit` 语义：断开本连接，不杀服务，其他连接不受影响（壳层裁决，不入事件流）。

## 6. 兼容性清单（消费端必须容忍）

- 未知 event type / 未知 endpoint / 未知 frame 内字段：忽略或重连，不崩。
- tool/call `arguments` 为截断摘要（120 字符），全量属后续观察面演进。
- tool/result `content` 可能是字符串或结构化 JSON（canonical value 原样）。
- usage（token 用量）字段本期缺位于 assistant/message（TurnComplete 时机问题），缺失=不显示用量，属已知演进项。
