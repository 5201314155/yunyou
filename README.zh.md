# 云悠（YunYou）

> [English](README.md) | 简体中文

用 Rust 写的终端 AI Agent：组件内核 + 全插件化主板。对话、工具调用、会话持久化、多进程团队协作、沙箱，一切能力插在内核缝上。

> 渊源：参考 DeepSeek Harness（dsh）的架构思想开发的 Rust 版本 Agent。
> **开源仓库**：https://github.com/5201314155/yunyou（安装脚本与 Release 二进制同源）


## 架构全景（主板-插槽-数据流）

基座=主板，插件=可换的卡：内核只提供插槽规格（契约发布即冻结、版本化演进），一切能力以插件形态插在缝上——对话、工具、会话、沙箱、团队协作皆插件，拔掉任何一张卡内核照活。

```
用户输入 → agent-loop（对话循环，取消/重试/步数护栏）
         → 工具执行（read/write/edit/bash + 路径闸 + Landlock/seccomp 沙箱）
         → 会话 JSONL 落盘（重启恢复/多开互斥）
         → 渲染层（流式增量/工具卡片/树状归集/紧凑落款）
```

crate 分区（17 现状，规划见蓝图终稿 §六）：

| 分区 | crate | 职责 |
| --- | --- | --- |
| 主板内核 | `yunyou` | Context/Fiber 状态机/事件五分发/插件生命周期/manifest 声明面 |
| 共享工具 | `yunyou-util` | BoxFut/深比较/LEB128 |
| 组件层（bundled 插件） | `llm` `llm-openai` `session` `tools` `system-prompt` `agent-loop` `config` `credentials` `retry` `commands` | LLM 契约与实现/会话持久化/工具执行/提示词组装/主循环/配置/凭据/重试策略/命令注册缝 |
| 壳与前端 | `yunyou-cli`（bin `yunyou`）`yunyou-web` `frontend/` | 终端 REPL/渲染/命令面板；v2 官方 UI 与前端 monorepo |
| 规划中 | rpc-types/webserver/web-shell/sandbox/gateway/launcher/procmgr/worker-core | 蓝图终稿 §六 终局清单（随里程碑落位） |

插件作者入口：`docs/接口/`（message/sandbox/manifest/tools/commands 五契约规格+示例）+`.yunyou/skills/yunyou-plugin-dev.md` 技能。

## 一条命令安装（全局命令）

```bash
curl -fsSL https://github.com/5201314155/yunyou/raw/main/install.sh | sh
```

安装器行为：优先下载 GitHub Release 的预编译二进制（**零 Rust 工具链**）→
没有预编译包则回退 `cargo install --git`（源码直装）→ 自动补 PATH。

升级重跑同一条命令即覆盖；卸载 `rm ~/.local/bin/yunyou`。

## 快速开始

```bash
# 构建（需要 Rust 1.75+）
cargo build --release -p yunyou-cli

# 零配置启动（不写任何配置文件也能跑：内置默认值+环境变量覆盖）
./target/release/yunyou

# 指定配置启动
./target/release/yunyou --config /path/to/yunyou.toml

# 指定根目录（配置+会话落点；缺省 $YUNYOU_HOME，再缺省 ~/.yunyou）
./target/release/yunyou --home /path/to/home
```

配置文件查找顺序：`--config` 显式指定（文件必须存在，否则报错）→ `<yunyou-home>/yunyou.toml`（存在则加载）→ 都没有则全默认配置启动。

## 能力速览（全部插件形态，/plugins 可见可开关）

- **工具**：内置六件 + SDK 外部插件（社区）
- **人设**：`.yunyou/persona.md` 改文件即生效
- **技能**：`.yunyou/skills/*.md` 自动进目录段，模型按需自读（渐进披露）
- **权限**：四模式（default/acceptEdits/plan/bypass）+工作区边界+高危拦截（工作流插件）
- **定时**：模型对话中自选时间（schedule_create，调度插件）
- **模式**：/mode plan|work|creation（计划审批/默认/创造模式——不开创造模式，AI 无插件开发能力）
- **插件管理**：`yunyou plugin new|check|install|outdated|update|remove`（三来源更新+契约兼容闸）

## 写一个插件（五分钟）

插件=用 [`yunyou-plugin-sdk`](crates/plugins/yunyou-plugin-sdk/SDK.md) 写的**独立可执行文件**——零内核依赖，`cargo add yunyou-plugin-sdk` 即开工，编译产物被云悠 spawn 为子进程（崩溃隔离：插件死≠主板死）。

```bash
# ① 脚手架：manifest.toml+模板 main+README 一键生成（分类目录约定自动含）
yunyou plugin new my-plugin --category tools

# ② 写工具逻辑（src/main.rs）：参数 schema 自动进模型面（LLM 照 schema 填参）
cd my-plugin && $EDITOR src/main.rs && cargo build --release

# ③ 安装：按 manifest.toml 的 id+category 自动归类落 <home>/plugins/<类>/<id>/
yunyou plugin install .

# ④ 重启云悠 → /plugins 可见可开关 → 模型直接调用你的工具
```

最小插件全文（src/main.rs）：

```rust
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use yunyou_plugin_sdk::{tool, Plugin};

#[derive(Deserialize, JsonSchema)]
struct EchoArgs {
    /// 要回显的文本
    text: String,
}

fn main() -> std::process::ExitCode {
    Plugin::new("echo", "0.1.0")
        .category("tools")
        .tool(tool("echo", "原样返回输入文本", |args: EchoArgs| {
            Ok(json!({ "echoed": args.text }))
        }))
        .serve()
}
```

完整文档：[`crates/plugins/yunyou-plugin-sdk/SDK.md`](crates/plugins/yunyou-plugin-sdk/SDK.md)（15 分钟上手，插件作者唯一读物）；示例：[`crates/plugins/hello-sdk/`](crates/plugins/hello-sdk/)。

## 配置文件全解（yunyou.toml）

配置文件是 **TOML** 格式，所有段都可省略（省略=用默认值）。拼错键名会**启动即报错**（deny_unknown_fields，不静默吞）——这是刻意设计。

下面是一份带注释的完整示例（各字段默认值以 `=` 后标注）：

```toml
# ── LLM 端点（必看段）─────────────────────────────
[llm]
# OpenAI 兼容端点（/v1 结尾）
base_url = "http://127.0.0.1:8317/v1"          # 默认本地 8317
# API key，两种写法：
#   1) 明文（不推荐，配置文件会存密钥）
#   2) 凭据引用（推荐）：写 "$YUNYOU_LLM_API_KEY"，
#      真实值放凭据存储文件（见下文「凭据」），配置文件不落密钥
api_key = ""
# 模型名（OpenAI 兼容路由格式）
model = "z-ai/glm-5.3-flash"
# 请求超时秒数
timeout_secs = 300
# 流式空闲超时毫秒（超过无增量即断流报错）
stream_idle_timeout_ms = 30000

# ── 会话持久化 ────────────────────────────────────
[session]
# 会话 JSONL 落盘路径；缺省 <yunyou-home>/sessions/ 下自动管理
# path = "/home/me/.yunyou/sessions/chat.jsonl"

# ── 工作区（文件工具的根）─────────────────────────
[workspace]
# 文件读写工具的根目录（越界访问被拒绝）；缺省当前目录
# root = "/home/me/project"

# ── 人设 ─────────────────────────────────────────
[system_prompt]
# 系统提示词人设（会拼进系统提示词段）
# persona = "你是一个严谨的运维助手"

[agent_loop]
# agent_loop 段也可配人设（与 system_prompt.persona 的关系见插件侧文档）
# persona = "……"

# ── 人设文件（推荐：改文件即生效，零代码零配置）──
# AGENTS.md 四级分源（行业标准文件名，全面对齐 claude 且更好）：
#   /etc/yunyou/AGENTS.md            策略级（组织下发，可选）
#   ~/.yunyou/AGENTS.md              用户全局
#   <项目>/AGENTS.md                 项目级（随仓库走）
#   <项目>/.yunyou/local/AGENTS.md   本地私有（不入库）
# 注入=按层序拼接（后层可见前层，冲突以后层为准）；同规则多层重复会在
# 提示词尾人话点名（冲突检测，claude 是静默覆盖）；每层带 [agents:层名] 锚。
# 优先级：AGENTS.md 链 > persona.md（.yunyou/persona.md 项目级 > ~/.yunyou/persona.md
# 用户级）> 上面两处 toml 字符串配置。md 内容支持 `#` 标题分段；空文件视为不存在
# 继续回落。改人设 = 改一个 md 文件。
# 单工具执行超时秒数（0 = 不限时）
tool_timeout_secs = 300
# 工具循环步数护栏（默认不设=跑到任务完成为止；显式设值=跑满即暂停可「继续」续跑，非腰斩）
# max_steps = 50
# 本轮 token 预算护栏（默认不设；usage 缺报的端点此护栏不生效=诚实不估算）
# token_budget = 100000

# ── 工具开关（逐个关内置工具）─────────────────────
# [tools.builtin]：工具名→bool，缺省全开。false = 该工具不注册、
# 不进模型面（LLM 看不见=真关，非仅禁执行）。可用名：bash、apply_patch、
# read、write、edit、glob、grep；写未知名启动即报错（拼错防呆）。
# [tools.builtin]
# bash = false                  # 例：禁掉 bash，只留文件工具
# grep = false                  # 例：禁掉 grep


# ── 重试策略 ─────────────────────────────────────
[retry]
max_retries = 3                # 最大重试次数
# retryable_codes = ["429", "500", "502", "503"]   # 可重试 HTTP 码白名单
initial_delay_ms = 500         # 首次退避
max_delay_ms = 8000            # 退避上限
jitter_ratio = 0.2             # 退避抖动比例
retry_on_timeout = true        # 超时是否重试

# ── 凭据服务 ─────────────────────────────────────
[credentials]
# 凭据存储文件（600 权限，缺省 <yunyou-home>/credentials.toml）
# store_path = "credentials.toml"

# ── 沙箱（Linux Landlock+seccomp）────────────────
[sandbox]
enabled = true                 # 沙箱总开关
writable_tmp = true            # 允许写 /tmp（编译器/管道临时文件）
network_denied = true          # 禁断子进程网络（默认禁）

# ── 插件 ────────────────────────────────────────
# [plugins.<插件id>] 段：enable 控制开关，config 原样透传给插件
# [plugins.my-plugin]
# enable = true
# [plugins.my-plugin.config]
# some_key = "value"
```

### 环境变量覆盖（不写配置文件的最简方式）

以下三个环境变量存在即覆盖配置文件（最简启动路径）：

```bash
export DSH_TEST_LLM_BASE_URL="https://api.example.com/v1"
export DSH_TEST_LLM_API_KEY="sk-..."
export DSH_TEST_LLM_MODEL="vendor/model-name"
./yunyou
```

> 注：变量名以 `DSH_TEST_` 开头是历史遗留（联调验收期定名），语义是正式的 LLM 覆盖，非仅测试用。

## 凭据（API key 推荐存法）

配置文件里写 `api_key = "$YUNYOU_LLM_API_KEY"` 这种 `$NAME` 形式 = 凭据引用，真实值从凭据存储解析，**配置文件本身不含密钥**：

```toml
# ~/.yunyou/credentials.toml（自动 600 权限；YUNYOU_LLM_API_KEY 是约定键名）
YUNYOU_LLM_API_KEY = "sk-你的真实key"
```

- 引用解析顺序：凭据存储 → 环境变量 → 缺失报错（不假装有值）。
- `$PATH` 这类任意环境变量名也能被解析——别在配置里写有歧义的引用名。

## 常用操作

REPL 内斜杠命令（以 `/` 开头，补全面板可用）：

| 命令 | 作用 |
|---|---|
| `/help`（`/h`） | 命令列表 |
| `/new` | 开新会话 |
| `/sessions` | 会话列表/切换/回放 |
| `/plugins` | 插件状态表 |
| `/quit`（`/q`） | 退出 |

> 插件可注册自己的命令（commands.v1 契约），装了新插件这里会长出新命令。

非交互单任务：`yunyou "把 foo.rs 里的 TODO 清一遍"`。

## 沙箱说明（Linux）

启用后 bash 子进程跑在 Landlock（文件系统白名单）+ seccomp（网络禁断）里：

- 工具写文件只能落在 workspace root 与 /tmp（writable_tmp=true 时）；
- network_denied=true 时子进程不能联网（`curl` 等会失败——真实失败透传给模型，不是沙箱故障）；
- 非 x86_64 架构目前 fail-closed（沙箱不可用即拒绝执行，不裸奔）。

## 目录结构

```
~/.yunyou/
├── yunyou.toml        # 主配置（本文件上方全解）
├── credentials.toml   # 凭据存储（600）
└── sessions/          # 会话 JSONL
```

## 从源码开发

```bash
cargo build --workspace          # 全量构建
cargo test --workspace           # 全量测试（含 tests/ 集成目标）
cargo clippy --workspace --all-targets
```

架构与贡献规范见 `docs/`（组件内核方案/插件契约/接口规格）与 `AGENTS.md` 类文档；插件开发从 `.yunyou/skills/yunyou-plugin-dev.md` 入手。

## 参与贡献与路线图

- 贡献规则与审查流程：[CONTRIBUTING.md](CONTRIBUTING.md)（内核不接受随意 PR——功能一律插件形态进 `plugins/<id>/`）。
- 路线图：[ROADMAP.md](ROADMAP.md)（M2 内核 → M3 地基封板 ✅ → M4 UI 缝二期 → M5 market 子进程协议 → M6 团队运行时 → M7 开源基建）。
- 插件开发从 `.yunyou/skills/yunyou-plugin-dev.md` 技能文档入手。

## License

MIT（见 THIRD_PARTY_NOTICES.md 的第三方依赖许可底册）。
