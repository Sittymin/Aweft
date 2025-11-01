use crate::bridge::converters;
use crate::screen::area::ScrAreaKey;
use crate::store::AppStore;
use crate::types::MainWindow;
use crate::ui_update::UIUpdateCommand;

/// UI 更新执行器
pub struct UIUpdater;

impl UIUpdater {
    /// 执行 UI 更新指令
    pub fn execute(instance: &MainWindow, store: &AppStore, commands: &[UIUpdateCommand]) {
        let state = store.state();

        for command in commands {
            match command {
                UIUpdateCommand::UpdateAll => {
                    Self::update_all(instance, store);
                }

                UIUpdateCommand::UpdateAreas => {
                    let areas = converters::convert_areas(state);
                    instance.set_scrareas(areas.as_slice().into());
                }

                UIUpdateCommand::UpdateSingleArea(area_key) => {
                    Self::update_single_area(instance, store, *area_key);
                }

                UIUpdateCommand::UpdateMultipleAreas(keys) => {
                    Self::update_multiple_areas(instance, store, keys);
                }

                UIUpdateCommand::UpdateEdges => {
                    let (edges_display, edges_touch) = converters::convert_edges_both(state);
                    instance.set_scredgedisplays(edges_display.as_slice().into());
                    instance.set_scredgetouchareas(edges_touch.as_slice().into());
                }
                UIUpdateCommand::UpdateEdgesOnlyDisplay => {
                    let edges_display = converters::convert_edges_display(state);
                    instance.set_scredgedisplays(edges_display.as_slice().into());
                }

                UIUpdateCommand::UpdateEdgesOnlyTouchArea => {
                    let edges_touch = converters::convert_edges_touch(state);
                    instance.set_scredgetouchareas(edges_touch.as_slice().into());
                }

                UIUpdateCommand::UpdateSplitOverlay => {
                    Self::update_split_overlay(instance, state);
                }
                UIUpdateCommand::UpdateJoinOverlay => {
                    Self::update_join_overlay(instance, state);
                }

                UIUpdateCommand::HideOverlays => {
                    instance.set_show_split_preview_overlay(false);
                    instance.set_show_join_highlight_overlay(false);
                    instance.set_show_close_area_overlays(false);
                }
            }
        }
    }

    fn update_single_area(instance: &MainWindow, store: &AppStore, _area_key: ScrAreaKey) {
        // 🎯 精确更新：只更新一个区域
        // 当前实现：重新设置整个数组（Slint 会内部优化）
        // TODO: 未来优化 - 使用 Model::row_changed()

        let state = store.state();
        let areas = converters::convert_areas(state);
        instance.set_scrareas(areas.as_slice().into());
    }

    fn update_multiple_areas(instance: &MainWindow, store: &AppStore, _keys: &[ScrAreaKey]) {
        // 当前实现：重新设置整个数组
        // TODO: 未来优化 - 批量更新指定区域

        let state = store.state();
        let areas = converters::convert_areas(state);
        instance.set_scrareas(areas.as_slice().into());
    }

    fn update_split_overlay(instance: &MainWindow, state: &crate::store::state::AppState) {
        if let Some(split_overlay) = converters::convert_split_overlay(state) {
            instance.set_split_preview_overlay_data(split_overlay);
            instance.set_show_split_preview_overlay(true);
        } else {
            instance.set_show_split_preview_overlay(false);
        }
    }
    fn update_join_overlay(instance: &MainWindow, state: &crate::store::state::AppState) {
        // NOTE: 如果计算邻接关系失败会返回 None
        if let Some((join_overlay, close_overlays)) = converters::convert_join_overlay(state) {
            instance.set_join_highlight_overlay_data(join_overlay);
            instance.set_show_join_highlight_overlay(true);
            instance.set_close_area_overlay_datas(close_overlays.as_slice().into());
            instance.set_show_close_area_overlays(!close_overlays.is_empty());
        } else {
            instance.set_show_join_highlight_overlay(false);
            instance.set_show_close_area_overlays(false);
        }
    }

    fn update_all(instance: &MainWindow, store: &AppStore) {
        let state = store.state();

        let areas = converters::convert_areas(state);
        instance.set_scrareas(areas.as_slice().into());

        let (edges_display, edges_touch) = converters::convert_edges_both(state);
        instance.set_scredgedisplays(edges_display.as_slice().into());
        instance.set_scredgetouchareas(edges_touch.as_slice().into());
    }
}
