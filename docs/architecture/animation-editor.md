# 动画编辑器

**状态**：✅ 已接入依赖图（数据层/派生层分离，对齐 LineEditor 管线）；曲线编辑器 + 摄影表（Dope Sheet）

**架构**：
```
AnimationCallbacks → AnimationAction → AppStore::dispatch_animation（reducer，只改数据层）
  → 返回精确 tag → depsgraph → op_animation_*（投影，更新派生层）
```

**两层状态**（`src/store/graph_editor/`，即图编辑器）：
- **数据层**（reducer 可写）：`channels_data`（通道属性）、`editing_keyframes`（关键帧数据）、
  选中集与 UI 态（selected_handles/搜索/缩放/锚点/pending 选择）
- **派生层**（只由投影写）：`channels`/`keyframe_models`/`extrapolations`（Slint 绑定模型）、
  `segments`/`handles`/`dope_connections`/`summary_keyframes`/`selected_display`/
  `preview_handles`/`action_start`/`action_end` 与 Slint 标量

**位置/拖拽上下文类型**（元组语义化）：
- `HandleLoc{curve, key, is_in}`（depsgraph/tag.rs）——selected_handles、handle_preview 等
- `KeyframeLoc{curve, key}`（graph_editor）——关键帧锚点/重合点
- `SummaryDrag{index, delta_time}`（depsgraph/tag.rs）——汇总拖拽
- `KeyframeUpdate`/`KeyframeTimeUpdate`——reducer 收集后统一写回

**Tag 细化**（`depsgraph/tag.rs`）：
- `AnimationStructureChanged` — 装载/重置/搜索（全量重建 + 过滤视图重绑定）
- `AnimationKeyframesChanged{curves, summary_ms, dragged_curve, dragged_summary, handle_preview}` — 拖拽/插值/手柄
- `AnimationSelectionChanged{summary_ms}` — 关键帧/通道选中（纯选中视觉）
- `AnimationDragEnded` — 拖拽结束整理（预览残留清理/重排/补柄），与选中分离——
  keyframe_released/handle_released（moved=true）打它，其他组件（如 NLA）可单独依赖
- `AnimationChannelChanged{curve}` — 可见/激活/可编辑/颜色模式
- `AnimationScalarsChanged` — 播放头/标量

**投影**（`projections/animation.rs` + `graph_editor/projection.rs`，每个 apply_* 按职责拆单一目的子函数）：
- `op_animation_structure` → apply_structure（重建 keyframe_models/channels 行/filters/派生模型/外推/Action 菜单）+ 重新绑定过滤视图到 Slint
- `op_animation_keyframes` → apply_keyframes（sync_keyframe_model + rebuild_curve + update_extrapolation + 汇总/预览子函数）
- `op_animation_selection` → apply_selection（纯选中同步：sync_channel_rows / sync_selected_display / sync_keyframe_selection / update_handle_selection_visual / update_summary_entries）
- `op_animation_drag_ended` → apply_drag_ended（restore_handle_preview（关键帧拖拽的 preview 替换恢复普通手柄）/ resort_keyframes / ensure_default_handles（拖拽结束后为贝塞尔关键帧补缺失侧默认水平柄——边缘延伸是水平的，默认柄水平延伸 1/2 间距；拖拽中不补柄，仅结束整理））
- `op_animation_channel` → apply_channel（通道行/段手柄颜色/连接线/汇总）
- `op_animation_scalars` → 同步标量（handles 全部动画 kind）

**拖拽语义**（对齐 LineEditor preview/commit）：
- 拖拽（高频）只改数据层 + Keyframes tag，投影同帧同步嵌套模型（set_row_data 保住 grab）
- 外推线：`extrapolations`（排序后首末）由投影每帧更新——拖拽期间实时切换新首末
  （关键帧模型保序，Slint 外推线读 extrapolations 而非 keyframes[0]/[len-1]）；
  松开后 `resort_keyframes` 重排乱序通道恢复模型有序
- 汇总标记拖拽：`dragged_summary` 就地更新时间（不 insert/remove，保住 grab）
- 手柄拖拽：`handle_preview`（被拖+同步+对向柄列表）→ 投影重建 `preview_handles`；
  交互终止点（选中变化）清空 preview_handles；手柄拖拽结束不打 DragEnded
  （手柄可能被故意钳到 0 长度，ensure 不该覆盖）
- 松开/插值变更后 `write_view_to_model` 写回 model Action

**摄影表（Dope Sheet）**（`ui/areas/dope_sheet/`，与曲线编辑器共用数据/选中，拖拽只调时间）：
- 布局参照 Blender：左侧 `ChannelPanel`（搜索框 + 通道列表 + 汇总标题行）+ 顶部 `TimeRuler`
  + 主画布（每通道一行按时间铺开关键帧）；滚轮仅横向缩放（`CanvasNavigation.lock-y-zoom`）、
  中键双向平移、列表滚轮与画布 pan-y 共用同一 clamp 范围
- 交互回调：`dope_keyframe_pressed/dragged/released`（内部即曲线编辑器 `keyframe_*` 同一条
  路径，`dv=0`）、`dope_marquee_select`（时间范围 × 行区间，行 → Rust 映射真实通道）、
  `dope_summary_pressed/dragged/released`（汇总条带跨通道同步拖拽）
- 搜索独立：`dope_search` 只过滤 `dope_filter`（FilterModel），与 `graph_filter` 互不干扰；
  搜索框聚焦走全窗口编辑遮罩（EditOverlayState 挖孔定位）
- 汇总条带交互路径（`summary.rs`）：按下即写回通道选中（汇总标记独立于 channels 嵌套循环，
  按下期间更新安全）；拖拽返回 `AnimationKeyframesChanged{dragged_summary}`，投影就地更新
  被拖标记 time（不 insert/remove，保住 grab）；松开返回 `AnimationSelectionChanged`（现有
  条目 ms ∪ 关键帧当前 ms 的并集，逐 ms 精确重建 + 移除空条目，不整体 set_vec）+
  `AnimationDragEnded`（重排乱序 + 补柄）
- 拖拽期间只有被拖标记就地移动，各 ms 条目 count/掩码陈旧——由松开时的并集重算收敛

**已知 TODO**：调色盘（channel.rs）/ NLA
