# read

> 真源：`crates/tools/yunyou-tools/src/builtin/fs_tools.rs`。

读取项目内文本文件，返回带行号的内容；相对路径基于工作区根解析（git 项目根；非 git 目录=启动目录；可 `[workspace] root` 配置指定），不随 bash cd 漂移；需要 bash 当前目录时先用 bash pwd 取绝对路径。

- **参数**：`path`（必填）。
- **越界**：解析后必须仍在工作区根内（含 symlink 归一），越界拒绝。
