//! UI 投影：声明依赖的数据域 + 按 TagKind 路由的原子操作。
//!
//! 每个投影组件（`Component`）聚合一组操作（`Operation`），每个操作声明
//! `handles`（处理的 TagKind，作为依赖图入口）+ `code`（编译期操作标识，
//! 供跨组件 depends_on 引用）。tag 变化时 entry_ops 查表命中入口，沿出边
//! BFS 闭包 + 拓扑执行，单轮完成（构建期建图）。

use crate::depsgraph::{
    Component, ComponentId, DataId, Depsgraph, Operation, OperationCode, OperationId, TagKind,
};

mod animation;
mod line_editor;
mod nla;
mod node_graph;
mod screen;

pub use animation::{
    op_animation_channel, op_animation_drag_ended, op_animation_keyframes, op_animation_scalars,
    op_animation_selection, op_animation_structure,
};
pub use line_editor::{op_line_collapse, op_line_selection, op_line_structure, op_line_timecode};
pub use nla::{
    op_nla_position, op_nla_selection, op_nla_strip_structure, op_nla_structure,
    op_nla_track_added, op_nla_track_properties,
};
pub use node_graph::{
    op_node_graph_chain, op_node_graph_links, op_node_graph_socket, op_node_graph_structure,
};
pub use screen::{op_screen_layout, op_screen_overlays};

/// 注册所有投影组件到依赖图。在 `AppStore` 初始化时调用一次。
pub fn register_all(depsgraph: &mut Depsgraph) {
    depsgraph.register(Component {
        id: ComponentId::new(),
        depends_on: &[DataId::ProjectStructure, DataId::Selection],
        operations: vec![
            Operation {
                id: OperationId::new(),
                code: OperationCode::LineStructure,
                handles: &[TagKind::LineStructure],
                depends_on: &[],
                exec: op_line_structure,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::LineTimecode,
                handles: &[TagKind::LineTimecode],
                depends_on: &[],
                exec: op_line_timecode,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::LineCollapse,
                handles: &[TagKind::GroupCollapse],
                depends_on: &[],
                exec: op_line_collapse,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::LineSelection,
                handles: &[TagKind::Selection],
                // 依赖结构操作：粘贴/解散产生的选中 tag 需要行已插入
                // （reducer 打双 tag 声明联动，依赖边保证执行顺序）。
                depends_on: &[OperationCode::LineStructure],
                exec: op_line_selection,
            },
        ],
    });

    depsgraph.register(Component {
        id: ComponentId::new(),
        depends_on: &[DataId::NodeGraph],
        operations: vec![
            Operation {
                id: OperationId::new(),
                code: OperationCode::NodeGraphStructure,
                handles: &[TagKind::NodeGraphStructure],
                depends_on: &[],
                exec: op_node_graph_structure,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NodeGraphLinks,
                handles: &[TagKind::NodeGraphLinks],
                // 连线按 order 映射下标，order 由结构操作维护 → 依赖结构先跑
                depends_on: &[OperationCode::NodeGraphStructure],
                exec: op_node_graph_links,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NodeGraphSocket,
                handles: &[TagKind::NodeGraphSocket],
                // 单行定位依赖 order → 依赖结构先跑
                depends_on: &[OperationCode::NodeGraphStructure],
                exec: op_node_graph_socket,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NodeGraphChain,
                handles: &[TagKind::NodeGraphChain],
                depends_on: &[],
                exec: op_node_graph_chain,
            },
        ],
    });

    depsgraph.register(Component {
        id: ComponentId::new(),
        depends_on: &[DataId::Animation],
        operations: vec![
            Operation {
                id: OperationId::new(),
                code: OperationCode::AnimationStructure,
                handles: &[TagKind::AnimationStructure],
                depends_on: &[],
                exec: op_animation_structure,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::AnimationCurve,
                handles: &[TagKind::AnimationKeyframes],
                // 依赖结构操作：keyframe_models/过滤视图由 apply_structure 重建
                depends_on: &[OperationCode::AnimationStructure],
                exec: op_animation_keyframes,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::AnimationSelection,
                handles: &[TagKind::AnimationSelection],
                depends_on: &[OperationCode::AnimationStructure],
                exec: op_animation_selection,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::AnimationDragEnded,
                handles: &[TagKind::AnimationDragEnded],
                depends_on: &[OperationCode::AnimationStructure],
                exec: op_animation_drag_ended,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::AnimationChannel,
                handles: &[TagKind::AnimationChannel],
                depends_on: &[OperationCode::AnimationStructure],
                exec: op_animation_channel,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::AnimationScalars,
                handles: &[
                    TagKind::AnimationStructure,
                    TagKind::AnimationKeyframes,
                    TagKind::AnimationSelection,
                    TagKind::AnimationDragEnded,
                    TagKind::AnimationChannel,
                    TagKind::AnimationScalars,
                ],
                depends_on: &[],
                exec: op_animation_scalars,
            },
        ],
    });

    depsgraph.register(Component {
        id: ComponentId::new(),
        depends_on: &[DataId::Animation],
        operations: vec![
            Operation {
                id: OperationId::new(),
                code: OperationCode::NlaStructure,
                handles: &[TagKind::NlaStructure],
                depends_on: &[],
                exec: op_nla_structure,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NlaSelection,
                handles: &[TagKind::NlaSelection],
                // 依赖结构操作：strip_models 由 apply_structure 重建后才可更新选中态
                depends_on: &[OperationCode::NlaStructure],
                exec: op_nla_selection,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NlaStripStructure,
                handles: &[TagKind::NlaStripStructure],
                // 依赖结构操作：strip_models/过滤视图由 apply_structure 重建
                depends_on: &[OperationCode::NlaStructure],
                exec: op_nla_strip_structure,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NlaStripPosition,
                handles: &[TagKind::NlaStripPosition],
                // 位置更新依赖结构操作（移动/重排后的新下标），否则 set_row_data 会写错行
                depends_on: &[
                    OperationCode::NlaStructure,
                    OperationCode::NlaStripStructure,
                ],
                exec: op_nla_position,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NlaTrackProperties,
                handles: &[TagKind::NlaTrackProperties],
                // 依赖结构操作：tracks/strip_models 由 apply_structure 重建后才可更新行级属性
                depends_on: &[OperationCode::NlaStructure],
                exec: op_nla_track_properties,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::NlaTrackAdded,
                handles: &[TagKind::NlaTrackStructure],
                // 依赖结构操作：tracks/strip_models 由 apply_structure 重建后才可追加行
                depends_on: &[OperationCode::NlaStructure],
                exec: op_nla_track_added,
            },
        ],
    });

    depsgraph.register(Component {
        id: ComponentId::new(),
        depends_on: &[DataId::ScreenLayout, DataId::DragState],
        operations: vec![
            Operation {
                id: OperationId::new(),
                code: OperationCode::ScreenOverlays,
                // DragOverlay：拖拽中写预览；DragEnded：拖拽结束清除预览
                // （drag_operation 已置 None，write_overlays 走 None 分支清掉 overlay）。
                handles: &[TagKind::DragOverlay, TagKind::DragEnded],
                depends_on: &[],
                exec: op_screen_overlays,
            },
            Operation {
                id: OperationId::new(),
                code: OperationCode::ScreenLayout,
                handles: &[
                    TagKind::ScreenStructure,
                    TagKind::AreaType,
                    TagKind::DragEdgeMove,
                    TagKind::DragEnded,
                ],
                depends_on: &[],
                exec: op_screen_layout,
            },
        ],
    });

    // 跨组件 Relation 边：把 depends_on 的 "组件名/操作名" 解析为 OperationId。
    // Blender 构建期建图——此后单轮求值即可覆盖跨组件联动。
    depsgraph.resolve_relations();
}
