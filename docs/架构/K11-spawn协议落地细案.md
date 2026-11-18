# K-11 spawn 协议落地细案（步 2 service 分支接线规格）

> 承办：云点｜性质：实施规格（b 案四步序步 2 前置设计面；级联解挡即码）｜日期 2026-11-18
> 已落地件：launcher build_worker_command（15937e0e1）/worker-core attach_parent_watch（31002492b）。

## 一、接线规格（team service 步 2 分支）

**分支点**：TeamConfig.worker_mode（task_types 枚举已落地）——装配处二选一：
- Fiber（缺省）=现态 MemberPlugin 子 fiber（零改）；
- Process=成员创建请求→launcher::spawn_worker(SpawnSpec)→procmgr::register(pid)→DeathEvent 消费循环挂接。

**DeathEvent 消费（收敛序四步的 service 侧）**：
```
procmgr.sweep_timeouts(now) / reap_exit(pid, status)
  → DeathEvent{pid, session_id, reason}
  → service 侧：board.claimed 任务→回 pending（附 K-16 中断 Report 语义）
  → hub 通报成员下线（Pending 认领语义留重启 worker）
  → lead 升级报（K-16 ②a 衔接）
```
消费循环=装配期 spawn 的常驻任务（tokio select：sweep 定时器+DeathEvent 通道）。

## 二、argv/env/socket 全表

| 项 | 值 | 来源 |
|---|---|---|
| argv[0] | worker_bin 绝对路径 | target 构建产物或配置指定 |
| --session | `<member-name>-<unix_millis>`（派生自 TeamConfig.path 父目录） | 会话派生惯例 |
| --proto-socket | `<home>/run/gw.sock` | ipc-types paths 单源 |
| env | PATH+TMPDIR 双键 | launcher 既有白名单 |
| PDEATHSIG | worker 侧自挂 host_pid（b 案 attach_parent_watch） | 31002492b |
| 沙箱 | policy_json+helper（manifest permissions 合成） | K-19 §4.1 勘察项联动 |

## 三、验收接线（六场映射）

六场（细案 2effdb406）→本细案件位：①握手全链=worker-core 7/7 已锁②心跳超时=procmgr sweep 4/4 已锁+service 消费=步 2 ③④⑤=集成批⑥=回归批。

## 四、级联依赖注记

team check 级联 agent-loop（member.rs AgentLoopRunner 依赖）——agent-loop 四件 M（新团队修复批②嫌疑）收敛前 team scoped check 不可得；本细案=设计面先行，码=解挡后一小时窗。
