# 网关 API 人工文档（K-12 阶段 C，方案 §8 四件套之一）

> 机器可读形态：`docs/接口/openapi-gateway.yaml`（骨架+路由覆盖锁，tests/openapi_coverage.rs 强制与实现一致）。本文=配对时序/两维部署矩阵/错误处置/scope 申请的人话入口。权威设计：`docs/架构/网关落地方案.md`（D1-D7 业主定案）。

## 1. 两段式令牌流（配对时序）

```
网关签发配对凭证（扫码=QR 载荷 dsh_pair_ 令牌 5 分钟 / 验证码=8 位短码 10 分钟）
    ↓ App 侧
POST /pair/consume        （扫码：Authorization: Bearer <dsh_pair_…>，body {device_name}）
POST /pair/device-code    （验证码：body {code, device_name}）
    ↓ 恰好一次消费（原子防重放；过期/已消费/锁定=拒绝）
200 { session_token: "dsh_sess_…", session_id, device_id, scope }
    ↓ 此后全部请求
Authorization: Bearer <dsh_sess_…>（30 天滑动过期；可撤销立即生效）
```

- 配对凭证一次性：消费成功即失效（重放=409）；失败连续 5 次=锁 15 分钟（429）。
- 会话令牌哈希存储（SHA-256，库泄露不泄可用令牌）；吊销经设备管理端点立即生效。

## 2. 两维部署矩阵（§5/§6）

| 格 | 配对载体 | endpoint 形态 | TLS | 测试 |
|---|---|---|---|---|
| 内网×扫码 | QR（ws://IP:7900+令牌） | ws:// | 不强制（D6） | G1 |
| 内网×验证码 | 界面短码 | ws:// | 不强制 | G2 |
| 公网×扫码 | TLS 端点 QR | wss://域名 | **强制 fail-loud**（未配拒启动） | G3 |
| 公网×验证码 | TLS 端点短码 | wss://域名 | 强制 | G4 |

四格鉴权路径完全同构（同一中间件、同一令牌表、同一 scope 检查），差异仅载体与 TLS——不存在「某一格忘了鉴权」的可能（§6 原文）。G5=healthz 非格（匿名、零信息、恰为 `ok`）。

## 3. 端点与错误处置

九端点清单见 openapi 骨架（security= bearerAuth 除 healthz/pair 消费外统一）。错误统一形态：

| 码 | 场景 | 处置 |
|---|---|---|
| 401 | 无 Bearer/凭证无效/过期/已吊销 | 重新配对或检查令牌 |
| 403 | scope 不足 | 走 scope 申请（§4） |
| 409 | 配对重放/设备数上限（默认 10） | 换新配对/移除旧设备 |
| 429 | 失败锁定（5 次→15 分钟） | 等锁过期，勿重试 |

人话 body 不泄内部细节（§10.2 纪律 2）；`/healthz` 恒 `ok`。

## 4. scope 申请（§7.2）

新设备默认 `chat + panel:read`；`panel:write` 默认**不**授予（业主 D4 定案）——须管理员显式授予（换发新令牌：旧令牌吊销+审计 scope_changed 留痕）。三值冻结：`chat`/`panel:read`/`panel:write`；未知 scope 值解析跳过（加法演进兼容）。

## 5. 联调边界（阶段标注）

- G1/G2/G5=已实测（oneshot 内存服务全链）；
- G3/G4 TLS 握手 E2E+chat WS E2E=**候 agent 侧 ipc-server 就位**（云探线）——桥接逻辑（ws.rs）与 IPC 客户端（ipc_client.rs）已就位，联调格届时标注转绿。
