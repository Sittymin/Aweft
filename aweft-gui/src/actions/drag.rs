use slint::ModelRc;

use crate::types::{ActionZoneCorner, SlintKey};

/// 拖拽相关 Action
#[derive(Debug, Clone)]
pub enum DragAction {
    /// ActionZone 拖拽（区域角落）
    ActionZone(ActionZoneDrag),
    /// 边缘拖拽
    Edge(EdgeDrag),
}

/// ActionZone 拖拽的生命周期
#[derive(Debug, Clone)]
pub enum ActionZoneDrag {
    /// 拖拽中
    Update {
        area_key: SlintKey,
        corner: ActionZoneCorner,
        offset_x: f32,
        offset_y: f32,
    },
    /// 结束拖拽
    End { area_key: SlintKey, is_cancel: bool },
}

/// 边缘拖拽的生命周期
#[derive(Debug, Clone)]
pub enum EdgeDrag {
    /// 拖拽中
    Update {
        vert_keys: ModelRc<SlintKey>,
        offset_x: f32,
        offset_y: f32,
    },
    /// 结束拖拽
    End { is_cancel: bool },
}
