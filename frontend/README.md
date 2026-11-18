# web/ —— 云悠 web 前端 vendor 树（官方移植）

## 来源与基线

- 上游：`deepseek-harness`（MIT），业主亲指定 v2 路径（任务40 卡面 rev10/rev12）。
- 基线 commit：**e8f7b93a344964de3dca1ace9e72b6e4d6de87cb**（2026-11-16 由云屏钉定，
  云点复审底册=本树 vs 该基线 diff）。
- 复制方式：rsync 逐包拷贝（排除 node_modules/lib/dist/tests/snapshots/\*.spec.ts）；
  除本 README、根 package.json、根 pnpm-workspace.yaml、根 tsdown.config.ts 外，
  文件内容与上游基线逐字节一致（改动文件头注「移植自 deepseek-harness@e8f7b93，
  改动：…」——当前仅 tsdown.config.ts 一件）。

## 裁剪说明（相对上游整仓）

- **保留**：apps/web（Vite 入口壳）+ packages/ 内 client 面构建闭包（约 300 工作区包，
  按依赖链收敛，收敛过程=三轮 pnpm install 缺口回填至不动点）+ vendor/*（cordis 系
  九件）+ scripts/ 构建脚本三件+ tsconfig 基座三件+ native/system/packages/entry
  （node-addon-system，webworker-runtime 依赖）。
- **不入树**：host 世界原样面（apps/cli、apps/desktop、benchmarks、website、python、
  native 其余、experimental 大部、llm/shell/fs 等域的宿主实现体随闭包入树但只出
  client 产物）；测试树与 spec 文件。
- **裁剪形态差异登记**（云点复审对照用）：
  1. 根 package.json：重建为最小根（仅 tsdown/typescript/tsx/lightningcss 四 devDep
     +两构建脚本；packageManager=pnpm@11.9.0 对齐本机，上游 11.28.5）；
  2. pnpm-workspace.yaml：glob 缩至 vendor/*、packages/*/*、apps/*、
     native/system/packages/*；overrides 保留 cosmokit/schemastery 两条；allowBuilds
     子集四件 deny（宿主世界传递依赖，浏览器构建不消费）；
  3. tsdown.config.ts：client-face 专用（host 分支删除，见文件头注）；
  4. apps/web/package.json：devDeps 裁至构建实需十件（上游 25+ 件为宿主/测试世界）。

## 构建

```bash
pnpm install                # 依赖安装（npmmirror）
pnpm run build              # 一键链：build:client → build:web → build:boot
```

三段链：

1. `build:client`：tsdown 全子集 client 产物（各包 `lib/client.js` 模块束）；
2. `build:web`：vite 构建 apps/web → `apps/web/dist`（壳产物唯一落点）；
3. `build:boot`：`scripts/build-yunyou-boot.ts` 把 client 模块束注入
   `apps/web/dist/index.html` 并落 `dist/modules/` 与清单（boot 组装）。

产物只落 `frontend/apps/web/dist` 一处：Rust 侧 `crates/net/yunyou-web`
的 build.rs 在编译期直接消费该目录（include_bytes! 嵌入清单），无 crate 根
同步副本、无 rsync 复制链——dist 缺席时 cargo build 直接失败并提示先跑本链。
业主运行时零 node（见 crate build.rs embed_dist 与 src/dist.rs）。

图标再生成（云标 PNG 四张，零依赖脚本）：

```bash
pnpm exec tsx scripts/gen-yunyou-icons.ts
```

## 许可

- 上游 MIT 许可证随树保留：`LICENSE.upstream`（=上游根 LICENSE 原件）。
- 逐包 LICENSE/版权头未动；第三方 npm 依赖登记见 crate 根
  `../THIRD_PARTY_NOTICES.md`。
- 改动文件头注纪律：凡内容相对基线有改动的文件，头注写明
  「移植自 deepseek-harness@e8f7b93，改动：…」。
