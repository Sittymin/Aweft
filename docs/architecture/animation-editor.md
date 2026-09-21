# 动画编辑器

**状态**：✅ 已接入依赖图（数据层/派生层分离，对齐 Sequence 管线）；曲线编辑器 + 摄影表（Dope Sheet）+ NLA 编辑器（统一 DataId::Animation，Graph/NLA TagKind 隔离）

**架构**：
```
AnimationCallbacks → AnimationAction::Graph/Nla → AppStore::dispatch_animation（统一动画域，子路由 Graph/Nla）
  → 返回精确 tag（Graph* / Nla*）→ depsgraph 单 Component（DataId::Animation，13 ops）→ op_graph_*/op_nla_*（投影，更新派生层）
```

**两层状态**（`src/store/animation/graph/` 图编辑器 + `src/store/animation/nla/` NLA，聚合于 `AnimationState{graph,nla}`）：
- **数据层**（reducer 可写）：`graph.channels_data`/`editing_keyframes`、选中集与 UI 态；`nla.track_view`/`editing_strips`
- **派生层**（只由投影写）：`graph.channels`/`keyframe_models`/`extrapolations`/`segments`/`handles`/`dope_connections`/`summary_keyframes`/`selected_display`/`preview_handles`/`action_start`/`action_end`；`nla.tracks`/`tracks_filter`/`strip_models`

**位置/拖拽上下文类型**（元组语义化）：
- `HandleLoc{curve, key, is_in}`（depsgraph/tag.rs）——selected_handles、handle_preview 等
- `KeyframeLoc{curve, key}`（graph_editor）——关键帧锚点/重合点
- `SummaryDrag{index, delta_time}`（depsgraph/tag.rs）——汇总拖拽
- `KeyframeUpdate`/`KeyframeTimeUpdate`——reducer 收集后统一写回

**Tag 细化**（`depsgraph/tag.rs`）：
- `GraphStructureChanged` — 装载/重置/搜索（全量重建 + 过滤视图重绑定）
- `GraphKeyframesChanged{curves, summary_ms, dragged_curve, dragged_summary, handle_preview}` — 拖拽/插值/手柄
- `GraphSelectionChanged{summary_ms}` — 关键帧/通道选中（纯选中视觉）
- `GraphDragEnded` — 拖拽结束整理（预览残留清理/重排/补柄），与选中分离——
  keyframe_released/handle_released（moved=true）打它，其他组件（如 NLA）可单独依赖
- `GraphChannelChanged{curve}` — 可见/激活/可编辑/颜色模式
- `GraphScalarsChanged` — 播放头/标量

**投影**（`projections/animation/graph.rs` + `store/animation/graph/projection.rs`，每个 apply_* 单职责；`projections/animation/mod.rs` 重导出；`projections/mod.rs` 单 Component 注册 13 ops）：
- `op_graph_structure` → apply_structure（重建 keyframe_models/channels 行/filters/派生模型/外推/Action 菜单）+ 重新绑定过滤视图到 Slint
- `op_graph_keyframes` → apply_keyframes（sync_keyframe_model + rebuild_curve + update_extrapolation + 汇总/预览子函数）
- `op_graph_selection` → apply_selection（纯选中同步：sync_channel_rows / sync_selected_display / sync_keyframe_selection / update_handle_selection_visual / update_summary_entries）
- `op_graph_drag_ended` → apply_drag_ended（restore_handle_preview（关键帧拖拽的 preview 替换恢复普通手柄）/ resort_keyframes / ensure_default_handles（拖拽结束后为贝塞尔关键帧补缺失侧默认水平柄——边缘延伸是水平的，默认柄水平延伸 1/2 间距；拖拽中不补柄，仅结束整理））
- `op_graph_channel` → apply_channel（通道行/段手柄颜色/连接线/汇总）
- `op_graph_scalars` → 同步标量（handles 全部 Graph kind；NLA 变更不触发）

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

**通用工具**（`store/utils/` 单一文件夹）：`marquee::normalize_time/clamp_rows`、`selection::is_*`、`drag::is_valid_delta` 预先查找，避免重复实现

**时间线（Timeline）**（`ui/areas/timeline/`，复用 Sequence 可见行模型，与字幕行同一数据源）：
- 框选：`SequenceAction::MarqueeSelect`（时间 × 行，求交语义，`store/utils/marquee::clamp_rows`）+ `DeselectAll`（空白清除），`store/sequence.rs:marquee_select` 仅命中**事件行**——组头不直接选中，子项全选时经计数派生为间接选中（与行编辑器 Shift 范围选择同语义），`Shift` 加选/`Ctrl` 切换/`None` 替换
- 片段拖拽：`TimelineBlockDragPreview/Commit/Cancel`（无状态预览/提交/取消，同字幕行 `DragTimecodePreview/Commit` 增量模式，delta 为世界时间秒）。预览不写 sequence，行映射器把覆盖同时应用到时间码文本与 `start_sec/end_sec`——一条 Tag 同时驱动行编辑器与时间线块；按住项在直接选中集内（事件或组头）= 整个选中集一同移动（组头直接选中蕴含子树全选，选中集内组头打聚合界覆盖实时跟随）；按住未选中块=单动；右键取消恢复
- 交互：`CanvasNavigation:marquee` 单一 `TouchArea` 复用（框选/中键平移/右键取消），`TimelinePanel` 列表与画布行共享 `pan-y`

**摄影表（Dope Sheet）**（`ui/areas/dope_sheet/`，与曲线编辑器共用数据/选中，拖拽只调时间）：
- 布局参照 Blender：左侧 `ChannelPanel`（搜索框 + 通道列表 + 汇总标题行）+ 顶部 `TimeRuler`
  + 主画布（每通道一行按时间铺开关键帧）；滚轮仅横向缩放（`CanvasNavigation.lock-y-zoom`）、
  中键双向平移、列表滚轮与画布 pan-y 共用同一 clamp 范围
- 交互回调：`dope_keyframe_pressed/dragged/released`（内部即曲线编辑器 `keyframe_*` 同一条
  路径，`dv=0`）、`dope_marquee_select`（时间范围 × 行区间，行 → Rust 映射真实通道）、
  `dope_summary_pressed/dragged/released`（汇总条带跨通道同步拖拽）
- 搜索独立：`dope_search` 只过滤 `dope_filter`（FilterModel），与 `graph_filter` 互不干扰；
  搜索框聚焦走全窗口收起遮罩（`DismissOverlayState` 挖孔定位，右键菜单共用同一条）
- 汇总条带交互路径（`summary.rs`）：按下即写回通道选中（汇总标记独立于 channels 嵌套循环，
  按下期间更新安全）；拖拽返回 `GraphKeyframesChanged{dragged_summary}`，投影就地更新
  被拖标记 time（不 insert/remove，保住 grab）；松开返回 `GraphSelectionChanged`（现有
  条目 ms ∪ 关键帧当前 ms 的并集，逐 ms 精确重建 + 移除空条目，不整体 set_vec）+
  `GraphDragEnded`（重排乱序 + 补柄）
- 拖拽期间只有被拖标记就地移动，各 ms 条目 count/掩码陈旧——由松开时的并集重算收敛

**NLA 编辑器**（`ui/areas/animation/` + `src/store/animation/nla/`，聚合于 `AnimationState`）：
- 布局参照 Blender：左侧 `NlaPanel`（搜索栏 + 轨道列表 + 启用/锁开关）+ 顶部 `TimeRuler`
  + 主画布（轨道泳道按时间铺开片断）；滚轮仅横向缩放、中键双向平移（同摄影表）
- 第 0 轨为**虚拟全局 Action 轨**（is-action）：显示当前曲线编辑器编辑的 Action
  名称/亮色条带（action-start/end，1:1 时间）+ 汇总关键帧（复用摄影表 summary-keyframes），
  只读；`nla-track-push-down` 把当前 Action 作为片断"向下插入"（自动找能容纳的轨道，
  没有则新建）。命中检测 `strip_hit_test` 纯只读（数据层 `track_view`/`editing_strips`），
  选中经 `Action::Animation(AnimationAction::Nla(StripPressed))` dispatch（`bridge/callbacks.rs:setup_nla_callbacks` 纯命中→dispatch，统一 `dispatch_animation`）
- 架构：`AnimationAction::Nla(NlaAction) → AppStore::dispatch_animation（子路由 Nla，改数据层 + model AnimData.tracks，返回精确 Nla* tag）
  → depsgraph 单 Component（DataId::Animation）→ op_nla_*（投影，更新派生层）`；权威数据在 model（`anim_datas` 的 `AnimData`）。`DataId::Animation` 单一，`TagKind`（Graph* vs Nla*）隔离
- 两层状态：**数据层**（reducer 可写）：`nla.track_view`/`editing_strips`；**派生层**（只由投影写）：
  `nla.tracks`/`tracks_filter`/`strip_models`。`set_global_action`仅改数据层返回`NlaTrackChanged{0}`
- Tag 细化（`depsgraph/tag.rs`，DataId::Animation 单一）：
  - `NlaStructureChanged` — 装载/搜索过滤视图重建（全量重建派生层 + 重绑定）
  - `NlaSelectionChanged{updated, deselect_all}` — 片断选中（updated 同时携带稳定 ID 与当前下标，`set_row_data`选中列）
  - `NlaStripPositionChanged{updated}` — 拖拽位置（按稳定 ID 找回当前下标后 `set_row_data`，保住 grab）
  - `NlaStripStructureChanged{moved,reordered,inserted}` — 跨轨/同轨重排/插入（移动 Tag 携带稳定 ID，`remove+insert` row_added/removed）
  - `NlaTrackChanged{track}` — 启用/锁定/全局 Action 名称（`set_row_data`行级）
  - `NlaTrackAdded{track}` — 下推新建轨道（`push` row_added）
- 投影（`projections/animation/nla.rs`，6 ops 归入动画单 Component；`NlaTrackProperties/Added`显式依赖`NlaStructure`）：
  `op_nla_structure`（apply_structure + rebind_nla_tracks）/ `op_nla_selection`（apply_selection）/
  `op_nla_strip_structure`（apply_moved/apply_reordered/apply_inserted 三子函数）/
  `op_nla_position`（apply_position）/ `op_nla_track_properties` / `op_nla_track_added`
- 拖拽语义（对齐曲线编辑器 preview/commit，增量 `delta` 由 Slint 传来，最终提交校验，不克隆快照）：拖拽（高频）只改数据层 + `NlaStripPosition/Structure` Tag，
  投影 `set_row_data` 同步派生层；松开 `strip_released` 整理 dragging + `commit_to_model` 写回 model；右键 `StripDragCancel` 按编辑器内部稳定 ID 快照恢复轨道/顺序/时间，并通过精确结构 + 位置 Tag 走依赖图自动传播；
  多选整体同移：从按下快照计算统一 `offset`/`row_shift`；每个片段独立计算目标轨道约束后求交集，任一片段钳制会让整个选中集使用同一钳制结果；跨轨先批量移除再批量插入
  跨轨移动在拖动中瞬间完成（数据层 remove+insert，投影按稳定 ID 同步最新 start/end/选中态；`apply_moved→reordered→inserted`拆分保序）
- 锁定轨/全局 Action 轨只读：`strip_hit_test` 纯命中，`strip_pick`仅测试兼容
- 测试：`nla_editor/tests.rs` 单测（`project`走`apply_selection/strip_structure/position`单职责；`hit_test`纯只读）+
  `tests/nla_editor_integration.rs`（dispatch → 投影 → model 写回全链路；`strip_hit_test`+`StripPressed` dispatch）

**位置锚点的两种单位**（同一个锚点，可并存）：
- `PropertyPath::PositionX/Y` = 排版像素；`PositionRatioX/Y` = 排版尺寸的分数（0..1）。
  核心数据侧（aweft-model）只有这两个抽象概念，不含 ASS 语义。
- **唯一换算点**是 `aweft-convert` 的 `Cursor::anchor_at`：`offset + layout * ratio`，
  分数部分随排版尺寸（= `AppState::preview_frame_size()`，见
  [video-playback.md](video-playback.md)）等比缩放，像素部分作固定偏移。新增消费者
  不要另起一处换算。
- 一条 strip 走完，`convert_action_strip` 末尾调用一次 `Cursor::advance_to`，把该时刻
  解析出的像素写回 `offset` 并把 `ratio` 清零——下一条 strip 接续的是绝对位置。段内每段
  都从同一个基值解析，把每段终点折回基值会让分数部分被重复累加。
- ASS 侧只在 `\move` 端点用像素；静态样式位置仍不进脚本（`style_to_static_tags`
  从不发 `\pos`），因此纯分数基准值目前没有对应的 `StyleOpKind`，只能靠常量曲线表达。

**位置拟合的误差口径**：`error_tolerance` 对位置通道读作「偏排版尺寸的百分之几」——
`chord_error` 把「真实锚点」到「该时刻线性插值位置」的距离除以排版尺寸
（`Cursor::layout` 的较大边），默认 0.01 即 1% 画面。两处口径都会被否掉：只量到弦所在
无限直线的垂直距离，单轴往复（只有 Y 动的上下浮动）所有采样点都落在那条直线上、误差恒
为 0，整段来回会被一根静止的 `\move` 吃掉；按弦长归一化则在转折点附近失真，弦长趋零而
弯曲量只是二阶小量，相对误差反而飙升，每格都不合格、DP 连不通，最后退化成逐格切段（一行
几百条 `Dialogue`）。代价是端点只在容差内逼近波的极值，不会精确落在关键帧时间上。

**拟合跑在后台线程**：DP 分段是格点平方量级的求值，一份真实工程的脚本要几百毫秒，在 UI
线程上同步跑就等于每次动字幕冻住界面。`op_subtitle_script` 因此只把 `Project` 副本交给
`store/subtitle_fit.rs` 的工作线程，进度经两条通道回到 UI 线程的 pump，pump 直接写
`OverlayState.show-subtitle-fit-overlay`（与波形批次同属「后台增量」，不进依赖图），收尾才把
脚本字节交给 `SubtitlePipeline::set_script`。这份进度画在**视口右上角**的小浮层上
（`aweft-gui/ui/areas/viewport/subtitle_fit.slint`，live_dp 那张曲线图：曲线 + 网格 +
有效性色带 + 当前格点 + 候选段，收尾逐段着色）。浮层刻意不做成模态：卡片里一个 TouchArea
都没有，按下事件穿到它下面的画布上，拟合期间照常编辑——浮层只负责解释「画面为什么还是旧的」。
并发策略是单飞 + 合并最新：同时最多一份在飞，期间新到的请求只置 `fit_pending`，手上这份
跑完再用当时最新的数据补一份，浮层在两次之间不撤。后台线程不回写任何真相——它的唯一产物
是脚本文本，副本里过期的求值缓存只影响一次命中。导出（还没实现）这类必须等结果的场合，
将来另加一个阻塞式反馈，别拿这块角标当挡操作的遮罩用。

**测试字幕动画的播种**（`aweft-gui/src/app/test_data.rs`，HACK，随真实工程流程落地后整块删除）：
`create_test_subtitle_animation` 必须在 `initialize_node_graph_editors` **之前**调用——
链菜单、当前链下标与默认布局都从 model 派生，播种晚于初始化则编辑器看不到这两条链。

**已知 TODO**：调色盘（channel.rs）
