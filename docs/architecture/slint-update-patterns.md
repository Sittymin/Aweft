# Slint 局部更新模式

投影更新 UI 模型的依据：优先行级 API，避免组件销毁重建。

## 更新成本

| 操作 | 通知 | Repeater 行为 | 组件影响 |
|---|---|---|---|
| `set_row_data` | `row_changed` | 原地 update | **零重建**（最优） |
| `insert`/`remove` | `row_added`/`row_removed` | splice，存量复用 | 局部 |
| `set_vec`/`clear`/换模型 | `reset` | 全量清空重建 | **兜底** |

## 可复用 adapter（免手动 index map）

| Adapter | 用途 | 最小化行为 |
|---|---|---|
| `MapModel` | 映射/类型转换 | 透传通知，零开销 |
| `FilterModel` | 条件过滤 | 逐行 diff |
| `SortModel` | 排序 | 单行移动 diff |
| `ReverseModel` | 倒序 | 纯算术转发 |

LineEditor：`FilterModel` 折叠过滤 + `MapModel` 映射，不手动维护 `key → index`。
（Screen：`VecModel` + 手动 `index_map`——edge 合并行多 key 共一行，几何驱动。）

## ListView 虚拟化

只实例化可见行，滚动复用、视口外销毁——不可见行 `set_row_data` 近乎零成本。
行数再多，ListView + `set_row_data` 就不怕全量重建。

## LineEditor 行数据组织：RowData + MapModel

`LineRowsModel` 只产出轻量行（锚点字段，O(1) 构造），`MapModel` 映射为完整 UI 行，
这样组织的原因：

1. **惰性聚合**：聚合值只在 Slint 拉取该行时算（滚动只算可见行）；
2. **类型解耦**：轻量行不含 Slint 类型，可独立测试；
3. **复用 MapModel**：类型转换 + 通知透传 + 懒加载。

通知必须由源模型发出，禁止直接往 UI 行模型链末端操作（会绕过 MapModel）。

## 逐帧重建的实时模型

实时例外通路（`before_rendering` 里重算的那类：帧图像、逐帧播放时钟、字幕行框）每帧都算一份
全量行数据，出口**必须先 diff 再写**：`set_row_data` 无条件发 `row_changed`，通知引发重绘，
重绘又回到 `before_rendering`——不比对旧值就把重绘循环自己转起来了。写法是逐行比、只在值不同
时 `set_row_data`，尾部多出来的行 `remove`，缺的 `push`；行数不变且逐个全等时一个通知都不发。
比对要求行类型 `PartialEq`（Slint 的 Rust 结构体已自带）。

这类模型同样禁用 `set_vec`/`clear`/换模型实例：它们是 `reset`，Repeater 会全量清空重建，逐帧
调用等于每帧销毁重建一遍组件。模型实例在 `setup` 时装一次（`slint::ModelRc::from(rc.clone())`），
之后只做行级操作。

## 其他

- Screen：`set_row_data`（几何局部）/ `insert`、`remove`（split/join）/ `set_vec`
  （仅初始化）；多 key 共一行用 `index_map`，插入/删除时平移。
- 跨模型共享状态用 `Rc<RefCell>`（零拷贝）。
- 拖拽预览写覆盖表不写数据层，commit/cancel 后清除并重算该行。

## 全量重建允许场景

1. 初始化（从空创建）。
2. 纯边移动拖拽结束（拓扑变化，边去重）。
3. 动画过滤视图重建后重新绑定 Slint global（FilterModel 捕获搜索词，旧实例不感知新词）。
4. 节点图切换链（派生链菜单 + 全量重建快照）。

其余场景必须走行级更新；结构操作必须产生精确 tag（未精确化会 `log::error`）。
