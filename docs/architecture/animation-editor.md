# 动画编辑器

**状态**：✅ 已接入依赖图（数据层/派生层分离，对齐 LineEditor 管线）

**架构**：
```
AnimationCallbacks → AnimationAction → AppStore::dispatch_animation（reducer，只改数据层）
  → 返回精确 tag → depsgraph → op_animation_*（投影，更新派生层）
```

**两层状态**（`src/store/curve_editor/`）：
- **数据层**（reducer 可写）：`channels_data`（通道属性）、`editing_keyframes`（关键帧数据）、
  选中集与 UI 态（selected_handles/搜索/缩放/锚点/pending 选择）
- **派生层**（只由投影写）：`channels`/`keyframe_models`（Slint 绑定模型）、`segments`/
  `handles`/`dope_connections`/`summary_keyframes`/`selected_display`/`preview_handles`/
  `action_start`/`action_end` 与 Slint 标量

**Tag 细化**（`depsgraph/tag.rs`）：
- `AnimationStructureChanged` — 装载/重置/搜索（全量重建 + 过滤视图重绑定）
- `AnimationKeyframesChanged{curves, summary_ms, dragged_curve, dragged_summary}` — 拖拽/插值/手柄
- `AnimationSelectionChanged{summary_ms}` — 关键帧/通道选中
- `AnimationChannelChanged{curve}` — 可见/激活/可编辑/颜色模式
- `AnimationScalarsChanged` — 播放头/标量

**投影**（`projections/animation.rs` + `curve_editor/projection.rs`）：
- `op_animation_structure` → apply_structure（重建 keyframe_models/channels 行/filters/派生模型/Action 菜单）+ 重新绑定过滤视图到 Slint
- `op_animation_keyframes` → apply_keyframes（嵌套模型 diff 同步 + rebuild_curve（数量不变 set_row_data / 变化 splice）+ 汇总增量）
- `op_animation_selection` → apply_selection（selected_display/嵌套 selected/手柄选中/汇总掩码）
- `op_animation_channel` → apply_channel（通道行/段手柄颜色/连接线/汇总）
- `op_animation_scalars` → 同步标量（handles 全部动画 kind）

**拖拽语义**（对齐 LineEditor preview/commit）：
- 拖拽（高频）只改数据层 + Keyframes tag，投影同帧同步嵌套模型（set_row_data 保住 grab）
- 汇总标记拖拽：`dragged_summary` 就地更新时间（不 insert/remove，保住 grab）
- 松开/插值变更后 `write_view_to_model` 写回 model Action
- `preview_handles`（手柄软钳制预览）依赖拖拽上下文，reducer 直写（对齐 OverlayState 语义）

**已知 TODO**：调色盘（channel.rs）/ Dope Sheet UI / NLA
