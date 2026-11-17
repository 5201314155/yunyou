# 路线图（ROADMAP）

> 里程碑拆分底稿=docs/设计/主板蓝图-生态架构蓝图终稿.md §七；本文件是对外视图（状态随批更新）。

| 里程碑 | 内容 | 状态 |
| --- | --- | --- |
| M2 内核+五件套+CLI | 组件内核/LLM/会话/工具/系统提示/agent-loop/配置/CLI（发布提交 08e0f6d，live 通过） | ✅ |
| M3 地基封板 | 插件化 S1-S8 全链+L1 装载器发现层（plugins/<id>/ 零编译装载）+命令体系+渲染与命令收尾+P0 缺陷清账+自举投影口径实态验收 | ✅（2026-11-16 封板宣告） |
| M4 UI 缝二期（web v2） | 云幕线 web v2（entry 图+combo+__DSH_BOOT__ 注入）+G1a UiEntry v1.1+G2 rpc-types 纯类型 | 🚧（批①-④ 已入库，「移植可跑」达成；remote.mux 网关 WS 桥=剩余缺口） |
| M5 market 子进程协议 | 三态装载二期段+沙箱 S11（Landlock+seccomp 网络禁断）+plugin-manager+application 五态验证+分册4 P-2/P-6 | 📋 设计已批（L2 七点全批+S11 设计 263 行），落码候排 |
| M6 团队运行时（K-11） | launcher/任务图/worker 双模式/修复链/面板（Z1-Z8 预算验收）；网关同批 | 📋 方案已批（团队运行时落地方案 §9.1）；team 插件 v1 已落地（K-7，live 认账）为进程内第一层 |
| M7 开源基建（K-15/任务28） | 插件开发指南/cargo-generate 模板/README 全景图/混沌 CI 徽章/CONTRIBUTING/License/Roadmap+整容批（crate 平名化+话术清扫） | 🚧 本卡（CONTRIBUTING/Roadmap 已落；九条盘点细案在案） |

## 里程碑之外（持续线）

- 性能验收：分册4 冻结口径（B1-B9 地基指标+Z1-Z8 自修复预算），基准套件首轮分布后进门禁。
- 观测面：/plugins 来源标注+cordis_inspect 对标+审计探针（分册4 §5.2）。
- 生态承载：见蓝图终稿 §四「生态机制最小集」（开发/安装/活着/分发/观测五环）。
