# UI 依赖图设计文档

> 分支：`ui-depsgraph`（从 `main` 分出）
> 目标：用依赖图调度器替换旧的 `StateChange → UIUpdateCommand → UIUpdater` 手工管线，
> 实现以数据为中心的**最小化 UI 更新**（避免 Slint 组件销毁重建）。

## 1. 背景与动机

随着 Area 数量增多（LineEditor / Viewport / Timeline / GraphEditor / DopeSheet / NLA / 节点图……），
Area 间相互影响越来越多：

- 一条字幕时间码变化 → 行列表、Timeline、祖先分组聚合都要更新；
- 播放头移动 → GraphEditor、DopeSheet、NLA、Timeline 同步；
- 节点图数据变化 → 样式图、应用图、链切换器联动。

旧的 `StateChange → UIUpdateCommand → UIUpdater` 是**手工映射管线**：每个 reducer 手动构造
`StateChange`，`UIUpdateCommand::from_state_change` 手动映射成指令，`UIUpdater` 手动 match 每条
指令更新 Slint 模型。新增一个 Area 就要动三处代码，且极易退化成 `UpdateAll` 全量刷新。

依赖图方案让**每个投影自己声明"我依赖什么数据、我的操作如何执行"**，新增 Area 只需注册一个投影。

## 2. 核心概念（借鉴 Blender Dependency Graph，裁剪适配 UI）

### 2.1 数据域（`DataId`）

可被标记变更的权威数据。reducer 修改数据后对数据域打 tag。

```rust
pub enum DataId {
    ProjectStructure,  // 字幕行/分组/时间码/文本/层级
    Selection,         // 选中状态
    ScreenLayout,      // areas/edges/verts 几何与类型
    DragState,         // 拖拽预览
    Animation,         // AnimData/Action（曲线/Dope/NLA）
}
```

**数据依赖边**：组件通过 `Component::depends_on: &'static [DataId]` 声明它依赖哪些数据域。
tag 的 `data_id()` 命中组件依赖的任一数据域，该组件即进入本次调度
（Blender 的 `IDNode → ComponentNode` 关系）。这是跨组件联动的载体：
多个组件依赖同一数据域 → 一次 tag 同时驱动它们。

### 2.2 变更（`Tag`）

一次数据变更的描述，**带 payload**（如 `event_id, time`）供投影做精准局部更新。
每个 `Tag` 归一到某个 `DataId`（`Tag::data_id()`，决定哪些组件被调度），
并归一到某个操作语义分类（`Tag::kind() -> TagKind`，决定组件内哪个操作被路由）：

- **元素级**：值变化 tag 携带 `item_id` + 元素标识 + `ancestors`（祖先分组链），
  投影据此精确更新"该行"与"所有祖先分组聚合"。

### 2.3 投影组件与操作（`Component` / `Operation`）

- **组件（`Component`）**：一个 UI 视图的"求值方面"（如 LineEditor 的行列表、分组聚合、选中状态）。
  声明 `depends_on`（依赖的数据域）。注册后仅作描述，操作本体移入 `Depsgraph::op_nodes`。
- **操作（`Operation`）**：原子更新动作（"更新某行时间码"、"插入一行"），**图节点自持 `exec`**
  （Blender 的 OperationNode）。每个操作声明 `handles: &'static [TagKind]`——它处理哪些语义分类。
  依赖只来自显式 `depends_on`（跨组件或组件内统一机制），**不按声明顺序自动连边**。

```rust
pub struct Operation {
    pub id: OperationId,
    pub code: OperationCode,                    // 编译期唯一标识
    pub handles: &'static [TagKind],            // 入口路由
    pub depends_on: &'static [OperationCode],    // 显式 Relation（跨组件或组件内）
    pub exec: OperationExec,                    // fn(&mut AppStore, &[Tag])
}
```

**节点存储**：`Depsgraph::op_nodes: HashMap<OperationId, Operation>`——注册时从
`Component.operations` 移入，求值 `op_nodes[&op_id].exec` O(1) 直接调用。
未来操作需加运行时状态（dirty 标记、统计）直接在 `Operation` 上加字段。

### 2.4 调度（`Depsgraph::evaluate`）

```
reducer 改数据 → depsgraph.tag(Tag)   // 可多次
depsgraph.evaluate(&mut store)
  → flush：每个 tag 的 (data_id, kind) → entry_ops **O(1) 查表**拿到入口操作
           → 沿全部出边（显式 depends_on 解析的 Relation 边）BFS 扩散闭包
  → Kahn 拓扑排序（跨组件依赖在前的先执行）
  → 单轮依次执行
  → 清空 pending_tags
```

**跨组件传播**（Blender 的 Relation）：操作通过 `Operation::depends_on`
声明依赖其他操作（`OperationCode` 枚举，编译期检查、全局唯一——前缀隐含组件维度，
如 `Line*` 属 LineEditor）。投影组件是核心库随版本更新的内容，无运行时动态注册
需求，故不需组件名字符串。`register_all` 末尾 `resolve_relations` 把它们解析为
OperationId 出边（构建期建图）。求值时单轮 flush 沿这些边算出完整闭包——
无需"操作打新 tag 下一轮"。

**入口索引（Blender 的 entry tags）**：`register` 时按 `组件 depends_on ×
操作 handles` 预建 `(DataId, TagKind) → Vec<OperationId>` 表，flush 时
`tag.data_id() × tag.kind()` 直接查表得到入口操作——构建期索引，运行时
O(1)，不做任何线性扫描。

### 2.5 与 Blender DEG 的对应

| Blender | 我们 |
|---|---|
| IDNode（data-block） | `DataId`（数据域） |
| IDNode tag → ComponentNode 依赖 | `Component::depends_on`（注册时并入 entry_ops 索引） |
| ComponentNode（求值方面） | `Component`（投影组件） |
| OperationNode（原子操作） | `Operation`（原子更新动作） |
| 跨组件 Relation 边（operation → operation） | `Operation::depends_on` + `resolve_relations` |
| entry_tags（数据块 tag → 入口操作，构建期预建） | `entry_ops: HashMap<(DataId, TagKind), Vec<OperationId>>` |
| flush（沿关系边扩散闭包） | `flush_affected`（沿全部出边 BFS） |
| 构建期建图 / 单轮求值 | `register()`+`resolve_relations()` 建图 / `evaluate()` 单轮 |

## 3. Slint 最小化更新机制（调研结论，投影实现的依据）

### 3.1 关键成本差异

| 操作 | 通知 | Repeater 行为 | 组件影响 |
|---|---|---|---|
| `set_row_data(row, data)` | `row_changed(row)` | 对现有实例原地 `update()` | **零重建**（最优） |
| `insert`/`remove` | `row_added`/`row_removed` | splice，存量实例复用 | 局部 |
| `set_vec`/`clear`/换模型 | `reset()` | `instances.clear()` | **全量销毁重建**（兜底） |

### 3.2 Slint 自带的 adapter（不必手动维护 index map）

| Adapter | 用途 | 反向映射 | 最小化行为 |
|---|---|---|---|
| `MapModel` | 行数据映射 | — | 透传通知，零开销 |
| `FilterModel` | 条件过滤 | `unfiltered_row()` | 逐行 diff |
| `SortModel` | 排序 | `unsorted_row()` | 单行移动 diff |
| `ReverseModel` | 倒序 | — | 纯算术转发 |

**启示**：LineEditor 视图层不手动维护 `key → index` 映射——用 `FilterModel` 做折叠过滤，
`MapModel` 让"内部行结构 → UI 行 item"的投影透传 `row_changed`，天然最小化。
（Screen 侧不同：Area/Edge 用 `VecModel` + 手动 `index_map`，因 edge 合并行多 key 共
一行，index 映射是几何驱动更新的需要。）

### 3.3 ListView 虚拟化

`ListView` 的 `for` 只实例化可见行，滚动复用实例，视口外销毁。
对不可见行 `set_row_data` 近乎零成本（无实例直接跳过）。
**行数再多，只要用 ListView + set_row_data，就不必担心全量重建。**

## 4. 投影实现规范

投影按 tag 分类选择更新策略：

1. **值变化**（`EventTimecodeChanged`/`SelectionChanged`/`GroupCollapsedToggled`）→
   收集 affected rows，`notify.row_changed(i)` 局部更新（含祖先分组头聚合传播）。
2. **结构变化**（`ItemsInserted`/`ItemsRemoved`/`ItemsPasted`/`GroupsUngrouped`）→
   精确 `notify.row_added/row_removed`（Repeater splice，存量复用）。
3. **布局变化**（`ScreenResized`/`ScaleChanged`）→ 逐个 `set_row_data` 几何局部更新；
   `AreasSplit`/`AreasJoined` → 原 area 保留 + `row_added/row_removed` 精确增删。
4. **初始化**（`ScreenInitialized`）→ `set_vec` 全量重建（从空创建，唯一合理场景）。

**原则**：所有行/分组结构操作必须产生具体 tag（精确 insert/remove），
不允许走 `StructureChanged` 全量兜底——`op_line_structure` 收到 `StructureChanged`
会明确 `log::error`（防止未来惰性使用全量更新）。全量重建仅保留给初始化
（从空创建）与纯边移动拖拽结束（边结构可能因去重而变化）。

## 5. 已完成的架构落地

1. ✅ depsgraph 三层结构（`aweft-gui/src/depsgraph/`）：DataId → Component → Operation + 单测
2. ✅ **入口索引 + 数据依赖边传播**：`entry_ops` 构建期预建 `(DataId, TagKind) → 入口操作`
   （O(1) 查表），`Operation::depends_on` → `resolve_relations` 建跨组件边，
   flush 沿全部出边 BFS 闭包 + Kahn 拓扑
3. ✅ 投影声明与注册（`store/projections/`）：LineEditor / Screen 按 TagKind 拆细操作
4. ✅ **LineEditor 数据源化**：`LineRowsModel` 实现 `slint::Model` 产出轻量 `RowData`，
   `FilterModel` 在 RowData 层做折叠过滤（用 `ancestor_ids` ItemId 判断，零 UUID parse，
   被过滤掉的行完全不构造 UI 行），`LineRowMapper`（`MapModel`）映射为
   `SlintSubtitleRow`（含分组聚合），`ModelNotify` 驱动增量更新
5. ✅ **元素级 Tag**：`EventTimecodeChanged{event_id, ancestors}` /
   `SelectionChanged{item_id, affected}`——值变化通知 Slint 精确重算该行 + 祖先分组头
6. ✅ 删除旧管线（`ui_update/`、`state_change.rs`）+ 行副本缓存（旧 `all_rows_model`、
   index map 平移）；全量映射 `all_rows` 移入测试模块（仅测试用）
7. ✅ 接入 `dispatch_and_render`，无头集成测试通过
8. ✅ **动画编辑器接入依赖图**：reducer 只改数据层（`channels_data`/`editing_keyframes`）返回
   精确 tag（Structure/Keyframes/Selection/Channel/Scalars），投影 `op_animation_*` 同步
   派生层（Slint 绑定模型 + 标量）；过滤视图重建后重新绑定 Slint global
   （FilterModel 捕获搜索词，旧实例不感知新词）

**最终数据流**：

```
UI 事件 → callbacks → Action → AppStore::dispatch(action)
  └─ reducer 改 model（project/screen_map）+ 返回 Vec<Tag>（元素级，带 ancestors）
  └─ depsgraph.tag(tag)
AppStore::evaluate_depsgraph()
  └─ flush：tag.data_id() → 依赖组件 → handles 入口 → BFS 闭包 → Kahn 拓扑执行
    └─ LineEditor.Timecode：notify_row_changed(该行 + 祖先分组头) → Slint 精确重算
    └─ LineEditor.Selection：set_selected + notify 受影响行
    └─ LineEditor.Structure：notify.row_added/row_removed 精确增删
    └─ Screen：areas/edges 更新 / OverlayState 覆盖层
    └─ Animation：channels/keyframe_models/segments/handles/connections/summary 增量同步 + 标量
```

**Slint 标准复用**（不重复造轮子）：

- 行映射：`MapModel`（`LineRowMapper`）——轻量 `RowData` → UI 行，透传 `row_changed`
- 折叠过滤：`FilterModel` + `reset()`——按折叠状态逐行 diff
- 行级更新：`notify.row_changed/row_added/row_removed`（`LineRowsModel` 手写 `Model` 的必要 API）
- Screen 数据：`VecModel` 精确更新——`set_row_data`（几何局部）、
  `insert`/`remove`（split/join 的 `row_added/row_removed`）、`set_vec`（仅初始化）
- 行选择/结构操作的共享状态：`Rc<RefCell<...>>` 跨模型共享（零拷贝）

### 5.1 为什么数据源要用 `RowData` + `MapModel` 两层

`SubtitleState.rows`（Slint 全局）最终消费的是**完整 `SlintSubtitleRow`**，
但数据源 `LineRowsModel` 实现的 `Model<RowData>` 只产出**轻量数据**
（`item_id` + `depth` + `ancestors`，O(1) 从锚点构造）。中间由 `MapModel`
负责转换。两层各有存在理由：

1. **惰性聚合**：完整行构造含 `aggregate_group`（递归算分组时间/行数）。
   若数据源直接产出完整行，结构变化需全量构造所有行（O(N) × 每行聚合）。
   拆开后 `row_data(i)` 只是复制锚点 3 个字段（O(1)），聚合延迟到 Slint
   实际拉取那行时才算（滚动时只算可见行）。
2. **类型解耦**：`RowData` 不含 Slint UI 类型，`LineRowsModel` 可独立测试。
3. **复用标准机制**：`MapModel` 免费提供三件事——
   - **类型转换**：`Model<RowData>` → `Model<SlintSubtitleRow>`（否则要手写 Model 实现）；
   - **通知透传**：源模型 `row_changed(i)` → MapModel 自动对那一行重新调 `map_fn` →
     通知下游 Slint（否则要手写"源模型 + 映射 + 通知转发"三套逻辑）；
   - **懒加载**：Slint 拉取哪行才调 `map_fn`，不预构造。

**分工**：投影/B 层只负责"数据变了 → 调 `notify_row_changed`"；
MapModel 负责"收到通知 → 重映射那一行 → 让 Slint 重绘"。
注意：通知必须由源模型（`LineRowsModel`）发出，不能往 `SubtitleState.rows`
链的末端直接操作——代码注释强调"不应当再使用 `set_rows` 修改数据"
（`window_manager.rs`），所有更新都从 B 层通知进来，否则会绕过 MapModel。

**无头 UI 测试**：

- `aweft-gui/tests/depsgraph_integration.rs`：dispatch → tag → evaluate → 投影 → UI 模型
  （屏幕初始化/分割/缩放/合并、TagKind 路由隔离、两行局部化、同轮多 tag、拖拽 overlay）
- `aweft-gui/tests/ui_callbacks_integration.rs`：走真实 UI 回调链
  （`setup_callbacks` + `LineEditorCallbacks.invoke_*`），覆盖回调层 uuid 解析、
  `parse_time`、is_batch 分支 → dispatch → depsgraph → UI 模型（时间码、选择、折叠、
  分组聚合、复制/剪切/粘贴/删除链、加组 + 批量时间码）

## 6. 已知取舍

- **跨组件依赖通过 `Operation::depends_on` 显式声明**（Blender Relation），
  `resolve_relations` 构建期解析为边，单轮求值覆盖。投影是纯读数据 + 写 UI 模型，
  不修改 reducer 数据、不产生新 tag（Blender 的求值修改数据→pending recalc 场景
  在我们这不出现——reducer 一次打全 tag，投影一次性消费）。
- **重算函数用函数指针**而非闭包：避免 `Depsgraph` 持有 `AppStore` 引用导致循环借用。
- **`depends_on` 用 `OperationCode` 枚举**而非直接 `OperationId`：
  因 `OperationId` 运行时生成，无法静态引用；枚举提供编译期检查（拼错在编译期暴露）。
  操作增加 `OperationCode` 变体即可无限扩展（前缀隐含组件维度，全局唯一）。
- **Tag 不区分"结构 vs 值"**：所有结构操作都已精确化（`row_added/row_removed`），
  投影直接按具体 tag 变体做精确更新，无需 `is_structural()` 式的预分类。
  全量重建仅限初始化（`ScreenInitialized`）与纯边移动拖拽结束。
