//! UI 投影：声明依赖的数据域 + 按 TagKind 路由的原子操作。
//!
//! 每个投影组件（`Component`）聚合一组操作（`Operation`），每个操作声明
//! `handles`（处理的 TagKind，作为依赖图入口）+ `code`（编译期操作标识，
//! 供跨组件 depends_on 引用）。tag 变化时 entry_ops 查表命中入口，沿出边
//! BFS 闭包 + 拓扑执行，单轮完成（构建期建图）。

use crate::depsgraph::{
    Component, ComponentId, DataId, Depsgraph, Operation, OperationCode, OperationId, TagKind,
};

mod line_editor;
mod screen;

pub use line_editor::{op_line_collapse, op_line_selection, op_line_structure, op_line_timecode};
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
