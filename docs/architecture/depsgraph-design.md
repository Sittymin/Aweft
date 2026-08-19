# UI 依赖图设计

> 每个投影声明"依赖什么数据、操作如何执行"，tag 驱动**最小化 UI 更新**
> （避免 Slint 组件销毁重建）。新增 Area 只需注册一个投影。

## 数据流

```
UI 事件 → Action → dispatch（reducer 改数据 + 返回 Tag）→ evaluate → 投影写 UI 模型
```

## 核心概念

### DataId（数据域）

可被标记变更的权威数据，reducer 修改后打 tag。组件声明 `depends_on` 依赖；
tag 命中任一数据域即调度该组件，多个组件共享数据域 → 一次 tag 联动多个投影。
（数据域全集见 `depsgraph/tag.rs`。）

### Tag（变更）

带 payload 的变更描述，供投影精准局部更新。每个 Tag 归一到 `DataId`（决定哪些
组件被调度）+ `TagKind`（决定组件内哪个操作被路由）。元素级值变化 tag 携带
`item_id` + `ancestors`（祖先分组链），投影据此更新"该行"+"所有祖先分组头聚合"。

### Component / Operation（投影）

- **Component**：UI 视图的一个求值方面（行列表/分组聚合/选中态），声明 `depends_on`。
- **Operation**：原子更新动作，图节点自持 `exec`；`handles` 做入口路由，
  `depends_on` 做显式依赖边。**依赖只来自显式声明，不按声明顺序自动连边**
  （组件内亦然）。

### 调度（evaluate）

1. 构建期预建入口索引 `(DataId, TagKind) → 入口操作`（组件 `depends_on` × 操作
   `handles`），flush 时 O(1) 查表；
2. 从入口沿出边 BFS 扩散闭包（含跨组件边）；
3. Kahn 拓扑排序（稳定），依赖在前的先执行；
4. 单轮执行 + 清空 pending——一次 flush 即完整闭包，无需"打新 tag 下一轮"。
   跨组件边构建期解析，引用未注册操作 panic（构建期错误）。

### 与 Blender DEG 对应

| Blender | 我们 |
|---|---|
| IDNode（data-block） | `DataId` |
| IDNode tag → ComponentNode 依赖 | `Component::depends_on` |
| ComponentNode（求值方面） | `Component` |
| OperationNode（原子操作） | `Operation` |
| 跨组件 Relation 边 | `Operation::depends_on`（构建期解析） |
| entry_tags（构建期预建） | `(DataId, TagKind)` → 入口操作索引 |
| flush（沿关系边扩散闭包） | BFS 闭包 |
| 构建期建图 / 单轮求值 | register / evaluate |

## 投影实现规范

1. **值变化** → `notify_row_changed` 局部更新（含祖先分组头聚合传播）。
2. **结构变化** → 精确 `row_added/row_removed`（Repeater splice，存量复用）。
3. **布局变化** → 逐个 `set_row_data`；split/join → 原 area 保留 + 精确增删。
4. **初始化** → `set_vec` 全量重建（从空创建）。

**原则**：结构操作必须产生具体 tag，禁止全量兜底（收到未精确化的结构 tag 会
`log::error`）。全量重建属例外，须有明确理由（如从空创建、拓扑变化、切换链）。

## 已知取舍

- 跨组件依赖显式声明（Blender Relation），构建期解析、单轮求值；投影纯读数据 +
  写 UI 模型，不改数据、不产生新 tag。
- 重算函数用**函数指针**：避免 `Depsgraph` 持有 `AppStore` 引用造成循环借用。
- 依赖引用用**编译期枚举**而非运行时 id：可静态引用、拼错在编译期暴露，加变体即扩展。
- Tag 不区分"结构 vs 值"：结构操作已全部精确化，直接按 tag 变体更新。

## 相关文档

- Slint 局部更新、可复用 adapter、两层数据源 → `slint-update-patterns.md`
- 无头 UI 测试 → `headless-testing.md`
- 动画编辑器投影 → `animation-editor.md`
