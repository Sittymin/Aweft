# 无头 UI 测试

用 Slint testing 后端（无事件循环）驱动真实管线：
`dispatch → evaluate → 投影 → UI 模型`，断言 Slint 模型内容。

## 模式

- 每个测试先初始化 testing 后端 + 创建窗口实例 + 构造 `AppStore`。
- **dispatch 后必须手动 evaluate**（dispatch 只打 tag，不自动求值）。
- 断言读 store 的 UI 模型（`row_count` / 行数据）。
- 回调链测试走真实 UI 回调入口（`invoke_*`），覆盖回调层的 uuid 解析、时间解析、
  批量分支。
- 帮助函数构造 Rows → Map 映射模型（仅测试用，实际链路 Rows → Filter → Map）。

## 覆盖清单（aweft-gui/tests/）

**depsgraph_integration.rs**（数据 → 投影闭环，不经回调）：初始化全量重建、split 复用
组件 + 精确增删、缩放几何局部更新、单行局部化（只通知受影响行）、拖拽 overlay 预览
清除、合并保留 edge touch/actionzone、同轮多 tag、动画数据 → 派生模型。

**dope_sheet_integration.rs**（摄影表交互，dispatch → 投影闭环）：关键帧拖拽只移动
时间（dv=0）且写回 model、框选按时间 × 行区间命中、汇总条带跨通道同步拖拽（按下选
该时刻全部 / 拖拽同步移动 / 松开重算条目 + 写回）、搜索独立过滤（dope_filter 与
graph_filter 互不干扰）。

**ui_callbacks_integration.rs**（真实回调链）：时间码输入/批量 → 行更新 + 分组头聚合、
add-line 时间解析分支、折叠 → FilterModel 过滤、选择路由（单选/范围/切换）+ 分组头
选中态、非法 uuid 忽略、解散分组、复制/剪切/粘贴/删除链、拖拽预览覆盖 → commit。

**graph.rs 单测**（纯图，无 Slint）：入口路由、操作去重、无关数据域不触发、无隐式连边、
显式 depends_on 传播、拓扑序、未注册操作 panic。

**nla_editor_integration.rs**（NLA 交互，dispatch → 投影闭环）：装载派生层与 model 对齐（全局 Action 轨 + 泳道/片断）、拖拽时间钳制与跨轨瞬间移动、松开写回 `AnimData.tracks`、轨道启用/锁定/搜索过滤与“向下插入”（自动建轨）、过滤视图命中映射（`strip_hit_test`+`strip-pick`）、拖拽重排后跨轨两层同步、空白取消选中。
