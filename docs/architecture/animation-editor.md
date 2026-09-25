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
- **唯一换算点**是 `aweft-convert` 的 `Cursor::anchor_at`：`(基线像素 + Σ像素曲线) +
  layout * (基线分数 + Σ分数曲线)`。样式位置（`position` / `position_ratio`）是**基线**，
  曲线是相对它的位移而不是它本身——这条规则是「视口能拖动画行」的前提（见下）。分数部分
  随排版尺寸（= `AppState::preview_frame_size()`，见 [video-playback.md](video-playback.md)）
  等比缩放，像素部分作固定偏移。新增消费者不要另起一处换算。
- 一条 strip 走完，`convert_action_strip` 末尾调用一次 `Cursor::advance_to`，把该时刻
  解析出的像素写回 `offset` 并把 `ratio` 清零——下一条 strip 接续的是绝对位置。段内每段
  都从同一个基值解析，把每段终点折回基值会让分数部分被重复累加。
- ASS 侧有两条像素出口：动画锚点走 `\move` 端点，**静态**锚点走 `\pos`。后者由
  `match_to_ass_tags` 发（不在 `style_to_static_tags` 里——那个函数不拿排版尺寸，换不了
  像素），换算仍旧只走 `Cursor::anchor_at`，静态时通过 `Cursor::base_anchor` 传空曲线。
  两者在 libass 里**先到者生效**（`ass_parse.c` 的 `EVENT_POSITIONED` 分支整个忽略后来者），
  所以 `\pos` 只补给不含 `\move` 的行；预置到动画行上会让该行的位置动画失效。动画行的基线
  不必另走 `\pos`：它已经折进 `\move` 的每个端点里。
- `\pos` 的锚点是**行框底部中点**，不是左上角：唯一那份生成样式 `Default` 用
  `Alignment::BottomCenter`（`aweft-ass/src/types/style.rs`），libass 的 `get_base_point`
  按对齐方式决定 `\pos` 钉住盒子的哪个点。视口拖动的落点换算必须按这个语义写。
- 纯分数基准值只有曲线一个来源：`SetPosition` 这个 `StyleOpKind` 写的是 `position`
  （像素）而非 `position_ratio`，视口拖动就落在它上面——落的是**增量**（`ShiftLinePosition`
  的 `dx`/`dy` 加到端口原值上，端口没值从 0 起算），因为送绝对坐标会把按下那一瞬动画恰好
  走到的偏移烤进基线，一行拖下来位置就漂。
- `设位置` 除样式口外还多一个 `position` 的 **Vec2 数据出口**（按下标追加在末尾，符合上面
  那条「新增输出口一律追加」的规则），把写进样式的那份位置作为数据供下游运算；端口没有值时
  求值会把这个出口从缓存里**删掉**，下游才会退回 socket 默认值而不是拿到上一次的旧数字。
- **数值输入口在节点上直接显示并编辑端口默认值，一个分量占一行**：未连接的 `float`/`int`
  一行一个框，`Vec2`/`Vec3`/`ClipRect` 分别占 2/3/4 行并带轴标签。分量顺序与行数只在
  `store/node_graph.rs` 的 `socket_labels`/`socket_numbers`/`socket_value_from_parts` 这一组
  函数里定义（x/y/z、x1/y1/x2/y2），`.slint` 那边照 `number-texts` 逐行摆框，不另记一份顺序。
  输出口不给框（值是算出来的），已连线的输入口也不给（数据由连线供，同一行摆两处真相会打架）。
  提交的文本解析不了就不写，删空的中间态不该把已有值清零。
- **端口因此不再等分节点高度**：圆点与端口名只画在该口的**第一行**，行号 `socket.row` 是
  输入列内累加出来的。凡是按行算 y 的地方——命中判定 `find_socket_at`、拖拽释放
  `find_drop_socket`、节点高度 `node_height`、连线端点与橡皮线的 `socket-offset`、端口预设
  菜单的弹出位置——都必须读模型里的 `row`/`span`，拿端口下标乘行高就会整体错位（顺带修掉了
  拖输出口时橡皮线起点偏低一格的老错：那里传的是 sockets 组合下标，输入在前）。

**应用链怎么绑到某一行字幕**（链与行之间没有引用字段，只有条件这一条路）：
- 应用链与样式链都不持有行 ID：唯一能问到「当前是哪一行」的入口是 `ContextInput` 的输出
  （actor / line_text / line_index / id）。自动生成的链就靠这条路径，别另找旁路。
- `line_index` 是当前**可见行序号**，插入/删除/隐藏任意一行都会让它移位。凡是需要跨编辑
  存活绑定（视口拖动写出的那条链）都必须用 `LineContext.item_id` 配 `TextEq`；按 index 绑
  `IntEq` 只适合当场的一次性判断。
- `ContextInput` 的 `id` 是**追加在输出口末尾**（下标 3）：已存盘图的端口按下标记录，
  插队会让老连线指错口。新增输出口一律走这个规则。
- 图求值缓存（`EvalCache`）**按行有效**：`NodeGraph::evaluate` 开头比对 `line_key`，与
  上一轮不同就整图标脏。`ensure_node_evaluated` 只重算脏节点，少了这一步的话同一张图求值
  第二行会直接沿用第一行的 `ContextInput` 输出——每行一条链的功能会静默地只命中第一行。

**视口拖动写出的链对**（`src/store/subtitle_chains.rs`，位置的唯一真相）：
- 拖动一行 = 生成一对链：样式链 `样式源 → 设位置(Vec2 排版像素) → 组输出`，应用链
  `上下文.id → 文本全等(expected = item_id) → 应用 → 组输出`。行上没有也不该有旁路位置字段。
- **一行一对，不共用一条链**：端口只能在 `add_node` 时声明，没有给既有节点追加输入口的 API，
  所以应用链的组输出口没法再接第二个「应用」。共用一条链的方案走不通。
- 找回这对链**靠结构不靠名字，也不加持久化标记字段**：从 `expected == item_id` 的 `文本全等`
  出发，顺着连线走「它的输出门控了哪个 应用 的条件口」→「那个 应用 的样式口来自哪个 样式链引用」
  →「引用的样式链里第一个 设位置 的 position 口」。用户改链名、往链里加节点都不影响找回；
  代价是这段图结构一旦有人手工拆开（比如把 文本全等 的 expected 改成常量文本）绑定就断，
  下次拖动会再生成一对，旧的那对留在菜单里由用户清理。
- 应用链 `push` 到 `style_layer.graphs` 末尾 = 最高优先级（后者经 `merge_over` 覆盖前者），
  所以视口拖动的位置总是压过用户手工搭的链；这是刻意的，别改成插入到固定下标。
- 新建这对链打 `NodeGraphChange::ChainListChanged`（两侧各一次），投影走
  `reload_chain_list`：只重派生两个菜单，并把当前编辑的链**按 id** 重新定位下标。不能用
  `ChainSwitched`——样式链菜单按名字排序，中间插进一条新链会让原来的下标指向另一条链，
  而拖动一行不该把用户正在编辑的那条换掉。

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
