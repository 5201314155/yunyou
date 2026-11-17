# 网关 IPC 协议人工文档（K-12 阶段 C，方案 §8 四件套之一）

> 机器可读形态：消息类型单源=`crates/yunyou-ipc-types`（serde+schemars 唯一真源，§2.3 消息族封闭枚举——私加消息类型=编译失败）。权威设计：网关落地方案 §2（业主 D1 裁决新增，一期范围）。双端消费：yunyou-cli（agent 侧 ipc-server，云探线）+yunyou-gateway（网关侧 ipc_client，云络）。

## 1. 传输与帧格式（§2.1）

- **Unix domain socket**：`<yunyou-home>/run/gateway.sock`（`run/` 0700、socket 0600；已存在先安全删除防残留劫持）；127.0.0.1 TCP 为 Windows 等无 UDS 平台备选（一期 Linux 不实现）。
- **帧格式=JSON Lines**：每行一消息，`\n` 分隔（可调试、与 JSON Schema 纪律同源；不用二进制长度前缀）。编解码单源=`yunyou_ipc_types::{encode_frame, decode_frame}`。

## 2. 凭证机制（§2.2 进程外不可得，双因子）

1. **UDS 文件权限**（到不了文件就连不上）：`run/` 0700 + socket/secret 0600；
2. **首帧 secret 认证**：agent 启动生成 32B 随机 secret（CSPRNG，0600，每次重启轮换）→ 网关连上后首帧 `hello{secret_hash: SHA-256 十六进制}`（明文不过线）→ agent 比对 → `hello_ok` / `hello_err`+断连审计。
- **fail-loud**：连续 3 次认证失败 → agent 删除 socket 并告警（防暴力试 secret；AUTH_FAIL_LIMIT 常量单源）。
- 心跳：`ping`/`pong` 30s 间隔，连续 3 次超时断开。

## 3. 消息族（§2.3 封闭清单——新增=方案修订）

| 消息 | 方向 | 要点 |
| --- | --- | --- |
| hello / hello_ok / hello_err | 网关→agent / 回 | secret 哈希+协议版本；resume 信息可选 |
| ping / pong | 双向 | 心跳 |
| session.open / session.opened | 网关→agent / 回 | device↔session 惰性创建（§2.5：同设备重连续聊、多设备隔离） |
| chat.send | 网关→agent | 用户输入 → agent 投喂 AgentLoop |
| chat.events | agent→网关 | 会话事件流，seq 单调递增（源自会话 JSONL 持久化序号） |
| panel.tasks.read | 网关→agent | 只读投影（agent 侧编排层生成，网关不见原始存储） |
| panel.command.submit | 网关→agent | 只入队（消费权在编排层 admin 组件） |

**暴露面铁律（业主硬约束③）**：封闭清单内**不存在**任务图直写、状态流转、花名册管理消息——类型层封闭枚举+反序列化端未知标签拒绝双执行。

## 4. 断连重连与事件补发（§2.6 业主硬约束⑤）

- `chat.events` 每会话单调 `seq`（源自会话 JSONL 持久化序号——天然可回放）；
- 重连：`hello.resume{session_id, last_seq}` → agent 从会话存储回放 `seq > last_seq` 按序补齐 → 恢复实时流；
- **背压语义（丢连接优于丢事件）**：agent 侧每连接有界通道（默认 256）；满=断开该网关连接——事件不丢（持久化），重连走回放补发；agent-loop 主路径（写会话存储）永不被网关消费速度阻塞；
- 退避：网关侧 1s 起指数、上限 30s+抖动；hello_err → 立即重读凭证一次，仍失效=长退避+告警。

## 5. 部署约束（§2.1/§9.1）

- 一期目标平台 Linux（UDS）；Windows 备选不实现；
- 网关进程为纯 tokio 服务（无组件内核）；`yunyou-gateway` 不依赖 `yunyou` 内核（deps-gate 黑名单语义，deny.toml 注记在案）；
- 凭证文件路径约定：`run/gateway.sock`+`run/gateway-ipc-secret`（ipc-types::paths 单源）。

## 6. 联调边界（阶段标注）

- 帧编解码/握手协议/退避序列=阶段 A 已入库（单测锁）；
- agent 侧 ipc-server（凭证轮换/hello 认证/会话桥接/补发）=任务 3 cli 批次（云探线）——就位前 E2E 联调格标注「候 ipc-server」，网关侧接口面已冻结（IpcConnection connect/send/recv 三件）。
