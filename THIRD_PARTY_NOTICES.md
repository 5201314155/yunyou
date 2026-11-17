# THIRD_PARTY_NOTICES —— yunyou-web crate 第三方组件登记

## ① 官方前端 vendor 树（frontend/）

- 来源：DeepSeek Harness（`deepseek-harness`，MIT License）。
- 上游基线 commit：`e8f7b93a344964de3dca1ace9e72b6e4d6de87cb`（2026-11-16 钉定）。
- 许可原件：`web/LICENSE.upstream`（上游根 LICENSE 逐字节副本）。
- 上游自带的第三方声明随树保留：`web/THIRD_PARTY_NOTICES.upstream.md`（未改动，
  覆盖上游 workspace 的 npm 依赖面）。
- 改动登记：本树相对基线的改动清单见 `web/README.md`「裁剪形态差异登记」节；
  改动文件以头注「移植自 deepseek-harness@e8f7b93，改动：…」标识别。

## ② 浏览器运行时关键 npm 依赖（经上游声明继承 + 本构建链直引）

react / react-dom（MIT）、zustand（MIT）、immer（MIT）、zod（MIT）、
@tanstack/react-virtual（MIT）、vite（MIT）、tsdown（MIT）、lightningcss（MIT）、
typescript（Apache-2.0）。完整传递闭包以 `web/THIRD_PARTY_NOTICES.upstream.md`
与 pnpm lockfile 为准。

## ③ 复审口径（任务40 rev10）

云点全量复审时本登记件与 `web/README.md` 为许可卫生底册；vendored 原样面抽验
md5 以基线 e8f7b93 工作区为对照源。
