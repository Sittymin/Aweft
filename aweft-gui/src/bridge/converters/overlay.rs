use crate::bridge::types::{
    CloseAreaOverlayData, JoinHighlightOverlayData, SplitPreviewOverlayData, scale_close_overlays,
    scale_join_overlay, scale_split_overlay,
};
use crate::store::state::{AppState, DragOperation};

/// 转换分割预览覆盖层数据
pub fn convert_split_overlay(state: &AppState) -> Option<SplitPreviewOverlayData> {
    if let DragOperation::Split { preview_data, .. } = &state.current_drag_operation {
        let scale_factor = state.theme_settings.get_scale_factor();
        Some(scale_split_overlay(preview_data.clone(), scale_factor))
    } else {
        None
    }
}

/// 转换合并高亮覆盖层数据
pub fn convert_join_overlay(
    state: &AppState,
) -> Option<(JoinHighlightOverlayData, Vec<CloseAreaOverlayData>)> {
    if let DragOperation::Join {
        source_area_key,
        adjacency,
        ..
    } = &state.current_drag_operation
    {
        let scale_factor = state.theme_settings.get_scale_factor();

        match state.screen_map.preview_join_adjacent_areas(
            *source_area_key,
            *adjacency,
            &state.theme_settings,
        ) {
            Ok((join_data, close_datas)) => {
                let join_data_logical = scale_join_overlay(join_data, scale_factor);
                let close_datas_logical = scale_close_overlays(close_datas, scale_factor);
                Some((join_data_logical, close_datas_logical))
            }
            Err(e) => {
                eprintln!("生成合并预览失败: {}", e);
                None
            }
        }
    } else {
        None
    }
}

/// 转换关闭区域覆盖层数据
pub fn convert_close_overlays(state: &AppState) -> Vec<CloseAreaOverlayData> {
    if let Some((_, close_overlays)) = convert_join_overlay(state) {
        close_overlays
    } else {
        Vec::new()
    }
}
