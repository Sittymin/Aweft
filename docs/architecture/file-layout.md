# 文件布局

## 命名约定（与 Blender 对齐）

- **图编辑器（Graph Editor）**：动画曲线编辑器（FCurve 通道/关键帧/手柄），UI 侧 `ui/areas/graph_editor/`
- **节点图（Node Graph）**：样式图/应用图节点编辑器，UI 侧 `ui/areas/node_graph/`

每个领域在 `actions → reducers → projections → store` 四层使用同一名字，且四层为
`src/` 下的同层兄弟目录（见下）。

## `aweft-gui/src/`

```
src/
├── actions/             # Action 枚举（交互入口，按域分文件：animation/area/drag/sequence/media/node_graph/screen）
│   └── animation/       # 动画域统一入口（DataId::Animation 单一，Graph/NLA TagKind 隔离）
│       ├── mod.rs       # AnimationAction::Graph(GraphAction) / Nla(NlaAction) 单域聚合
│       ├── graph.rs     # 曲线/摄影表（Graph）Action（Keyframe/Handle/Channel/Marquee/Summary）
│       └── nla.rs       # NLA 轨道/片断 Action（Strip/Track/PushDown/Search）
├── app/                 # 应用生命周期（window_manager：创建窗口、初始化 store/编辑器）
├── bridge/              # Slint ↔ Rust（callbacks / types / converters）
├── clipboard/           # 剪贴板（facade 强类型 API + backend 系统/内存 + kinds 数据种类）
├── depsgraph/           # 依赖图调度器（DataId / Component / Operation / Tag，graph.rs 调度实现）
├── projections/         # depsgraph 投影注册（只读 aweft-model，tag → 写派生 UI 模型；按 DataId 分文件：sequence/screen/animation/node_graph）
│   └── animation/       # 动画域单 Component（DataId::Animation，Graph+NLA 13 ops，TagKind 隔离）
│       ├── mod.rs       # 重导出 op_graph_* / op_nla_*，单域注册说明
│       ├── graph.rs     # 曲线/摄影表 6 ops（GraphStructure/Keyframes/Selection/DragEnded/Channel/Scalars）
│       └── nla.rs       # NLA 6 ops（NlaStructure/Selection/StripStructure/StripPosition/TrackProperties/TrackAdded）
├── reducers/            # 按域 reducer（**唯一改动 aweft-model 的入口**，改 `AppState.project` + 返回精确 tag，每域一文件/目录）
│   ├── area.rs / drag.rs / media.rs / screen.rs   # 纯函数 handle_*（只改 AppState）+ dispatch 包装
│   ├── sequence.rs      # Sequence 域（impl AppStore，改 Sequence + Selection）
│   ├── node_graph.rs    # NodeGraph 域
│   └── animation/       # 动画域统一分发（DataId::Animation 单一，Graph/NLA 子路由）
│       ├── mod.rs       # dispatch_animation → Graph/Nla 子分发
│       ├── graph.rs     # 曲线/摄影表子 reducer（改 Action 曲线 + 打 Graph* Tag）
│       └── nla.rs       # NLA 子 reducer（改 AnimData.tracks + 打 Nla* Tag）
├── render/              # 渲染进 Slint
│   ├── subtitle.rs      # 字幕渲染（wgpu + libass 图块实例化）
│   ├── subtitle.wgsl    # 字幕着色器
│   └── video_sink/      # 视频播放（gstreamer → Slint，software_rendering / wgpu_integration）
├── screen/              # 屏幕布局（area / edge / vertex / map / primitives + ops/：split/join/move_edge/cleanup）
├── store/               # 状态中心：`AppState` 持有 `aweft-model::Project`（唯一真相，reducers 改 / projections 只读）+ 各编辑器状态
│   ├── app_store.rs     # AppStore：dispatch 总表 + depsgraph 求值 + 访问器（单一数据源）
│   ├── state.rs         # AppState（持有 aweft-model Project + UI 态）
│   ├── ui_models.rs     # UiModels{ animation: AnimationState, sequence_state, style/app_graph, screen } 汇总
│   ├── utils/           # 通用工具（单一文件夹，避免重复造轮子）
│   │   ├── marquee.rs   # 时间/行归一化与 clamp（曲线/摄影表/时间线/NLA 复用）
│   │   ├── selection.rs # 修饰键语义 is_replacing/is_additive/is_toggling（点选/框选全局复用）
│   │   └── drag.rs      # 拖动增量校验 is_valid_delta（字幕行/NLA/曲线共用）
│   ├── animation/       # 动画域聚合状态（DataId::Animation 单一，因 Area 表现形式不同保留两组派生状态，TagKind 隔离）
│   │   ├── mod.rs       # AnimationState{ graph: GraphEditorState, nla: NlaEditorState } + 初始化
│   │   ├── graph/       # 图编辑器（Graph Editor/Dope Sheet）状态：数据层 + 派生层 + 交互/装载/投影实现
│   │   │   ├── mod.rs       # GraphEditorState（结构体 + AppStore 初始化入口）
│   │   │   ├── projection.rs# 投影：数据层 → 派生层（apply_tags/apply_*）
│   │   │   ├── load.rs      # Action 装载/提交（数据层）
│   │   │   ├── rebuild.rs   # 派生重建（rebuild_all/rebuild_curve）
│   │   │   ├── summary.rs   # 汇总条带（派生 + 交互）
│   │   │   ├── connections.rs # 摄影表连接线（派生）
│   │   │   ├── filters.rs   # 搜索过滤视图（数据层）
│   │   │   ├── preview.rs   # 拖拽钳制预览
│   │   │   ├── channel.rs   # 通道操作（数据层）
│   │   │   ├── keyframe.rs  # 关键帧交互（数据层）
│   │   │   ├── handle.rs    # 控制柄交互（数据层）
│   │   │   ├── marquee.rs   # 框选（数据层）
│   │   │   ├── curve_math.rs# 纯数学（贝塞尔/钳制/框选）
│   │   │   ├── interpolation.rs # 插值/颜色查询
│   │   │   └── tests.rs     # 单元测试
│   │   └── nla/         # NLA 编辑器状态（NlaEditorState：轨道/片断数据层 + 派生层 + model 读写 + 拖拽）
│   │       ├── mod.rs       # NlaEditorState（结构体 + AppStore 初始化入口）
│   │       ├── load.rs      # 装载/提交/全局 Action 同步（数据层；初始化例外直写派生）
│   │       ├── projection.rs# 投影：数据层 → 派生层（apply_selection/position/strip_structure 拆三子函数 + track）
│   │       ├── selection.rs # 选中/`hit_test`纯只读（`strip_hit_test`）/ `strip_pick`兼容
│   │       ├── drag.rs      # 拖拽（跨轨/水平钳制/重排，Tag 细化）
│   │       ├── strip.rs     # 片断视图构造（from_model/new_action）
│   │       ├── track.rs     # 轨道（`push_track`测试辅助/`find_fit_track`/`toggle_*`）
│   │       └── tests.rs     # 单元测试（`project`按 TagKind 单职责调度）
│   ├── node_graph.rs    # 节点图（样式图/应用图）状态：NodeGraphState + with_graph 唯一访问器 + 图操作
│   ├── sequence.rs      # 序列状态（SequenceState，时序聚合 + 折叠/选择）
│   └── sequence_rows_model.rs # 行数据源 Model（RowData）+ 折叠过滤/选择三态
└── lib.rs
```

## `aweft-gui/ui/`

```
ui/
├── app/             # 根窗口（window.slint）
├── areas/             # 编辑器 UI（line_editor / graph_editor（动画）/ dope_sheet（摄影表）/ animation（NLA）/ node_graph / style_graph / app_graph / viewport / subtitle）
├── assets/          # 图标（icons/）与纹理（textures/）
├── bridge/          # 全局（callbacks / state）
├── core/            # 类型（types / animation_types / area_type_metadata）
├── screen/          # 屏幕布局（area_container / action_zone / edges/ / overlays/）
├── widgets/         # 通用组件
└── theme/           # 主题（tokens / mixins / index）
```

## 数据流（store 是中心，不是链尾）

```
actions/{域}.rs（事件入口）→ reducers/{域}.rs（唯一改动 aweft-model 的入口，返回精确 tag）
  → depsgraph（DataId 入口索引 + Relation 闭包 + 拓扑）
  → projections/{域}.rs（只读 aweft-model → 写派生 UI 模型）
        ↕
   store/{编辑器}.rs（状态中心，AppState 持有 aweft-model::Project 唯一真相）
```

四层目录均为 `src/` 同层，深度一致；`store/` 字面即"落点"，不再有层级倒挂。
改动唯一真相只有 `reducers/` 一条路（例外：启动初始化 `window_manager` / `new_editing_action`）。

