use crate::DragOperation;
use crate::screen::area::ScrAreaKey;
use crate::store::state;
use crate::store::state_change::{AreasChange, DragOperationChange, ScreenChange, StateChange};

/// UI 更新指令 - 明确告诉 UI 层要做什么
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UIUpdateCommand {
    /// 更新所有区域
    UpdateAreas,

    /// 更新单个区域（精确更新）
    UpdateSingleArea(ScrAreaKey),

    /// 更新多个区域
    UpdateMultipleAreas(Vec<ScrAreaKey>),

    /// 更新所有边缘(需要注意 TouchArea 更新后将丢失已有焦点)
    UpdateEdges,
    UpdateEdgesOnlyDisplay,
    UpdateEdgesOnlyTouchArea,

    /// 更新覆盖层
    UpdateSplitOverlay,
    UpdateJoinOverlay,

    /// 隐藏覆盖层
    HideOverlays,

    /// 更新所有（回退方案）
    UpdateAll,
}

impl UIUpdateCommand {
    /// 从 StateChange 转换为 UI 更新指令
    pub fn from_state_change(change: &StateChange) -> Vec<Self> {
        match change {
            StateChange::None => vec![],

            StateChange::Screen(screen_change) => Self::from_screen_change(screen_change),

            StateChange::Areas(areas_change) => Self::from_areas_change(areas_change),

            StateChange::DragOperation(drag_change) => Self::from_drag_change(drag_change),

            StateChange::Multiple(changes) => {
                let mut commands = Vec::new();
                for change in changes {
                    commands.extend(Self::from_state_change(change));
                }
                Self::merge(commands)
            }
        }
    }

    fn from_screen_change(change: &ScreenChange) -> Vec<Self> {
        match change {
            ScreenChange::Initialized { .. } => vec![Self::UpdateAreas, Self::UpdateEdges],
            ScreenChange::Resized { .. } => vec![Self::UpdateAreas, Self::UpdateEdges],
            ScreenChange::ScaleChanged { .. } => vec![Self::UpdateAreas, Self::UpdateEdges],
        }
    }

    fn from_areas_change(change: &AreasChange) -> Vec<Self> {
        match change {
            AreasChange::TypeChanged { area_key, .. } => vec![Self::UpdateSingleArea(*area_key)],
            AreasChange::Split { .. } => {
                vec![Self::UpdateAreas, Self::UpdateEdges, Self::HideOverlays]
            }
            AreasChange::Joined { .. } => {
                vec![Self::UpdateAreas, Self::UpdateEdges, Self::HideOverlays]
            }
        }
    }

    fn from_drag_change(change: &DragOperationChange) -> Vec<Self> {
        match change {
            DragOperationChange::SplitPreviewUpdated => vec![Self::UpdateSplitOverlay],
            DragOperationChange::JoinPreviewUpdated => vec![Self::UpdateJoinOverlay],
            DragOperationChange::PreviewCleared => vec![Self::HideOverlays],
            DragOperationChange::EdgeUpdated => {
                vec![Self::UpdateAreas, Self::UpdateEdgesOnlyDisplay]
            }
            DragOperationChange::Ended {
                areas_changed: true,
            } => vec![Self::UpdateAreas, Self::UpdateEdges, Self::HideOverlays],
            DragOperationChange::Ended {
                areas_changed: false,
            } => vec![
                Self::UpdateAreas,
                Self::UpdateEdgesOnlyDisplay,
                Self::HideOverlays,
            ],
        }
    }

    /// 合并多个指令，优化更新
    pub fn merge(commands: Vec<Self>) -> Vec<Self> {
        use std::collections::HashSet;

        if commands.is_empty() {
            return vec![];
        }

        let mut has_update_all = false;
        let mut has_update_areas = false;
        let mut has_update_edges = false;
        let mut has_update_edges_only_display = false;
        let mut has_update_edges_only_touch_area = false;
        let mut has_update_split_overlay = false;
        let mut has_update_join_overlay = false;
        let mut has_hide_overlays = false;
        let mut single_area_keys = HashSet::new();

        for cmd in commands {
            match cmd {
                Self::UpdateAll => has_update_all = true,
                Self::UpdateAreas => has_update_areas = true,
                Self::UpdateEdges => has_update_edges = true,
                Self::UpdateEdgesOnlyDisplay => has_update_edges_only_display = true,
                Self::UpdateEdgesOnlyTouchArea => has_update_edges_only_touch_area = true,
                Self::HideOverlays => has_hide_overlays = true,
                Self::UpdateSingleArea(key) => {
                    single_area_keys.insert(key);
                }
                Self::UpdateMultipleAreas(keys) => {
                    single_area_keys.extend(keys);
                }
                Self::UpdateSplitOverlay => has_update_split_overlay = true,
                Self::UpdateJoinOverlay => has_update_join_overlay = true,
            }
        }

        // 如果有 UpdateAll，其他都不需要了
        if has_update_all {
            return vec![Self::UpdateAll];
        }

        let mut result = Vec::new();

        // 如果有 UpdateAreas，单个区域更新就不需要了
        if has_update_areas {
            result.push(Self::UpdateAreas);
        } else if !single_area_keys.is_empty() {
            result.push(Self::UpdateMultipleAreas(
                single_area_keys.into_iter().collect(),
            ));
        }

        if has_update_edges || (has_update_edges_only_touch_area && has_update_edges_only_display) {
            result.push(Self::UpdateEdges);
        } else if has_update_edges_only_display {
            result.push(Self::UpdateEdgesOnlyDisplay);
        } else if has_update_edges_only_touch_area {
            result.push(Self::UpdateEdgesOnlyTouchArea);
        }

        if has_update_split_overlay {
            result.push(Self::UpdateSplitOverlay);
        }

        if has_update_join_overlay {
            result.push(Self::UpdateJoinOverlay);
        }

        if has_hide_overlays {
            result.push(Self::HideOverlays);
        }

        result
    }
}
