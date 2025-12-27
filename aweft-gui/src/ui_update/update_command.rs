use crate::screen::area::{AreaType, ScrAreaKey};
use crate::screen::edge::ScrEdgeKey;
use crate::store::state_change::{AreasChange, DragOperationChange, ScreenChange, StateChange};
use std::rc::Rc;

/// UI 更新指令 - 明确告诉 UI 层要做什么
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UIUpdateCommand {
    /// 更新所有区域
    UpdateAreas,

    /// 更新单个区域几何（位置/尺寸）
    UpdateAreaGeometry(ScrAreaKey),
    // 更新区域类型
    UpdateAreaType {
        area_key: ScrAreaKey,
        new_type: AreaType,
    },

    /// 更新多个区域几何
    UpdateMultipleAreaGeometries(Rc<Vec<ScrAreaKey>>),

    /// 更新所有边缘(需要注意 TouchArea 更新后将丢失已有焦点)
    UpdateEdgesWithMerge,
    UpdateEdgesWithMergeOnlyDisplay,
    UpdateEdgesWithMergeOnlyTouchArea,
    /// 更新单个边（仅 Display，沿用已有合并结果）
    UpdateEdgeLocalOnlyDisplay(ScrEdgeKey),
    /// 更新多个边（仅 Display，沿用已有合并结果）
    UpdateMultipleEdgesLocalOnlyDisplay(Rc<Vec<ScrEdgeKey>>),

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
    pub fn from_state_change(change: StateChange) -> Vec<Self> {
        match change {
            StateChange::None => vec![],

            StateChange::Screen(screen_change) => Self::from_screen_change(screen_change),

            StateChange::Areas(areas_change) => Self::from_areas_change(areas_change),

            StateChange::DragOperation(drag_change) => Self::from_drag_change(drag_change),

            // 此部分为响应式，UI已经自动更新
            StateChange::LineEditor(_) => vec![],

            StateChange::Multiple(changes) => {
                let mut commands = Vec::new();
                for change in changes {
                    commands.extend(Self::from_state_change(change));
                }
                Self::merge(commands)
            }
        }
    }

    fn from_screen_change(change: ScreenChange) -> Vec<Self> {
        match change {
            // TODO: 这里没有必要全量计算 Edge 合并
            ScreenChange::Initialized { .. } => vec![Self::UpdateAreas, Self::UpdateEdgesWithMerge],
            ScreenChange::Resized { .. } => vec![Self::UpdateAreas, Self::UpdateEdgesWithMerge],
            ScreenChange::ScaleChanged { .. } => {
                vec![Self::UpdateAreas, Self::UpdateEdgesWithMerge]
            }
        }
    }

    fn from_areas_change(change: AreasChange) -> Vec<Self> {
        match change {
            AreasChange::TypeChanged {
                area_key,
                old_type: _,
                new_type,
            } => vec![Self::UpdateAreaType {
                area_key: area_key,
                new_type: new_type,
            }],
            AreasChange::Split { .. } => {
                vec![
                    Self::UpdateAreas,
                    Self::UpdateEdgesWithMerge,
                    Self::HideOverlays,
                ]
            }
            AreasChange::Joined { .. } => {
                vec![
                    Self::UpdateAreas,
                    Self::UpdateEdgesWithMerge,
                    Self::HideOverlays,
                ]
            }
        }
    }

    fn from_drag_change(change: DragOperationChange) -> Vec<Self> {
        match change {
            DragOperationChange::SplitPreviewUpdated => vec![Self::UpdateSplitOverlay],
            DragOperationChange::JoinPreviewUpdated => vec![Self::UpdateJoinOverlay],
            DragOperationChange::PreviewCleared => vec![Self::HideOverlays],
            DragOperationChange::EdgeUpdated {
                edge_keys_rc,
                area_keys_rc,
            } => {
                let mut cmds = Vec::new();
                if !edge_keys_rc.is_empty() {
                    cmds.push(Self::UpdateMultipleEdgesLocalOnlyDisplay(edge_keys_rc));
                }
                if !area_keys_rc.is_empty() {
                    cmds.push(Self::UpdateMultipleAreaGeometries(area_keys_rc));
                }
                cmds
            }
            DragOperationChange::Ended {
                areas_changed: true,
            } => vec![
                // TODO: 拆分 ActionZone 与 Edge 的 Drag 结束路径来优化不必要更新
                Self::UpdateAreas,
                Self::UpdateEdgesWithMerge,
                Self::HideOverlays,
            ],
            // 取消应用拖动结果
            DragOperationChange::Ended {
                areas_changed: false,
            } => vec![
                // TODO: 拆分 ActionZone 与 Edge 的 Drag 结束路径来优化不必要更新
                Self::UpdateAreas,
                Self::UpdateEdgesWithMergeOnlyDisplay,
                Self::HideOverlays,
            ],
        }
    }

    /// 合并多个指令，优化更新
    pub fn merge(commands: Vec<Self>) -> Vec<Self> {
        use std::collections::{HashMap, HashSet};

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
        let mut area_geom_keys = HashSet::new();
        let mut single_edge_keys = HashSet::new();
        let mut area_type_updates: HashMap<ScrAreaKey, AreaType> = HashMap::new();

        for cmd in commands {
            match cmd {
                Self::UpdateAll => has_update_all = true,
                Self::UpdateAreas => has_update_areas = true,
                Self::UpdateEdgesWithMerge => has_update_edges = true,
                Self::UpdateEdgesWithMergeOnlyDisplay => has_update_edges_only_display = true,
                Self::UpdateEdgesWithMergeOnlyTouchArea => has_update_edges_only_touch_area = true,
                Self::HideOverlays => has_hide_overlays = true,
                Self::UpdateAreaGeometry(key) => {
                    area_geom_keys.insert(key);
                }
                Self::UpdateMultipleAreaGeometries(keys) => {
                    area_geom_keys.extend(keys.iter().copied());
                }
                Self::UpdateEdgeLocalOnlyDisplay(key) => {
                    single_edge_keys.insert(key);
                }
                Self::UpdateMultipleEdgesLocalOnlyDisplay(keys) => {
                    single_edge_keys.extend(keys.iter().copied());
                }
                Self::UpdateSplitOverlay => has_update_split_overlay = true,
                Self::UpdateJoinOverlay => has_update_join_overlay = true,
                Self::UpdateAreaType { area_key, new_type } => {
                    area_type_updates.insert(area_key, new_type);
                }
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
        } else {
            // 如果没有全量更新，先应用区域类型的局部更新
            result.extend(
                area_type_updates
                    .into_iter()
                    .map(|(area_key, new_type)| Self::UpdateAreaType { area_key, new_type }),
            );

            if !area_geom_keys.is_empty() {
                result.push(Self::UpdateMultipleAreaGeometries(Rc::new(
                    area_geom_keys.into_iter().collect(),
                )));
            }
        }

        if has_update_edges || (has_update_edges_only_touch_area && has_update_edges_only_display) {
            result.push(Self::UpdateEdgesWithMerge);
        } else if has_update_edges_only_display {
            result.push(Self::UpdateEdgesWithMergeOnlyDisplay);
        } else if has_update_edges_only_touch_area {
            result.push(Self::UpdateEdgesWithMergeOnlyTouchArea);
        } else if !single_edge_keys.is_empty() {
            result.push(Self::UpdateMultipleEdgesLocalOnlyDisplay(Rc::new(
                single_edge_keys.into_iter().collect(),
            )));
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
