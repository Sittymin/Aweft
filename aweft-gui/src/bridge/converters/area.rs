use crate::bridge::types::{ScrAreaData, key_to_slint_key, rust_area_type_to_slint};
use crate::screen::area::{AreaType, ScrAreaKey};
use crate::store::AppStore;

/// 插件 Area 的两个落点字段：id 给切换菜单认当前项，工厂给容器嵌面板。
/// 工厂按插件 id 缓存在桥里，取回的是同一个值（`ComponentFactory` 按指针比较），
/// 所以投影重算不会让已经嵌好的面板重建；没有可用面板时退回空工厂。
fn plugin_fields(
    store: &AppStore,
    area_type: &AreaType,
) -> (slint::SharedString, slint::ComponentFactory) {
    match area_type {
        AreaType::Plugin(id) => (
            slint::SharedString::from(id.as_str()),
            store.plugin_bridge.panel_factory(id).unwrap_or_default(),
        ),
        _ => Default::default(),
    }
}

/// 将内部区域数据转换为 Slint 需要的格式
pub fn convert_areas(store: &AppStore) -> Vec<ScrAreaData> {
    let mut areas_data = Vec::new();
    let state = store.state();
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length() as f32;

    for (area_key, area) in &state.screen_map.areas {
        let (layout, winx, winy) = area.layout_rect();

        // 物理像素 -> 逻辑像素转换
        let logical_x = state.theme_settings.to_logical_px(layout.xmin as f64);
        let logical_y = state.theme_settings.to_logical_px(layout.ymin as f64);
        let logical_width = state.theme_settings.to_logical_px(winx as f64);
        let logical_height = state.theme_settings.to_logical_px(winy as f64);
        let (plugin_id, plugin_factory) = plugin_fields(store, &area.area_type);

        areas_data.push(ScrAreaData {
            area_key: key_to_slint_key(&area_key),
            area_type: rust_area_type_to_slint(&area.area_type),
            x: logical_x,
            y: logical_y,
            width: logical_width,
            height: logical_height,
            actionzone_side_length,
            plugin_id,
            plugin_factory,
        });
    }
    areas_data
}

/// 单个区域转换
pub fn convert_area(store: &AppStore, area_key: ScrAreaKey) -> Option<ScrAreaData> {
    let state = store.state();
    let actionzone_side_length = state.theme_settings.get_actionzone_side_length() as f32;
    let area = state.screen_map.areas.get(area_key)?;
    let (layout, winx, winy) = area.layout_rect();

    let logical_x = state.theme_settings.to_logical_px(layout.xmin as f64);
    let logical_y = state.theme_settings.to_logical_px(layout.ymin as f64);
    let logical_width = state.theme_settings.to_logical_px(winx as f64);
    let logical_height = state.theme_settings.to_logical_px(winy as f64);
    let (plugin_id, plugin_factory) = plugin_fields(store, &area.area_type);

    Some(ScrAreaData {
        area_key: key_to_slint_key(&area_key),
        area_type: rust_area_type_to_slint(&area.area_type),
        x: logical_x,
        y: logical_y,
        width: logical_width,
        height: logical_height,
        actionzone_side_length,
        plugin_id,
        plugin_factory,
    })
}
