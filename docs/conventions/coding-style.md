# 编码规范

**局部更新**：`set_row_data` > `set_vec` > `clear`；`rebuild_all` 仅初始化/重置用

**清空 VecModel**：用 `clear()`，不用 `set_vec(Vec::new())`

**Slint ↔ Rust**：直接用 Slint 生成枚举，不做 `i32` 中转（本次会话已全局替换动画编辑器部分）

**错误处理**：`log::warn!` + 提前 return

**分支风格**：优先 **卫语句（guard）+ 提前返回** 减少嵌套与认知负担；`if` 嵌套超过 2 层或 `else` 分支仅为 `return`/`continue` 时必须改为 `if !cond { return }`；与已有 `store/animation/graph/marquee.rs`、`store/sequence.rs`、`store/animation/nla/` 的早期返回风格保持一致

**函数长度**：像 `strip_dragged` 这类长函数尽量拆分为通用小函数（非强制，但超过 50 行或含多级嵌套时应拆分）；通用逻辑抽至 `store/utils/{marquee,selection,drag}.rs` 单一文件夹，供曲线/摄影表/时间线/NLA 复用，避免重复实现框选与拖动

**通用逻辑**：通用函数集中于 `store/utils/` 单一文件夹（`marquee`/`selection`/`drag`），新增功能前先在此查找是否已有通用函数（如 `normalize_time`/`clamp_rows`/`is_replacing`/`is_valid_delta`），避免重复造轮子；跨编辑器复用逻辑不得散落在各 `store/animation/*/marquee.rs` 等局部文件中

**回调命名**：`kebab-case`（`keyframe-pressed`）

**导入**：跨编辑器共享 → `core/types.slint`；编辑器专用 → `core/animation_types.slint`

**克隆审慎**：`clone` 大数据（`editing_keyframes`/`editing_strips`/`VecModel` 等）必须经过深思熟虑、确认无增量/最终校验等更轻量方案后才使用；优先增量回退（`prev_delta`）或最终偏移提交校验（同字幕行 `DragTimecodePreview/Commit` 与 Area 拆分合并）

**写文档**：只写全局验证过的内容，不猜测；不写会话日志（会膨胀）；保持简短
