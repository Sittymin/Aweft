use crate::depsgraph::Tag;
use crate::screen::area::{AreaType, ScrAreaKey};
use crate::store::state::AppState;
use anyhow::Result;

/// 处理区域类型切换
pub fn handle_switch_type(
    state: &mut AppState,
    area_key: ScrAreaKey,
    area_type: AreaType,
) -> Result<Vec<Tag>> {
    // 尝试获取并修改对应的 Area
    if let Some(area) = state.screen_map.areas.get_mut(area_key) {
        let old_type = area.area_type.clone();
        area.area_type = area_type.clone();

        log::info!("成功切换 Area {:?} 的类型为 {:?}", area_key, area.area_type);

        Ok(vec![Tag::AreaTypeChanged {
            area_key,
            old_type,
            new_type: area_type,
        }])
    } else {
        log::error!("错误：找不到 Area {:?}", area_key);
        Ok(vec![])
    }
}

/// 处理区域分割
pub fn handle_split(
    state: &mut AppState,
    area_key: ScrAreaKey,
    preview_data: crate::bridge::types::SplitPreviewOverlayData,
) -> Result<Vec<Tag>> {
    // 验证分割数据是否有效（避免0尺寸的区域）
    if preview_data.width2 <= 0.0 || preview_data.height2 <= 0.0 {
        log::warn!("分割数据无效（第二个区域尺寸为0），取消分割操作");
        return Ok(vec![]);
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

    log::info!("成功分割区域 {:?}", area_key);

    Ok(vec![Tag::AreasSplit {
        parent_key: area_key,
        new_area_key,
    }])
}

/// 处理区域合并
pub fn handle_join(
    state: &mut AppState,
    source_area_key: ScrAreaKey,
    target_area_key: ScrAreaKey,
    adjacency: crate::screen::primitives::Adjacency,
) -> Result<Vec<Tag>> {
    let theme_settings = state.theme_settings;

    // join 返回实际受影响集合（主合并 + 递归空位填充的删除、对齐裁剪的新增）
    let affected = state.screen_map.apply_join_adjacent_areas(
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

    log::info!(
        "成功合并区域 {:?} -> {:?}",
        source_area_key,
        target_area_key
    );

    Ok(vec![Tag::AreasJoined {
        removed_keys: affected.removed,
        added_keys: affected.added,
        merged_into: target_area_key,
    }])
}

impl crate::store::AppStore {
    pub(crate) fn dispatch_area(
        &mut self,
        action: crate::actions::area::AreaAction,
    ) -> Result<Vec<Tag>> {
        use crate::actions::area::AreaAction;

        match action {
            AreaAction::SwitchType {
                area_key,
                area_type,
            } => handle_switch_type(&mut self.state, area_key, area_type),
            AreaAction::Split {
                area_key,
                preview_data,
            } => handle_split(&mut self.state, area_key, preview_data),
            AreaAction::Join {
                source_area_key,
                target_area_key,
                adjacency,
            } => handle_join(&mut self.state, source_area_key, target_area_key, adjacency),
        }
    }
}
