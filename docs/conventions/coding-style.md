# 编码规范

**局部更新**：`set_row_data` > `set_vec` > `clear`；`rebuild_all` 仅初始化/重置用

**清空 VecModel**：用 `clear()`，不用 `set_vec(Vec::new())`

**Slint ↔ Rust**：直接用 Slint 生成枚举，不做 `i32` 中转（本次会话已全局替换动画编辑器部分）

**错误处理**：`log::warn!` + 提前 return

**回调命名**：`kebab-case`（`keyframe-pressed`）

**导入**：跨编辑器共享 → `core/types.slint`；编辑器专用 → `core/animation_types.slint`

**写文档**：只写全局验证过的内容，不猜测；不写会话日志（会膨胀）；保持简短
