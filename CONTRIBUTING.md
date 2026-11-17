# 贡献指南（CONTRIBUTING）

> 云悠（YunYou）——DeepSeek Harness 的 Rust 重写：组件化插件主板内核。
> 本指南面向想参与建设的你；写插件（不改内核）请直接看「插件贡献」节。

## 项目一句话

基座=主板，插件=可换的卡：内核只提供插槽规格（契约 12 条+UI 注册缝），一切能力以插件形态插在缝上。规格发布即冻结、版本化演进（改规格=发新插槽版本）。

## 贡献规则（先读这四条）

1. **内核不接受随意 PR**：`crates/yunyou`（主板内核）的改动属「主板要加强」例外条款，需 issue 先行+maintainer（云悠）与架构师（云枢）双裁；功能需求一律走插件形态。
2. **功能一律插件形态**：新能力落 `plugins/<id>/`（每插件一目录，manifest.toml 声明面）或 bundled 插件 crate 提案——30 秒看懂往哪放：能力本体进插件、跨插件服务走 Context 服务注入契约。
3. **契约即冻结面**：`docs/接口/` 五契约（message/sandbox/manifest/tools/commands v1）已发布即冻结；扩展=加法演进（v1.1/v2 新版本），禁悄悄改语义。
4. **纪律即门槛**：改动需过门禁（cargo test + clippy -D warnings + cargo deny）+台账/地图/接口文档同批同步（物理代码唯一真理，文档三件套随批）。

## 插件贡献（推荐入口）

- 开发规范：`docs/设计/CLI终端显示设计规范.md` 同级的插件技能文档（`.dsh/skills/yunyou-plugin-dev.md`——manifest 九字段/Plugin trait 契约/门禁四绿/热插拔纪律）。
- 接口契约：`docs/接口/`（message/sandbox/manifest/tools/commands v1 规格+示例，插件作者第一入口）。
- 安装形态：L1 配置级安装（`plugins/<id>/manifest.toml` 丢目录+开关，零编译）已上线；L2 market 子进程协议安装（stdio JSON-RPC）设计已批待落码。
- 最小实例：`plugins/hello/`（L1 发现层自举样例）。

## 审查流程

- 提交前自查：门禁四绿（test/clippy/deny/行数红线）+涉及文件清单+台账改动记录。
- 审查制：写审分离（作者不自审）+复核人制度；重大改动多线并审（架构/视觉/性能/工程质量）。
- 审查重点：反万能药（无脑 Arc<Mutex>/吞错/`transition: all` 式一把梭一律打回）+自证设计理由式注释（禁「参考官方」话术，对外引用带 license 归属）。

## 开发环境

- Rust stable（1.99+）；`cargo build`/`cargo test --workspace`。
- 门禁三件：`cargo test`+`cargo clippy --all-targets -- -D warnings`+`cargo deny check`。
- 卫生：个人 `CARGO_TARGET_DIR`（多席同机纪律）；隔离 `HOME` 做真机/PTY 验收（每场景显式 env，防污染真实用户数据）。
- 提交规范：中文提交信息写清做了什么；精确路径 add；禁 `add -A`。

## 沟通

- 台账即总线：`项目状态.md`（任务清单/模块索引/改动记录）+`项目地图.md`（目录镜像）——改动同批同步是硬纪律。
- 路线图见 `ROADMAP.md`；设计底册见 `docs/设计/主板蓝图-生态架构蓝图终稿.md`。
