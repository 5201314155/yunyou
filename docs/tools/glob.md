# glob

> 真源：`crates/tools/yunyou-tools/src/builtin/search.rs`。

按 glob 模式（如 `src/**/*.rs`）列出文件，返回相对工作区根的路径列表；模式基于工作区根解析，不随 bash cd 漂移。

- **参数**：`pattern`（必填）。
