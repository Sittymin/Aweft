use crate::bridge::types::{SlintKey, slint_area_key_to_key};
use crate::screen::area::ScrAreaKey;
use crate::store::state::{AppState, DragOperation};
use crate::store::state_change::{DragOperationChange, StateChange};
use anyhow::Result;
use slint::{Model, ModelRc};

/// 处理 ActionZone 拖拽更新
pub fn handle_actionzone_update(
    state: &mut AppState,
    area_key: SlintKey,
    corner: crate::bridge::types::ActionZoneCorner,
    offset_x: f32,
    offset_y: f32,
) -> Result<StateChange> {
    let scale_factor = state.theme_settings.get_scale_factor();
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length();

    let physical_offset_x = offset_x as f64 * scale_factor;
    let physical_offset_y = offset_y as f64 * scale_factor;

    let source_area_key: ScrAreaKey = slint_area_key_to_key(&area_key);

    // 防护检查：确保源区域仍然存在
    if !state.screen_map.areas.contains_key(source_area_key) {
        println!("警告：源区域 {:?} 已不存在，忽略拖拽事件", source_area_key);
        return Ok(StateChange::None);
    }

    if let Some(target_area_key) = state.screen_map.find_area_with_actionzone(
        source_area_key,
        corner,
        physical_offset_x,
        physical_offset_y,
        actionzone_side_length,
    ) {
        if target_area_key == source_area_key {
            // 在相同区域内 - 分割操作
            let (relative_offset_x, relative_offset_y) =
                state.screen_map.convert_corner_offset_to_relative_offset(
                    target_area_key,
                    corner,
                    physical_offset_x,
                    physical_offset_y,
                    actionzone_side_length,
                );

            let split_preview_overlay_data = state.screen_map.calculate_split_areas(
                target_area_key,
                physical_offset_x,
                physical_offset_y,
                relative_offset_x,
                relative_offset_y,
                &state.theme_settings,
            );

            // 更新拖拽状态
            state.current_drag_operation = DragOperation::Split {
                source_area: source_area_key,
                preview_data: split_preview_overlay_data,
            };

            Ok(StateChange::DragOperation(
                DragOperationChange::SplitPreviewUpdated,
            ))
        } else {
            // 移动到不同区域 - 合并操作
            if let Some(adjacency) = state.screen_map.detect_adjacency_by_keys(
                source_area_key,
                target_area_key,
                &state.theme_settings,
            ) {
                state.current_drag_operation = DragOperation::Join {
                    source_area_key,
                    target_area_key,
                    adjacency,
                };
                Ok(StateChange::DragOperation(
                    DragOperationChange::JoinPreviewUpdated,
                ))
            } else {
                // 非相邻：清除拖拽状态
                state.current_drag_operation = DragOperation::None;
                Ok(StateChange::DragOperation(
                    DragOperationChange::PreviewCleared,
                ))
            }
        }
    } else {
        // 离开任何区域
        state.current_drag_operation = DragOperation::None;
        Ok(StateChange::DragOperation(
            DragOperationChange::PreviewCleared,
        ))
    }
}

/// 处理 ActionZone 拖拽结束
pub fn handle_actionzone_end(
    state: &mut AppState,
    _area_key: SlintKey,
    is_cancel: bool,
) -> Result<StateChange> {
    if is_cancel {
        state.current_drag_operation = DragOperation::None;
        return Ok(StateChange::DragOperation(DragOperationChange::Ended {
            areas_changed: false,
        }));
    }

    // 提取拖拽状态，避免借用冲突
    let drag_operation = state.current_drag_operation.clone();

    let areas_changed = match drag_operation {
        DragOperation::Split {
            source_area,
            preview_data,
        } => {
            // 执行分割
            // HACK: 无视此函数的正常返回，!! 会导致未来撤销操作无法覆盖到分割
            crate::store::reducers::area::handle_split(state, source_area, preview_data)?;
            true
        }
        DragOperation::Join {
            source_area_key,
            target_area_key,
            adjacency,
        } => {
            // 执行合并
            // HACK: 无视此函数的正常返回，!! 会导致未来撤销操作无法覆盖到合并
            crate::store::reducers::area::handle_join(
                state,
                source_area_key,
                target_area_key,
                adjacency,
            )?;
            true
        }
        _ => false,
    };

    // 清除拖拽状态 (避免后续数据错误)
    state.current_drag_operation = DragOperation::None;

    Ok(StateChange::DragOperation(DragOperationChange::Ended {
        areas_changed,
    }))
}

/// 处理边缘拖拽更新
pub fn handle_edge_update(
    state: &mut AppState,
    vert_keys: ModelRc<SlintKey>,
    offset_x: f32,
    offset_y: f32,
) -> Result<StateChange> {
    use crate::screen::vertex::ScrVertKey;

    let scale_factor = state.theme_settings.get_scale_factor();
    let min_area_size = state.theme_settings.get_area_min_size();

    // 如果当前已经在边移动状态，继续移动
    if let DragOperation::MoveEdge {
        dir_axis_horizontal,
        origval,
        bigger,
        smaller,
        moving_vert_keys,
    } = &state.current_drag_operation.clone()
    {
        let theme_settings = state.theme_settings;
        let current_offset_x = offset_x as f64 - theme_settings.get_half_edge_width();
        let current_offset_y = offset_y as f64 - theme_settings.get_half_edge_width();

        match dir_axis_horizontal {
            true => {
                state.screen_map.apply_edge_move(
                    *dir_axis_horizontal,
                    moving_vert_keys,
                    *origval,
                    (current_offset_y * scale_factor).round() as i32,
                    *bigger,
                    *smaller,
                );
            }
            false => {
                state.screen_map.apply_edge_move(
                    *dir_axis_horizontal,
                    moving_vert_keys,
                    *origval,
                    (current_offset_x * scale_factor).round() as i32,
                    *bigger,
                    *smaller,
                );
            }
        }

        // 更新区域几何数据
        state.screen_map.update_all_areas_rcti(&theme_settings);

        return Ok(StateChange::DragOperation(DragOperationChange::EdgeUpdated));
    }

    // 开始新的边移动操作
    let moving_vert_keys: Vec<ScrVertKey> = vert_keys
        .iter()
        .map(|k| slint_area_key_to_key(&k))
        .collect();

    // 分析移动方向和原始坐标
    if let Some((dir_axis_horizontal, origval)) =
        state.screen_map.analyze_move_direction(&moving_vert_keys)
    {
        let (bigger, smaller) = state.screen_map.compute_move_limits(
            dir_axis_horizontal,
            &moving_vert_keys,
            origval,
            min_area_size,
        );

        // 设置拖拽状态
        state.current_drag_operation = DragOperation::MoveEdge {
            dir_axis_horizontal,
            origval,
            bigger,
            smaller,
            moving_vert_keys,
        };

        println!(
            "开始边移动操作: 方向={}, 原始值={}, 限制=({}, {})",
            if dir_axis_horizontal {
                "水平"
            } else {
                "垂直"
            },
            origval,
            bigger,
            smaller
        );
    }

    Ok(StateChange::DragOperation(DragOperationChange::EdgeUpdated))
}

/// 处理边缘拖拽结束
pub fn handle_edge_end(state: &mut AppState, is_cancel: bool) -> Result<StateChange> {
    let areas_changed = if let DragOperation::MoveEdge {
        dir_axis_horizontal,
        origval,
        bigger,
        smaller,
        moving_vert_keys,
    } = &state.current_drag_operation.clone()
    {
        let theme_settings = state.theme_settings;

        if is_cancel {
            // 取消：恢复到原始位置
            println!("取消边移动，恢复到原始位置");
            state.screen_map.apply_edge_move(
                *dir_axis_horizontal,
                moving_vert_keys,
                *origval,
                0, // delta = 0 恢复原位置
                *bigger,
                *smaller,
            );
            state.screen_map.update_all_areas_rcti(&theme_settings);
            false
        } else {
            // 确认：保持当前位置
            println!("确认边移动");
            state.screen_map.remove_duplicate_vertices();

            // 标记窗口边框边缘
            let current_rect = state.current_window_rect();
            state
                .screen_map
                .mark_window_border_edges(&current_rect, &theme_settings);

            true
        }
    } else {
        false
    };

    // 清除拖拽状态
    state.current_drag_operation = DragOperation::None;

    Ok(StateChange::DragOperation(DragOperationChange::Ended {
        areas_changed,
    }))
}
