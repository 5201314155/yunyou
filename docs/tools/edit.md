# edit

> 真源：`crates/tools/yunyou-tools/src/builtin/fs_tools.rs`。

把文件中唯一出现的 old 文本替换为 new；出现零次或多次都会报错。相对路径基于工作区根解析，不随 bash cd 漂移；结果回显绝对路径。成功返回 `{replaced, path, old, new}`（供 CLI diff 详情卡渲染）。多区域/跨文件批量修改用 apply_patch。

- **参数**：`path`、`old`、`new`（必填）。
