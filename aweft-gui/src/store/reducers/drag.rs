use crate::bridge::types::{slint_area_key_to_key, SlintKey};
use crate::depsgraph::Tag;
use crate::screen::area::ScrAreaKey;
use crate::screen::edge::ScrEdgeKey;
use crate::store::state::{AppState, DragOperation};
use anyhow::Result;
use slint::{Model, ModelRc};
use std::rc::Rc;

/// 处理 ActionZone 拖拽更新
pub fn handle_actionzone_update(
    state: &mut AppState,
    area_key: SlintKey,
    corner: crate::bridge::types::ActionZoneCorner,
    offset_x: f32,
    offset_y: f32,
) -> Result<Vec<Tag>> {
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length();

    let physical_offset_x = state.theme_settings.to_physical_px(offset_x as f64);
    let physical_offset_y = state.theme_settings.to_physical_px(offset_y as f64);

    let source_area_key: ScrAreaKey = slint_area_key_to_key(&area_key);

    // 防护检查：确保源区域仍然存在
    if !state.screen_map.areas.contains_key(source_area_key) {
        log::warn!("警告：源区域 {:?} 已不存在，忽略拖拽事件", source_area_key);
        return Ok(vec![]);
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

            Ok(vec![Tag::SplitPreviewUpdated])
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
                Ok(vec![Tag::JoinPreviewUpdated])
            } else {
                // 非相邻：清除拖拽状态
                state.current_drag_operation = DragOperation::None;
                Ok(vec![Tag::PreviewCleared])
            }
        }
    } else {
        // 离开任何区域
        state.current_drag_operation = DragOperation::None;
        Ok(vec![Tag::PreviewCleared])
    }
}

/// 处理 ActionZone 拖拽结束
pub fn handle_actionzone_end(
    state: &mut AppState,
    _area_key: SlintKey,
    is_cancel: bool,
) -> Result<Vec<Tag>> {
    if is_cancel {
        state.current_drag_operation = DragOperation::None;
        return Ok(vec![Tag::DragEnded {
            areas_changed: false,
            topology_changed: false,
        }]);
    }

    // 提取拖拽状态，避免借用冲突
    let drag_operation = state.current_drag_operation.clone();

    let mut tags = Vec::new();

    let areas_changed = match drag_operation {
        DragOperation::Split {
            source_area,
            preview_data,
        } => {
            // 执行分割
            // HACK: 无视此函数的正常返回，!! 会导致未来撤销操作无法覆盖到分割
            tags.extend(crate::store::reducers::area::handle_split(
                state,
                source_area,
                preview_data,
            )?);
            true
        }
        DragOperation::Join {
            source_area_key,
            target_area_key,
            adjacency,
        } => {
            // 执行合并
            // HACK: 无视此函数的正常返回，!! 会导致未来撤销操作无法覆盖到合并
            tags.extend(crate::store::reducers::area::handle_join(
                state,
                source_area_key,
                target_area_key,
                adjacency,
            )?);
            true
        }
        _ => false,
    };

    // 清除拖拽状态 (避免后续数据错误)
    state.current_drag_operation = DragOperation::None;

    tags.push(Tag::DragEnded {
        areas_changed,
        topology_changed: false,
    });
    Ok(tags)
}

/// 处理边缘拖拽更新
pub fn handle_edge_update(
    state: &mut AppState,
    // 变化位置 vert
    vert_keys: ModelRc<SlintKey>,
    // 此坐标是相较于 UI Edge 矩形左上角的
    offset_x: f32,
    offset_y: f32,
) -> Result<Vec<Tag>> {
    use crate::screen::vertex::ScrVertKey;
    use std::collections::HashSet;

    let min_area_size = state.theme_settings.get_area_min_size();

    // 如果当前已经在边移动状态，继续移动
    if let DragOperation::MoveEdge {
        dir_axis_horizontal,
        origval,
        bigger,
        smaller,
        moving_vert_keys,
        edge_keys_rc,
        area_keys_rc,
    } = &state.current_drag_operation.clone()
    {
        let theme_settings = state.theme_settings;

        match dir_axis_horizontal {
            true => {
                // 将偏移变为相对于中心的
                let current_offset_y = offset_y as f64 - theme_settings.get_half_edge_width();
                state.screen_map.apply_verts_move(
                    *dir_axis_horizontal,
                    moving_vert_keys,
                    *origval,
                    theme_settings.to_physical_px(current_offset_y).round() as i32,
                    *bigger,
                    *smaller,
                );
            }
            false => {
                let current_offset_x = offset_x as f64 - theme_settings.get_half_edge_width();
                state.screen_map.apply_verts_move(
                    *dir_axis_horizontal,
                    moving_vert_keys,
                    *origval,
                    theme_settings.to_physical_px(current_offset_x).round() as i32,
                    *bigger,
                    *smaller,
                );
            }
        }

        // 更新区域几何数据
        state.screen_map.update_all_areas_rcti(&theme_settings);

        return Ok(vec![Tag::EdgeUpdated {
            edge_keys: edge_keys_rc.clone(),
            area_keys: area_keys_rc.clone(),
        }]);
    }

    // 开始新的边移动操作
    let moving_vert_keys: Vec<ScrVertKey> = vert_keys
        .iter()
        .map(|k| slint_area_key_to_key(&k))
        .collect();

    // 分析移动方向和原始坐标
    let (dir_axis_horizontal, origval) =
        state.screen_map.analyze_move_direction(&moving_vert_keys)?;
    // 计算可移动范围
    let (bigger, smaller) = state.screen_map.compute_move_limits(
        dir_axis_horizontal,
        &moving_vert_keys,
        origval,
        min_area_size,
    );

    // 计算受影响的边和区域 （仅仅开始 Drag 需要计算，后续可以复用）
    let moving_set: HashSet<ScrVertKey> = moving_vert_keys.iter().copied().collect();
    let mut edge_keys_set = HashSet::new();
    for (ek, e) in &state.screen_map.edges {
        if moving_set.contains(&e.v1) || moving_set.contains(&e.v2) {
            edge_keys_set.insert(ek);
        }
    }
    let mut area_keys_set = HashSet::new();
    for (ak, a) in &state.screen_map.areas {
        let verts = [
            a.v_top_left(),
            a.v_top_right(),
            a.v_bottom_right(),
            a.v_bottom_left(),
        ];
        if verts.iter().any(|v| moving_set.contains(v)) {
            area_keys_set.insert(ak);
        }
    }

    let edge_keys_rc: Rc<Vec<ScrEdgeKey>> = Rc::new(edge_keys_set.into_iter().collect());
    let area_keys_rc: Rc<Vec<ScrAreaKey>> = Rc::new(area_keys_set.into_iter().collect());

    // 设置拖拽状态
    state.current_drag_operation = DragOperation::MoveEdge {
        dir_axis_horizontal,
        origval,
        bigger,
        smaller,
        moving_vert_keys: moving_vert_keys.clone(),
        edge_keys_rc: edge_keys_rc.clone(),
        area_keys_rc: area_keys_rc.clone(),
    };

    log::info!(
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

    Ok(vec![Tag::EdgeUpdated {
        edge_keys: edge_keys_rc,
        area_keys: area_keys_rc,
    }])
}

/// 处理边缘拖拽结束
pub fn handle_edge_end(state: &mut AppState, is_cancel: bool) -> Result<Vec<Tag>> {
    let (areas_changed, topology_changed) =
        if let DragOperation::MoveEdge {
            dir_axis_horizontal,
            origval,
            bigger,
            smaller,
            moving_vert_keys,
            edge_keys_rc: _,
            area_keys_rc: _,
        } = &state.current_drag_operation.clone()
        {
            let theme_settings = state.theme_settings;

            if is_cancel {
                // 取消：恢复到原始位置
                log::info!("取消边移动，恢复到原始位置");
                state.screen_map.apply_verts_move(
                    *dir_axis_horizontal,
                    moving_vert_keys,
                    *origval,
                    0, // delta = 0 恢复原位置
                    *bigger,
                    *smaller,
                );
                state.screen_map.update_all_areas_rcti(&theme_settings);
                (false, false)
            } else {
                // 确认：保持当前位置
                log::info!("确认边移动");
                // 顶点合并会改变边结构（才需全量重建 edges）
                let topology_changed = state.screen_map.remove_duplicate_vertices();

                // 标记窗口边框边缘
                let current_rect = state.current_window_rect();
                state
                    .screen_map
                    .mark_window_border_edges(&current_rect, &theme_settings);

                (true, topology_changed)
            }
        } else {
            (false, false)
        };

    // 清除拖拽状态
    state.current_drag_operation = DragOperation::None;

    Ok(vec![Tag::DragEnded {
        areas_changed,
        topology_changed,
    }])
}
