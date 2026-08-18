# 文件布局

```
aweft-gui/src/
├── actions/           # Action 枚举（交互入口）
├── bridge/            # Slint ↔ Rust（callbacks / types / converters）
├── depsgraph/         # 依赖图（DataId / Component / Operation / Tag）
├── screen/            # 屏幕布局（Area / Edge / Vert）
├── store/
│   ├── app_store.rs           # 主 store（dispatch / depsgraph）
│   ├── curve_editor/          # 动画曲线编辑器
│   │   ├── mod.rs             # 结构体（数据层+派生层）+ AppStore 入口
│   │   ├── projection.rs      # 投影：数据层 → 派生层（apply_tags/apply_*）
│   │   ├── load.rs            # Action 装载/提交（数据层）
│   │   ├── rebuild.rs         # 派生重建（rebuild_all/rebuild_curve）
│   │   ├── summary.rs         # 汇总条带（派生 + 交互）
│   │   ├── connections.rs     # 摄影表连接线（派生）
│   │   ├── filters.rs         # 搜索过滤视图（数据层）
│   │   ├── preview.rs         # 拖拽钳制预览
│   │   ├── channel.rs         # 通道操作（数据层）
│   │   ├── keyframe.rs        # 关键帧交互（数据层）
│   │   ├── handle.rs          # 控制柄交互（数据层）
│   │   ├── marquee.rs         # 框选（数据层）
│   │   ├── curve_math.rs      # 纯数学（贝塞尔/钳制/框选）
│   │   └── interpolation.rs   # 插值/颜色查询
│   ├── graph_editor.rs        # 节点图编辑器
│   └── projections/           # 投影（tag → UI 模型）
└── wgpu_subtitle_renderer.rs  # 字幕渲染

aweft-gui/ui/
├── areas/             # 编辑器 UI（line_editor / graph_editor / viewport / node_graph）
├── bridge/            # 全局（callbacks / state）
├── core/              # 类型（types / animation_types）
├── screen/            # 屏幕布局
├── widgets/           # 通用组件
└── theme/             # 主题
```
