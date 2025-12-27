use crate::screen::area::{AreaType, ScrAreaKey};
use crate::store::state::AppState;
use crate::store::state_change::{AreasChange, StateChange};
use anyhow::Result;

/// 处理区域类型切换
pub fn handle_switch_type(
    state: &mut AppState,
    area_key: ScrAreaKey,
    area_type: AreaType,
) -> Result<StateChange> {
    // 尝试获取并修改对应的 Area
    if let Some(area) = state.screen_map.areas.get_mut(area_key) {
        let old_type = area.area_type.clone();
        area.area_type = area_type.clone();

        println!("成功切换 Area {:?} 的类型为 {:?}", area_key, area.area_type);

        Ok(StateChange::Areas(AreasChange::TypeChanged {
            area_key: area_key,
            old_type,
            new_type: area_type,
        }))
    } else {
        eprintln!("错误：找不到 Area {:?}", area_key);
        Ok(StateChange::None)
    }
}

/// 处理区域分割
pub fn handle_split(
    state: &mut AppState,
    area_key: ScrAreaKey,
    preview_data: crate::bridge::types::SplitPreviewOverlayData,
) -> Result<StateChange> {
    // 验证分割数据是否有效（避免0尺寸的区域）
    if preview_data.width2 <= 0.0 || preview_data.height2 <= 0.0 {
        println!("分割数据无效（第二个区域尺寸为0），取消分割操作");
        return Ok(StateChange::None);
    }

    let theme_settings = state.theme_settings;

    let new_area_key = state
        .screen_map
        .split_area(area_key, preview_data, &theme_settings)?;

    // 标记窗口边框边缘
    let current_rect = state.current_window_rect();
    state
        .screen_map
        .mark_window_border_edges(&current_rect, &theme_settings);

    println!("成功分割区域 {:?}", area_key);

    Ok(StateChange::Areas(AreasChange::Split {
        parent_key: area_key,
        new_area_key,
    }))
}

/// 处理区域合并
pub fn handle_join(
    state: &mut AppState,
    source_area_key: ScrAreaKey,
    target_area_key: ScrAreaKey,
    adjacency: crate::screen::primitives::Adjacency,
) -> Result<StateChange> {
    let theme_settings = state.theme_settings;

    // 记录将要被移除的区域
    let removed_keys = vec![source_area_key];

    state.screen_map.apply_join_adjacent_areas(
        source_area_key,
        target_area_key,
        adjacency,
        &theme_settings,
    )?;

    // 重新计算所有区域的几何数据
    state.screen_map.update_all_areas_rcti(&theme_settings);

    // 标记窗口边框边缘
    let current_rect = state.current_window_rect();
    state
        .screen_map
        .mark_window_border_edges(&current_rect, &theme_settings);

    println!(
        "成功合并区域 {:?} -> {:?}",
        source_area_key, target_area_key
    );

    // 输出调试信息
    println!("合并后的所有区域信息:");
    for (area_key, area) in &state.screen_map.areas {
        let (rect, _, _) = area.geometry();
        println!(
            "  区域 {:?}: 位置=({}, {}, {}, {})",
            area_key, rect.xmin, rect.ymin, rect.xmax, rect.ymax
        );
    }

    Ok(StateChange::Areas(AreasChange::Joined {
        removed_keys,
        merged_into: target_area_key,
    }))
}
