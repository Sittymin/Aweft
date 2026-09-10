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
- 纯 UI 交互（不涉及数据层）：测试文件内用 `slint::slint!` 声明最小宿主窗口 + `import` `ui/` 下真实
  组件，用 `ElementHandle`（`find_by_element_id` 的 id 是 `组件名::元素名`）派发真实点击/拖拽。
  三个坑：相对路径以 crate 根（`aweft-gui/`）为基，不是测试文件所在目录；`init_no_event_loop()` 下
  布局与 `if` 分支的实例化要靠 `mock_elapsed_time()` 推进，读几何前先推一帧；断言写在
  `slint::spawn_local` 异步块里时，必须由异步块末尾 `quit_event_loop()` 加块外
  `run_event_loop()` 驱动，否则异步块从不被轮询，断言静默不执行而测试恒绿（该函数返回
  `Result`，丢弃它会报 `unused_must_use`，是这类空转测试唯一的可见信号）。

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

**context_menu_dismiss.rs**（右键菜单“点菜单外收起”契约）：真实 `ContextMenu` + 真实 `DismissOverlay` +
最小宿主窗口（测试内 `slint::slint!` 直接 import `ui/` 组件，不拉 AppStore），锁定菜单上报的挖孔矩形是
MainWindow 窗口坐标（`absolute-position`，Area 偏移参与计算）、孔外**右键**按下即收起并自动关菜单
（`clicked` 只认左键，是当初波形图音轨菜单收不起来的根因）、孔内点击命中菜单项且收起时释放遮罩、
已收起菜单（`visible: false`）不再吞点击。
