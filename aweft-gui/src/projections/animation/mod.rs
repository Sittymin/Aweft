//! 动画域投影：统一 `DataId::Animation`（Action 曲线 + NLA 轨道）的派生层更新。
//!
//! 权威数据在编辑数据层（`GraphEditorState`/`NlaEditorState`）与 model（`AnimationLibrary`）。
//! reducer 只改数据层 + 打精确 tag；各 op 按 TagKind 路由，把数据层同步到
//! Slint 绑定模型。`TagKind` 隔离使曲线/摄影表与 NLA 可独立局部更新，但共享
//! 同一 `DataId` 入口与拓扑依赖。

pub mod graph;
pub mod nla;

pub use graph::{
    op_graph_channel, op_graph_drag_ended, op_graph_keyframes, op_graph_scalars,
    op_graph_selection, op_graph_structure,
};
pub use nla::{
    op_nla_position, op_nla_selection, op_nla_strip_structure, op_nla_structure,
    op_nla_track_added, op_nla_track_properties,
};
