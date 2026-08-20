//! 动画域状态：统一 `AnimationLibrary` 的 UI 侧两类编辑器状态。
//!
//! 数据域 `DataId::Animation` 单一，但因 Area 表现形式不同，保留两组派生状态：
//! - `graph`（曲线编辑器/摄影表）：`GraphEditorState`（通道/关键帧/段/手柄）
//! - `nla`（NLA 编辑器）：`NlaEditorState`（轨道/片断）
//! 共享同一依赖图数据域，`TagKind` 隔离；`UiModels.animation` 聚合二者，满足
//! `store/animation` 与 `actions/animation`、`reducers/animation`、
//! `projections/animation` 四层同名约束。

pub mod graph;
pub mod nla;

pub use graph::GraphEditorState;
pub use nla::NlaEditorState;

/// 动画域聚合状态：曲线编辑器 + NLA 编辑器。
#[derive(Default)]
pub struct AnimationState {
    pub graph: GraphEditorState,
    pub nla: NlaEditorState,
}
