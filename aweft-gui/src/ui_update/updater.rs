use crate::bridge::converters;
use crate::bridge::types::MainWindow;
use crate::bridge::types::slint_area_key_to_key;
use crate::screen::area::ScrAreaKey;
use crate::screen::edge::ScrEdgeKey;
use crate::store::AppStore;
use crate::ui_update::UIUpdateCommand;
use slint::Model;

/// UI 更新执行器
pub struct UIUpdater;

impl UIUpdater {
    /// 执行 UI 更新指令
    pub fn execute(instance: &MainWindow, store: &mut AppStore, commands: Vec<UIUpdateCommand>) {
        for command in commands {
            match command {
                UIUpdateCommand::UpdateAll => {
                    Self::update_all(instance, store);
                }

                UIUpdateCommand::UpdateAreas => {
                    let areas = {
                        let state = store.state();
                        converters::convert_areas(state)
                    };
                    store.ui_models_mut().set_scrareas(areas);
                }

                UIUpdateCommand::UpdateAreaGeometry(area_key) => {
                    Self::update_area_geometry(store, &area_key);
                }

                UIUpdateCommand::UpdateMultipleAreaGeometries(keys) => {
                    for key in keys.iter() {
                        Self::update_area_geometry(store, key);
                    }
                }

                // 包含 Edge 合并计算
                UIUpdateCommand::UpdateEdgesWithMerge => {
                    let (edges_display, edges_touch) = {
                        let state = store.state();
                        converters::convert_all_edges_both(state)
                    };
                    let ui_models = store.ui_models_mut();
                    ui_models.set_scredge_displays(edges_display);
                    ui_models.set_scredge_touch_areas(edges_touch);
                }
                // 包含 Edge 合并计算
                UIUpdateCommand::UpdateEdgesWithMergeOnlyDisplay => {
                    let edges_display = {
                        let state = store.state();
                        converters::convert_all_edges_display(state)
                    };
                    store.ui_models_mut().set_scredge_displays(edges_display);
                }
                // 包含 Edge 合并计算
                UIUpdateCommand::UpdateEdgesWithMergeOnlyTouchArea => {
                    let edges_touch = {
                        let state = store.state();
                        converters::convert_all_edges_touch(state)
                    };
                    store.ui_models_mut().set_scredge_touch_areas(edges_touch);
                }
                // 沿用之前的 Edge 合并数据（仅 Display，拖动时不更新 Touch）
                UIUpdateCommand::UpdateEdgeLocalOnlyDisplay(edge_key) => {
                    Self::update_edges_only_display(store, &[edge_key]);
                }
                // 沿用之前的 Edge 合并数据（仅 Display，拖动时不更新 Touch）
                UIUpdateCommand::UpdateMultipleEdgesLocalOnlyDisplay(keys) => {
                    Self::update_edges_only_display(store, &keys);
                }

                UIUpdateCommand::UpdateSplitOverlay => {
                    let state = store.state();
                    Self::update_split_overlay(instance, state);
                }
                UIUpdateCommand::UpdateJoinOverlay => {
                    let state = store.state();
                    Self::update_join_overlay(instance, state);
                }

                UIUpdateCommand::HideOverlays => {
                    instance.set_show_split_preview_overlay(false);
                    instance.set_show_join_highlight_overlay(false);
                    instance.set_show_close_area_overlays(false);
                }
                UIUpdateCommand::UpdateAreaType { area_key, new_type } => {
                    let slint_type = crate::bridge::types::rust_area_type_to_slint(&new_type);
                    store
                        .ui_models_mut()
                        .update_area_type(&area_key, slint_type);
                }
                UIUpdateCommand::UpdateLineEditorGroupCollapsedToggled { group_id } => {
                    store
                        .ui_models_mut()
                        .line_editor_state
                        .toggle_group(&group_id);
                }
                UIUpdateCommand::UpdateLineEditorTimecodeRowAndAncestors {
                    event_id,
                    is_start_time,
                    time,
                } => {
                    let (state, ui_models) = store.split_mut();
                    ui_models.line_editor_state.update_row_time(
                        &state.project,
                        event_id,
                        is_start_time,
                        time,
                    );
                }
                UIUpdateCommand::UpdateLineEditorTimecodeRowOnly {
                    event_id,
                    is_start_time,
                    time,
                } => {
                    store
                        .ui_models_mut()
                        .line_editor_state
                        .update_row_time_without_ancestor_groups(event_id, is_start_time, time);
                }
                UIUpdateCommand::UpdateLineEditorSelectSingle { item_id } => {
                    let (state, ui_models) = store.split_mut();
                    ui_models
                        .line_editor_state
                        .select_single(&state.project, item_id);
                }
                UIUpdateCommand::UpdateLineEditorSelectRange { item_id } => {
                    let (state, ui_models) = store.split_mut();
                    ui_models
                        .line_editor_state
                        .select_range(&state.project, item_id);
                }
                UIUpdateCommand::UpdateLineEditorSelectToggle { item_id } => {
                    let (state, ui_models) = store.split_mut();
                    ui_models
                        .line_editor_state
                        .select_toggle(&state.project, item_id);
                }
                UIUpdateCommand::UpdateLineEditorItemsInserted {
                    item_id,
                    new_item_id,
                } => {
                    let (state, ui_models) = store.split_mut();
                    if let Err(err) = ui_models.line_editor_state.handle_item_inserted(
                        &state.project,
                        item_id,
                        new_item_id,
                    ) {
                        log::error!("执行 UpdateLineEditorItemsInserted 失败: {}", err);
                    }
                }
                UIUpdateCommand::UpdateLineEditorItemsRemoved { item_ids } => {
                    let (state, ui_models) = store.split_mut();
                    ui_models
                        .line_editor_state
                        .handle_items_removed(&state.project, &item_ids);
                }
            }
        }
    }

    fn update_area_geometry(
        store: &mut AppStore,
        // state 当中可能还有引用
        area_key: &ScrAreaKey,
    ) {
        let new_data = {
            let state = store.state();
            converters::convert_area(state, *area_key)
        };
        if let Some(data) = new_data {
            store
                .ui_models_mut()
                .update_scrarea_geometry(&area_key, data);
        }
    }

    fn update_edge_all(store: &mut AppStore, edge_key: ScrEdgeKey) {
        Self::update_edges_all(store, &[edge_key]);
    }

    // 重新计算并更新局部的 Edge（利用当前模型中的 edge_keys 合并信息）
    fn update_edges_all(store: &mut AppStore, edge_keys: &[ScrEdgeKey]) {
        let ui_models = store.ui_models_mut();
        let (display_model, touch_model) = (
            ui_models.scredge_displays(),
            ui_models.scredge_touch_areas(),
        );

        // 通过 edge_key -> index 找到受影响的行，并用行内 edge_keys 重算
        let recomputed = converters::recompute_edges_by_indices(store, edge_keys);
        for (idx, new_display, new_touch) in recomputed {
            if let Some(mut row) = display_model.row_data(idx) {
                row.x = new_display.x;
                row.y = new_display.y;
                row.width = new_display.width;
                row.height = new_display.height;
                row.active = new_display.active;
                row.edge_keys = new_display.edge_keys.clone();
                display_model.set_row_data(idx, row);
            }
            if let Some(mut row) = touch_model.row_data(idx) {
                row.x = new_touch.x;
                row.y = new_touch.y;
                row.width = new_touch.width;
                row.height = new_touch.height;
                row.vert_keys = new_touch.vert_keys.clone();
                row.edge_keys = new_touch.edge_keys.clone();
                touch_model.set_row_data(idx, row);
            }
        }
    }
    fn update_edges_only_display(store: &mut AppStore, edge_keys: &[ScrEdgeKey]) {
        let ui_models = store.ui_models_mut();
        let display_model = ui_models.scredge_displays();

        // 通过 edge_key -> index 找到受影响的行，并用行内 edge_keys 重算
        let recomputed = converters::recompute_edges_by_indices(store, edge_keys);
        for (idx, new_display, _new_touch) in recomputed {
            if let Some(mut row) = display_model.row_data(idx) {
                row.x = new_display.x;
                row.y = new_display.y;
                row.width = new_display.width;
                row.height = new_display.height;
                row.active = new_display.active;
                row.edge_keys = new_display.edge_keys.clone();
                display_model.set_row_data(idx, row);
            }
        }
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

    fn update_all(_instance: &MainWindow, store: &mut AppStore) {
        let areas = {
            let state = store.state();
            converters::convert_areas(state)
        };
        store.ui_models_mut().set_scrareas(areas);

        let (edges_display, edges_touch) = {
            let state = store.state();
            converters::convert_all_edges_both(state)
        };
        {
            let ui_models = store.ui_models_mut();
            ui_models.set_scredge_displays(edges_display);
            ui_models.set_scredge_touch_areas(edges_touch);
        }
    }
}
