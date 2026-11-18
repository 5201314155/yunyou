# write

> 真源：`crates/tools/yunyou-tools/src/builtin/fs_tools.rs`。

把文本写入项目内的文件（覆盖写）。父目录必须已存在——缺失即报错，不会自动创建（先建目录或用 bash mkdir 后重写）。相对路径基于工作区根解析，不随 bash cd 漂移；结果回显绝对路径。

- **参数**：`path`、`content`（必填）。
