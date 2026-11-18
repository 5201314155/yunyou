# grep

> 真源：`crates/tools/yunyou-tools/src/builtin/search.rs`。

用正则递归搜索文本文件，输出 `文件:行号:内容` 列表；二进制文件跳过；默认搜索工作区根，`subdir` 相对工作区根。不随 bash cd 漂移。

- **参数**：`pattern`（必填）；`subdir` 可选。
