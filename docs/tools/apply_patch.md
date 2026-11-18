# apply_patch

> 真源：`crates/tools/yunyou-tools/src/builtin/patch_tool.rs`。

跨文件/同文件多区域/含新增删除移动的批量修改：提交 `*** Begin Patch…*** End Patch` 格式补丁（Add File 内容行 + 前缀；Update File 配 @@ 锚点与 -/+ 行；可 Move to 改名）。全部 hunk 先预检——任一不匹配整体拒绝、零副作用；通过后按序落盘，部分失败如实记账不回滚。结果回显绝对路径。单点小改用 edit。

- **参数**：`patch`（必填，含 Begin/End 包裹）。
