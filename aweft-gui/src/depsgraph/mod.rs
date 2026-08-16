//! UI 依赖图：以数据为中心的最小化更新调度器。
//!
//! 借鉴 Blender Dependency Graph，采用三层结构：
//!
//! - **数据域（`DataId`）**：可被标记变更的权威数据。reducer 修改数据后对数据域打 tag。
//! - **投影组件（`Component`）**：一个 UI 视图的"求值方面"（如 LineEditor 的行列表、
//!   分组聚合、选中状态）。声明它依赖哪些数据域。
//! - **操作（`Operation`）**：原子更新动作（如"更新某行时间码"、"插入一行"）。
//!   操作挂载在组件下，通过 `depends_on` 声明对其他操作的显式依赖
//!   （Blender 的 Relation，跨组件或组件内统一机制，不按声明顺序自动连边）。
//!
//! 调度（`Depsgraph::evaluate`）：
//! 1. 从被 tag 的数据域出发，收集所有声明依赖它的组件；
//! 2. 组件内按 `handles`（TagKind）匹配入口操作；
//! 3. 从入口沿**全部出边**（显式 `depends_on` 解析的 Relation 边）BFS 扩散闭包；
//! 4. 拓扑排序后单轮执行——联动在构建期建图（`resolve_relations`），
//!    一次求值完成，无需"操作打新 tag 下一轮"。
//!
//! 相比手工 `StateChange → UIUpdateCommand → UIUpdater` 管线，投影自己声明
//! "我依赖什么数据、我的操作如何执行"，新增 area/投影时只注册组件与操作。

mod graph;
mod tag;

pub use graph::{Component, ComponentId, Depsgraph, Operation, OperationId};
pub use tag::{DataId, NodeGraphChange, NodeGraphKind, OperationCode, SelectionKind, Tag, TagKind};
