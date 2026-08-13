use crate::bridge::types::{key_to_slint_key, rust_area_type_to_slint, ScrAreaData};
use crate::screen::area::ScrAreaKey;
use crate::store::state::AppState;

/// 将内部区域数据转换为 Slint 需要的格式
pub fn convert_areas(state: &AppState) -> Vec<ScrAreaData> {
    let mut areas_data = Vec::new();
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length() as f32;

    for (area_key, area) in &state.screen_map.areas {
        let (layout, winx, winy) = area.layout_rect();

        // 物理像素 -> 逻辑像素转换
        let logical_x = state.theme_settings.to_logical_px(layout.xmin as f64);
        let logical_y = state.theme_settings.to_logical_px(layout.ymin as f64);
        let logical_width = state.theme_settings.to_logical_px(winx as f64);
        let logical_height = state.theme_settings.to_logical_px(winy as f64);

        areas_data.push(ScrAreaData {
            area_key: key_to_slint_key(&area_key),
            area_type: rust_area_type_to_slint(&area.area_type),
            x: logical_x,
            y: logical_y,
            width: logical_width,
            height: logical_height,
            actionzone_side_length,
        });
    }
    areas_data
}

/// 单个区域转换
pub fn convert_area(state: &AppState, area_key: ScrAreaKey) -> Option<ScrAreaData> {
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length() as f32;
    let area = state.screen_map.areas.get(area_key)?;
    let (layout, winx, winy) = area.layout_rect();

    let logical_x = state.theme_settings.to_logical_px(layout.xmin as f64);
    let logical_y = state.theme_settings.to_logical_px(layout.ymin as f64);
    let logical_width = state.theme_settings.to_logical_px(winx as f64);
    let logical_height = state.theme_settings.to_logical_px(winy as f64);

    Some(ScrAreaData {
        area_key: key_to_slint_key(&area_key),
        area_type: rust_area_type_to_slint(&area.area_type),
        x: logical_x,
        y: logical_y,
        width: logical_width,
        height: logical_height,
        actionzone_side_length,
    })
}
