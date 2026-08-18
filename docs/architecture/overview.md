# 整体架构

**技术栈**：Slint（UI）+ Rust 后端 + wgpu（字幕渲染）+ 自定义依赖图调度

**数据流**：
```
UI 事件 → callbacks → Action → AppStore::dispatch
  → reducer 改 domain + 返回 Vec<Tag>
  → depsgraph.evaluate → 投影更新 UI 模型
```

**crate**：
- `aweft-model`：数据模型（Project / AnimationLibrary / NodeGraph）
- `aweft-gui`：GUI（actions / bridge / depsgraph / store / ui）
- `aweft-libass`：ASS 渲染绑定

**区域类型**：LineEditor ✅ | Viewport 占位 | StyleGraphEditor ✅ | AppGraphEditor ✅ | GraphEditor（动画）✅

**原则**：model 是唯一真相；`set_row_data` 局部更新；拖拽期间不更新嵌套 repeater 外层模型
