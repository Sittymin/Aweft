use crate::screen::area::{AreaType, ScrAreaKey};
use crate::screen::edge::ScrEdgeKey;
use crate::store::state_change::{
    AreasChange, DragOperationChange, LineEditorChange, ScreenChange, StateChange,
};
use aweft_model::ids::ItemId;
use aweft_model::structure::ItemLocation;
use chrono::TimeDelta;
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

    UpdateLineEditorGroupCollapsedToggled {
        group_id: String,
    },
    /// 更新某行时间码（同时刷新祖先分组的聚合时间）
    UpdateLineEditorTimecodeRowAndAncestors {
        event_id: ItemId,
        is_start_time: bool,
        time: TimeDelta,
    },
    /// 仅更新某行时间码（不刷新祖先聚合，常用于拖拽预览/取消）
    UpdateLineEditorTimecodeRowOnly {
        event_id: ItemId,
        is_start_time: bool,
        time: TimeDelta,
    },
    UpdateLineEditorSelectSingle {
        item_id: ItemId,
    },
    UpdateLineEditorSelectRange {
        item_id: ItemId,
    },
    UpdateLineEditorSelectToggle {
        item_id: ItemId,
    },
    UpdateLineEditorItemsInserted {
        item_id: ItemId,
        new_item_id: ItemId,
    },
    UpdateLineEditorItemsRemoved {
        item_ids: Vec<ItemId>,
    },
    /// 已复制 N 项（无 UI 投影变更，仅用于日志/未来 toast 提示）
    UpdateLineEditorItemsCopied {
        count: usize,
    },
    /// 已粘贴若干 root 子树。走 `handle_subtrees_pasted` 局部批量插入：
    /// 一次 flatten 所有子树 → 批量插入 VecModel → 标脏父级链更新行数与时间边界。
    UpdateLineEditorItemsPasted {
        new_root_ids: Vec<ItemId>,
        base_location: ItemLocation,
    },
    /// 已解散分组。走 `handle_group_ungrouped`：移除旧分组行 → 插入提升的子项行。
    UpdateLineEditorItemsUngrouped {
        group_id: ItemId,
        new_ids: Vec<ItemId>,
    },
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
            StateChange::LineEditor(line_editor_change) => {
                Self::from_line_editor_change(line_editor_change)
            }
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

    pub fn from_line_editor_change(change: LineEditorChange) -> Vec<Self> {
        match change {
            LineEditorChange::GroupCollapsedToggled { group_id } => {
                vec![Self::UpdateLineEditorGroupCollapsedToggled { group_id }]
            }
            LineEditorChange::TimecodeInputed {
                event_id,
                is_start_time,
                time,
            }
            | LineEditorChange::TimecodeIncremented {
                event_id,
                is_start_time,
                time,
            }
            | LineEditorChange::TimecodeDecremented {
                event_id,
                is_start_time,
                time,
            }
            | LineEditorChange::TimecodeCommitDraged {
                event_id,
                is_start_time,
                time,
            } => vec![Self::UpdateLineEditorTimecodeRowAndAncestors {
                event_id,
                is_start_time,
                time,
            }],
            LineEditorChange::TimecodePreviewDraged {
                event_id,
                is_start_time,
                time,
            }
            | LineEditorChange::TimecodeCancelDraged {
                event_id,
                is_start_time,
                time,
            } => vec![Self::UpdateLineEditorTimecodeRowOnly {
                event_id,
                is_start_time,
                time,
            }],
            LineEditorChange::TimecodesBatchPreviewDraged {
                event_ids,
                is_start_time,
                times,
            } => event_ids
                .into_iter()
                .zip(times.into_iter())
                .map(|(event_id, time)| Self::UpdateLineEditorTimecodeRowOnly {
                    event_id,
                    is_start_time,
                    time,
                })
                .collect(),
            LineEditorChange::SelectSingle { item_id } => {
                vec![Self::UpdateLineEditorSelectSingle { item_id }]
            }
            LineEditorChange::SelectRange { item_id } => {
                vec![Self::UpdateLineEditorSelectRange { item_id }]
            }
            LineEditorChange::SelectToggle { item_id } => {
                vec![Self::UpdateLineEditorSelectToggle { item_id }]
            }
            LineEditorChange::ItemsInserted {
                item_id,
                new_item_id,
            } => vec![Self::UpdateLineEditorItemsInserted {
                item_id,
                new_item_id,
            }],
            LineEditorChange::ItemsRemoved { item_ids } => {
                vec![Self::UpdateLineEditorItemsRemoved { item_ids }]
            }
            LineEditorChange::ItemsCopied { count } => {
                vec![Self::UpdateLineEditorItemsCopied { count }]
            }
            LineEditorChange::ItemsPasted {
                new_root_ids,
                base_location,
            } => vec![Self::UpdateLineEditorItemsPasted {
                new_root_ids,
                base_location,
            }],
            LineEditorChange::GroupsUngrouped { group_id, new_ids } => {
                vec![Self::UpdateLineEditorItemsUngrouped { group_id, new_ids }]
            }
        }
    }

    /// 合并多个指令，优化更新
    pub fn merge(commands: Vec<Self>) -> Vec<Self> {
        // TODO: 需要优秀实现
        commands
    }
}
