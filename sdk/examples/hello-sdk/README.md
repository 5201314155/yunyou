# hello-sdk

yunyou-plugin-sdk 自举示例插件：问好（hello）与加法（add）两个工具。

## 构建

```bash
cargo build --release -p hello-sdk
# 产物：target/release/hello-sdk（独立可执行，零内核链接）
```

## 安装

```bash
yunyou plugin install crates/plugins/hello-sdk
# 自动归类落 <yunyou-home>/plugins/tools/hello-sdk/
```

## 验收全链（task-10 自举）

SDK 写 → 独立编译（内核零重编译）→ install → 内核扫描 spawn → 工具注册 →
真调用成功。文档=SDK crate 的 SDK.md。
