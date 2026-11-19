# yunyou-plugin-sdk：15 分钟写一个云悠插件

> 插件作者唯一依赖。你不需要读 yunyou 内核的任何代码或文档——本文+`examples/`
> （或仓库内 `sdk/examples/hello-sdk/`——**仓库内自举测试件**：作者参考其 main.rs 写法，**不要拷它的 Cargo.toml**——那是仓库内部布线的 path 依赖，与你的包无关，见下「依赖来源」）就是全部材料。

## 你将得到什么

写一个 Rust 程序：声明若干**工具**（LLM 可调用的能力），`cargo build` 出一个
**独立可执行文件**。把它放进云悠的插件目录，云悠启动时会把它作为**子进程**
拉起，通过 stdin/stdout JSON Lines 协议对话：握手→上报工具清单→转发工具调用。
插件崩溃不影响主板（一插件一进程，崩溃隔离）。

## 最小插件全文（可直接抄）

```rust
// src/main.rs
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
        .description("回显文本的示例插件")
        .category("tools")
        .tool(tool("echo", "原样返回输入文本", |args: EchoArgs| {
            Ok(json!({ "echoed": args.text }))
        }))
        .serve()
}
```

`Cargo.toml`：

```toml
[package]
name = "echo-plugin"
version = "0.1.0"
edition = "2021"

[dependencies]
yunyou-plugin-sdk = "0.1"
schemars = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`manifest.toml`（与可执行文件同放插件目录）：

```toml
id = "echo"
version = "0.1.0"
format = 2
level = "market"
contracts = ["tools.v1"]
category = "tools"
auto_enable = true

[runtime]
type = "subprocess"
entry = ["echo-plugin"]        # 相对插件目录；即你 cargo build 的产物名
```

## 依赖来源（外部作者必读）

你的插件 Cargo.toml 里 SDK 依赖只有两种合法形态：

```toml
# ① git 形态（发布 crates.io 前的过渡态，脚手架默认生成）
yunyou-plugin-sdk = { git = "https://gitee.com/yiyuai/yunyou.git" }

# ② 版本号形态（SDK 发布 crates.io 后）
yunyou-plugin-sdk = "0.1"
```

**严禁 `path = "..."` 指向云悠内核源码树**——外部开发者没有（也不需要）内核源码，
path 形态只存在于我们仓库内部的测试布线（hello-sdk 示例的 Cargo.toml），与你的包无关。
脚手架（`yunyou plugin new`）生成的模板已是 git 形态，CI 断言锁死（生成包含 path 依赖=红）。

## 插件作者须知：Cargo 依赖安全冷却期（Rust 1.100+）

Rust 1.100 起引入 **security cooldowns**：crates.io 对「刚发布的新版本依赖」有
安全冷却窗口（新包/新版本发布后短期内有被下架或发现恶意行为的可能，冷却期
内构建会提示或延迟取用——以 cargo 当期文档为准）。对插件作者的建议：

- 插件 CI 开启 `cargo update` 后的冷却期检查（新依赖先在分支观察再合主）；

- 避免紧追「发布数小时内」的热门依赖新版本（下架窗口高发段）；

- 云悠插件安装器已把「冷却期开关」列入远期可选（安装时可选延迟采用新依赖，
  候 cargo 机制稳定后跟进）。

（情报源：业主 web_search 实搜的 Rust 1.100 简报；采纳清单见
docs/设计/rust-1.100-生态采纳清单-20261119.md）

## 三步装进云悠

```bash
cargo build --release
yunyou plugin install ./target/release/echo-plugin 所在目录   # 或手工拷贝
# 或从零开始：yunyou plugin new my-plugin --category tools 生成脚手架
```

重启云悠：插件被扫描、spawn、握手，工具出现在模型面；`/plugins` 可见可开关。

## API 逐个说

### `Plugin::new(id, version) -> Plugin`

构建器入口。`id`=全局唯一插件标识（manifest.toml 的 `id`，两处须一致）；
`version`=semver 字符串。

### `.category(类)`

分类目录约定（install 自动归类落 `plugins/<类>/<id>/`；如 `tools`、`integrations`、
`workflows`、`ui`）。须与 manifest.toml 的 `category` 一致。

### `.tool(...)` 与 `tool(name, description, handler)`

注册一个工具。便捷函数 `tool` 接受一个**强类型闭包**：

```rust
tool("工具名", "给 LLM 看的一句话描述", |args: MyArgs| -> Result<Value, ToolFailure> { ... })
```

`MyArgs` 须 `Deserialize + JsonSchema`——参数 JSON Schema **自动派生**进模型面，
LLM 照 schema 填参，SDK 反序列化后交给你。字段 doc-comment 会成为 schema 的
字段描述（给 LLM 的参数说明就写在这里）。

也可以手写裸形态（完全掌控 schema）：

```rust
struct MyTool;
impl yunyou_plugin_sdk::Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "..." }
    fn parameters(&self) -> String { r#"{"type":"object"}"#.into() }
    fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolFailure> { ... }
}
```

### `ToolFailure { code, message, hint }`

工具失败的人话三元组（code 沿 tools.v1 错误码语义，如 `"tool/invalid-args"`）；
`hint` 可选修复指路。返回它 = 模型面收到结构化失败，可自纠重试。

### `.serve()`

阻塞当前线程跑协议服务（stdin/stdout JSON Lines）：

```
宿主 ──hello(协议版本)──────────▶ 插件
     ◀──hello_ok（或 hello_err=版本不符退出）
     ──tools_list──────────────▶
     ◀──tools_list_ok（工具清单+schema）
     ──tool_invoke(invoke_id,name,args)─▶
     ◀──tool_result(invoke_id, ok|err)
     ──shutdown────────────────▶ 插件退出（宿主兜底强杀）
stdin EOF（宿主退出）＝插件退出
```

诊断信息一律写 **stderr**（宿主逐行前缀透传主板日志）；stdout 是协议专用通道，
禁打印任何非协议内容。

## 生命周期与纪律

- **一插件一进程**：你的进程死了，主板活着；调用中死亡=调用方收到
  `tool/process-crashed`，宿主侧可 restart 复活。
- **MVP 能力边界**（协议词汇未开闸=不可用，诚实告知）：无凭据访问
  （`permissions.credentials` 非空会被拒装）、无网络放行（`permissions.network=true`
  拒装）、无热更新（改文件重启生效）、无取消语义（单请求靠超时硬断）。
- **写权限**：默认只有插件目录自身可写（`$SELF`）；需要更多写根在 manifest.toml
  `[permissions] writable` 里声明（绝对路径）。
- **未知 manifest 键/未知协议帧一律拒绝**（fail-loud 防呆），配置拼错启动即报。

## 能力覆盖矩阵（12 条契约缝：能写什么、状态、边界）

> 痛点直答：**不用翻内核源码**——本矩阵+§API 逐个说=全部。「可写」=现在就能写
> 并真调用；「排队」=协议词汇未开闸（写了会被拒装或词汇不存在），开闸随 L2.1。

| # | 契约缝 | 能写什么插件（hello-sdk 级最小示例名） | 状态 | 权限边界 |
| --- | --- | --- | --- | --- |
| ① | 工具注册 | **工具类插件**：`hello`（问好）、`weather-query`（读本地缓存天气）、`sqlite-query`（本地库检索）、`image-resize`（本地图片处理）——一切「收参数→本地计算→返回 JSON」的能力 | **可写** | 写权限仅插件目录（$SELF）+manifest `permissions.writable` 声明项；无凭据、无网络 |
| ② | 工具→模型桥 | 零工作：工具清单经宿主自动进模型面（schemas()），作者只写 name/description/参数 schema | **自动** | 模型面只见三元组（name/description/parameters），timeout 等零泄漏 |
| ③ | 系统提示段 | 提示词段落贡献类（如领域话术注入） | 排队（命令/提示段词汇 L2.1） | — |
| ④ | 配置 schema 投影 | 需用户配置的插件（设置表单自动生成） | 排队（配置面词汇 L2.1） | — |
| ⑤ | 凭据服务 | 需密钥的插件（调第三方 API） | **排队·拒装**：manifest `permissions.credentials` 非空=拒装「凭据面未开放」 | L2.1 开闸后：声明名单即授权边界，审计留痕 |
| ⑥ | 输出渲染钩 | 工具卡片自定义渲染 | 排队（L2.1） | — |
| ⑦ | 生命周期 | 宿主管：spawn/握手/失败 Failed/restart 复活/卸载 killpg——插件无需也无法干预 | **自动** | 插件进程死亡≠主板死；调用中死亡=调用方收 `tool/process-crashed` |
| ⑧ | 事件面 | 事件订阅/发布类插件 | 排队（事件词汇 L2.1） | — |
| ⑨ | 工作区授权 | 越界写文件的插件 | **恒最小**：可写根=插件目录+$SELF+声明项；network 恒 false（声明 true=拒装） | 沙箱 Landlock+seccomp 强制，声明即边界 |
| ⑩ | bundle 补丁（tgz 安装） | 分发市场插件 | 排队（L2.2 市场/签名） | — |
| ⑪ | 目录元数据 | 多模型插件目录 | 预留位 | — |
| ⑫ | 后台 job 事件 | 长任务异步插件 | 排队（首个异步长任务插件立项时建） | — |

**一句话结论：L2-MVP 你能写的=工具类插件（①），其余缝全部排队**——这正是
hello-sdk 自举示例的形态。协议词汇封闭枚举（八变体），私加消息类型双端都过
不了解码=fail-loud，演进=设计修订不是约定。

## 发布前 checklist（装前自查）

```bash
yunyou plugin check <插件目录>
```

逐项 ✓/✗ 清单：manifest 五类权威校验/分类白名单/依赖形态（禁 path 指向内核源码树）/SDK 契约版本匹配/工具名撞名预警——全部 ✓ 再 `plugin install`（install 用同一套校验，check 只是让你提前看到）。装前四步：①cargo build --release 成功；②plugin check 全绿；③README 写清用途；④manifest 的 category 正确（install 按它归类）。

## 下一步

> 配置级安装（bundled 投影/开关，零编译 walkthrough）见 `docs/设计/插件开发指南.md`（L1 篇）。
- 抄 `hello-sdk` 示例改造成你的第一个插件；
- 读 SDK crate 内 `tests/` 与 `SDK.md`（即本文）；
- 技能文档《yunyou-plugin-dev》SDK 章有进阶问答。
