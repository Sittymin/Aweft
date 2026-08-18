# 文件布局

## 命名约定（与 Blender 对齐）

- **图编辑器（Graph Editor）**：动画曲线编辑器（FCurve 通道/关键帧/手柄），UI 侧 `ui/areas/graph_editor/`
- **节点图（Node Graph）**：样式图/应用图节点编辑器，UI 侧 `ui/areas/node_graph/`

每个领域在 `actions → reducers → projections → store` 四层使用同一名字，且四层为
`src/` 下的同层兄弟目录（见下）。

## `aweft-gui/src/`

```
src/
├── actions/             # Action 枚举（交互入口，按域分文件：animation/area/drag/line_editor/media/node_graph/screen）
├── app/                 # 应用生命周期（window_manager：创建窗口、初始化 store/编辑器）
├── bridge/              # Slint ↔ Rust（callbacks / types / converters）
├── clipboard/           # 剪贴板（facade 强类型 API + backend 系统/内存 + kinds 数据种类）
├── depsgraph/           # 依赖图调度器（DataId / Component / Operation / Tag，graph.rs 调度实现）
├── projections/         # depsgraph 投影注册（只读 aweft-model，tag → 写派生 UI 模型；按 DataId 分文件：line_editor/screen/animation/node_graph）
├── reducers/            # 按域 reducer（**唯一改动 aweft-model 的入口**，改 `AppState.project` + 返回精确 tag，每域一文件）
│   ├── area.rs / drag.rs / media.rs / screen.rs   # 纯函数 handle_*（只改 AppState）+ dispatch 包装
│   └── line_editor.rs / animation.rs / node_graph.rs # impl AppStore 方法（改 Domain + UI 态）
├── render/              # 渲染进 Slint
│   ├── subtitle.rs      # 字幕渲染（wgpu + libass 图块实例化）
│   ├── subtitle.wgsl    # 字幕着色器
│   └── video_sink/      # 视频播放（gstreamer → Slint，software_rendering / wgpu_integration）
├── screen/              # 屏幕布局（area / edge / vertex / map / primitives + ops/：split/join/move_edge/cleanup）
├── store/               # 状态中心：`AppState` 持有 `aweft-model::Project`（唯一真相，reducers 改 / projections 只读）+ 各编辑器状态
│   ├── app_store.rs     # AppStore：dispatch 总表 + depsgraph 求值 + 访问器（单一数据源）
│   ├── state.rs         # AppState（持有 aweft-model Project + UI 态）
│   ├── ui_models.rs     # UiModels（编辑器状态 + Slint VecModel 汇总）
│   ├── graph_editor/    # 图编辑器（动画）状态：数据层 + 派生层 + 交互/装载/投影实现
│   │   ├── mod.rs       # GraphEditorState（结构体 + AppStore 初始化入口）
│   │   ├── projection.rs# 投影：数据层 → 派生层（apply_tags/apply_*）
│   │   ├── load.rs      # Action 装载/提交（数据层）
│   │   ├── rebuild.rs   # 派生重建（rebuild_all/rebuild_curve）
│   │   ├── summary.rs   # 汇总条带（派生 + 交互）
│   │   ├── connections.rs # 摄影表连接线（派生）
│   │   ├── filters.rs   # 搜索过滤视图（数据层）
│   │   ├── preview.rs   # 拖拽钳制预览
│   │   ├── channel.rs   # 通道操作（数据层）
│   │   ├── keyframe.rs  # 关键帧交互（数据层）
│   │   ├── handle.rs    # 控制柄交互（数据层）
│   │   ├── marquee.rs   # 框选（数据层）
│   │   ├── curve_math.rs# 纯数学（贝塞尔/钳制/框选）
│   │   ├── interpolation.rs # 插值/颜色查询
│   │   └── tests.rs     # 单元测试
│   ├── node_graph.rs    # 节点图（样式图/应用图）状态：NodeGraphState + with_graph 唯一访问器 + 图操作
│   ├── line_editor.rs   # 字幕行编辑器状态（LineEditorState）
│   └── line_rows_model.rs # 行数据源 Model（RowData）+ 折叠过滤/选择三态
└── lib.rs
```

## `aweft-gui/ui/`

```
ui/
├── app/             # 根窗口（window.slint）
├── areas/             # 编辑器 UI（line_editor / graph_editor（动画）/ dope_sheet（摄影表）/ node_graph / style_graph / app_graph / viewport / subtitle）
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

