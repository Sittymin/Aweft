use crate::bridge::types::{ScrAreaData, key_to_slint_key, rust_area_type_to_slint};
use crate::screen::area::ScrAreaKey;
use crate::store::state::AppState;

/// 将内部区域数据转换为 Slint 需要的格式
pub fn convert_areas(state: &AppState) -> Vec<ScrAreaData> {
    let mut areas_data = Vec::new();
    let scale_factor = state.theme_settings.get_scale_factor();
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length() as f32;

    for (area_key, area) in &state.screen_map.areas {
        let (geometry_rect, winx, winy) = area.geometry();

        // 物理像素 -> 逻辑像素转换
        let logical_x = geometry_rect.xmin as f64 / scale_factor;
        let logical_y = geometry_rect.ymin as f64 / scale_factor;
        let logical_width = winx as f64 / scale_factor;
        let logical_height = winy as f64 / scale_factor;

        areas_data.push(ScrAreaData {
            area_key: key_to_slint_key(&area_key),
            area_type: rust_area_type_to_slint(&area.area_type),
            x: logical_x as f32,
            y: logical_y as f32,
            width: logical_width as f32,
            height: logical_height as f32,
            actionzone_side_length,
        });
    }
    areas_data
}

/// 单个区域转换
pub fn convert_area(state: &AppState, area_key: ScrAreaKey) -> Option<ScrAreaData> {
    let scale_factor = state.theme_settings.get_scale_factor();
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length() as f32;
    let area = state.screen_map.areas.get(area_key)?;
    let (geometry_rect, winx, winy) = area.geometry();

    let logical_x = geometry_rect.xmin as f64 / scale_factor;
    let logical_y = geometry_rect.ymin as f64 / scale_factor;
    let logical_width = winx as f64 / scale_factor;
    let logical_height = winy as f64 / scale_factor;

    Some(ScrAreaData {
        area_key: key_to_slint_key(&area_key),
        area_type: rust_area_type_to_slint(&area.area_type),
        x: logical_x as f32,
        y: logical_y as f32,
        width: logical_width as f32,
        height: logical_height as f32,
        actionzone_side_length,
    })
}
