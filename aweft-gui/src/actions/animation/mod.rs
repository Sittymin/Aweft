//! 动画域（Animation）：统一 `AnimationLibrary`（Action 曲线 + NLA 轨道）的操作入口。
//!
//! 数据域 `DataId::Animation` 单一（`Project.library.animation_library` 统一权威），
//! GUI 侧因 Area 表现形式不同保留两类编辑器（曲线/摄影表 vs NLA），但共享同一
//! 依赖图数据域（`TagKind` 隔离）。本模块聚合两类子 Action 为单一域入口，
//! 满足 `actions → reducers → projections → store` 四层同名约束。

pub mod graph;
pub mod nla;

pub use graph::GraphAction;
pub use nla::NlaAction;

/// 动画域统一 Action：曲线/摄影表（Graph）与 NLA 共享 `DataId::Animation`。
#[derive(Debug, Clone)]
pub enum AnimationAction {
    Graph(GraphAction),
    Nla(NlaAction),
}
