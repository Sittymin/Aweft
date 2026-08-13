//! Screen 投影操作：把屏幕布局/拖拽状态的变化投影到 Area/Edge UI 模型与覆盖层。

use crate::depsgraph::Tag;
use crate::store::AppStore;
use slint::Model;

/// 覆盖层操作：任何 DragState 变化都重新计算并写出 OverlayState。
pub fn op_screen_overlays(store: &mut AppStore, _tags: &[Tag]) {
    write_overlays(store);
}

/// 布局操作：处理 ScreenLayout（areas/edges）变化。
///
/// 更新策略分档（互斥，取优先级最高者）：
/// - `ScreenInitialized`：从空布局创建，唯一需要全量 `set_vec` 的场景。
/// - `ScreenResized`/`ScaleChanged`：仅几何坐标变（area/edge 数量与类型不变），
///   逐个 `set_row_data` 局部更新（零组件销毁重建）。
/// - `AreasSplit`：原 area 保留（组件复用），新 area 精确 `row_added` 插入。
/// - `AreasJoined`：被吞并 area 精确 `row_removed` 移除，存活 area 几何更新。
/// - `AreaTypeChanged`：单 area 类型局部更新。
/// - `EdgeUpdated`：拖拽中仅更新受影响 area 几何 + edge Display 局部重算。
/// - `DragEnded`：split/join 同轮已由 AreasSplit/AreasJoined 精确处理；
///   仅纯边移动结束且 areas_changed 时全量（边结构可能变化）。
/// - areas 精确增删后 edges 全量重算（edge 合并行是几何驱动，diff 复杂）。
///
/// 同轮多 tag 的正确性：reducer 先改 state（screen_map），投影的 convert_area
/// 读**最新 state**——因此 split/join 的几何更新已包含同轮其他 reducer 侧变化
/// （resize 缩放顶点、类型切换等），互斥分档不会丢失它们。未来协作（远端
/// UI 更新 tag 混入，如改非本次操作 area）可能需要改为"多 tag 叠加执行"，
/// 届时按 Blender 式（tag 累积、组件求值一次看到全部）重构。
pub fn op_screen_layout(store: &mut AppStore, tags: &[Tag]) {
    let mut edge_keys: Vec<crate::screen::edge::ScrEdgeKey> = Vec::new();
    let mut area_keys: Vec<crate::screen::area::ScrAreaKey> = Vec::new();
    let mut area_type_changed: Option<(
        crate::screen::area::ScrAreaKey,
        crate::bridge::types::SlintAreaType,
    )> = None;
    let mut full_rebuild = false;
    let mut geometry_update = false;
    let mut split: Option<(
        crate::screen::area::ScrAreaKey,
        crate::screen::area::ScrAreaKey,
    )> = None;
    let mut joined: Option<(
        Vec<crate::screen::area::ScrAreaKey>,
        Vec<crate::screen::area::ScrAreaKey>,
        crate::screen::area::ScrAreaKey,
    )> = None;

    for tag in tags {
        match tag {
            Tag::ScreenInitialized => full_rebuild = true,

            // 仅几何坐标变化（resize/scale），area/edge 数量不变 → 局部更新
            Tag::ScreenResized { .. } | Tag::ScaleChanged { .. } => geometry_update = true,

            // 分割：原 area 保留（改几何），新 area 插入其后 → 精确增量
            Tag::AreasSplit {
                parent_key,
                new_area_key,
            } => split = Some((*parent_key, *new_area_key)),

            // 合并：存活 area 几何更新，被吞并/递归删除的 area 移除，
            // 对齐裁剪产生的新 area 插入 → 精确增量（存量组件复用）
            Tag::AreasJoined {
                removed_keys,
                added_keys,
                merged_into,
            } => joined = Some((removed_keys.clone(), added_keys.clone(), *merged_into)),

            Tag::AreaTypeChanged {
                area_key, new_type, ..
            } => {
                let slint_type = crate::bridge::types::rust_area_type_to_slint(new_type);
                area_type_changed = Some((*area_key, slint_type));
            }

            Tag::EdgeUpdated {
                edge_keys: ek,
                area_keys: ak,
            } => {
                edge_keys.extend(ek.iter().copied());
                area_keys.extend(ak.iter().copied());
            }

            Tag::DragEnded { areas_changed } => {
                // split/join 的 handle_actionzone_end 同轮产生
                // [AreasSplit/AreasJoined, DragEnded]，前者已精确处理
                // （apply_split/apply_join），DragEnded 不应再全量覆盖。
                // 仅当本轮没有精确 split/join（纯边移动结束）且
                // areas_changed 时才全量（边结构可能因 remove_duplicate_vertices 变化）。
                if *areas_changed && split.is_none() && joined.is_none() {
                    full_rebuild = true;
                }
            }

            _ => {}
        }
    }

    // 执行：互斥分档，取优先级最高者（Blender 式分档：每次只处理一类变化）。
    // 正确性依赖 reducer 先改 state、投影读最新 state——见函数注释。
    if let Some((key, slint_type)) = area_type_changed {
        store.ui_models_mut().update_area_type(&key, slint_type);
    }

    if full_rebuild {
        rebuild_all(store);
    } else if let Some((parent_key, new_area_key)) = split {
        apply_split(store, parent_key, new_area_key);
    } else if let Some((removed_keys, added_keys, merged_into)) = joined {
        apply_join(store, &removed_keys, &added_keys, merged_into);
    } else if geometry_update {
        update_geometry_locally(store);
    } else if !edge_keys.is_empty() {
        // 拖拽中：只更新受影响的 area 几何 + edge 局部重算（仅 Display，
        // 不更新 Touch——避免 TouchArea 重建导致焦点丢失/额外 mouse 事件）。
        for key in &area_keys {
            let new_data = {
                let state = store.state();
                crate::bridge::converters::convert_area(state, *key)
            };
            if let Some(data) = new_data {
                store.ui_models_mut().update_scrarea_geometry(key, data);
            }
        }

        let ui_models = store.ui_models_mut();
        let display_model = ui_models.scredge_displays();
        let recomputed = crate::bridge::converters::recompute_edges_by_indices(store, &edge_keys);
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
}

/// 分割：原 area 保留（组件复用，仅改几何），新 area 插入其后（`row_added`）。
/// edges 合并行是几何驱动，全量重算（边结构已变，diff 复杂度高）。
fn apply_split(
    store: &mut AppStore,
    parent_key: crate::screen::area::ScrAreaKey,
    new_area_key: crate::screen::area::ScrAreaKey,
) {
    // 原 area 几何更新，得到其索引（新 area 插在其后）
    let parent_data = {
        let state = store.state();
        crate::bridge::converters::convert_area(state, parent_key).expect("split 源 area 应存在")
    };
    let parent_idx = store
        .ui_models_mut()
        .update_scrarea(&parent_key, parent_data)
        .expect("split 源 area 应在 index map 中");

    // 新 area 插入在原 area 之后（row_added，存量组件复用）
    let new_data = {
        let state = store.state();
        crate::bridge::converters::convert_area(state, new_area_key).expect("split 新 area 应存在")
    };
    store
        .ui_models_mut()
        .insert_scrarea_at(parent_idx + 1, new_data);

    // edges 全量重算（边结构变化）
    rebuild_edges(store);
}

/// 合并：存活 area 几何 update + 被移除 area 精确 `row_removed` +
/// 新增 area 精确 `row_added`。edges 全量重算。
///
/// **不用全量重建**：join 内部 `split_area_by_coordinates`（对齐裁剪）与
/// `recursive_fill_close_areas`（递归填充）会动态新增/删除 area，但受影响集合由
/// reducer 通过 `JoinAffected`（before/after key diff）精确告知，投影据此精确增删。
/// 存量 Area 组件连同内部面板完全复用——只有真正增删的受影响。
///
/// 顺序不变式：slotmap 迭代顺序 = 插入顺序，join 新增 area 排在末尾、删除的从
/// 中间移除不改变存活项相对顺序。因此"倒序移除删除项 + 追加新增项"后，模型
/// 顺序与 `convert_areas` 一致，无需重排。
fn apply_join(
    store: &mut AppStore,
    removed_keys: &[crate::screen::area::ScrAreaKey],
    added_keys: &[crate::screen::area::ScrAreaKey],
    merged_into: crate::screen::area::ScrAreaKey,
) {
    // 1. 存活 area 几何更新（merge 后几何已重算）
    let alive: Vec<crate::screen::area::ScrAreaKey> = store
        .state()
        .screen_map
        .areas
        .iter()
        .map(|(k, _)| k)
        .collect();
    for key in alive {
        if let Some(data) = crate::bridge::converters::convert_area(store.state(), key) {
            store.ui_models_mut().update_scrarea(&key, data);
        }
    }

    // 2. 移除被删除的 area（倒序，避免 index 平移冲突）
    let mut removed_indices: Vec<usize> = Vec::new();
    {
        let ui = store.ui_models();
        for key in removed_keys {
            if let Some(idx) = ui.scrarea_index_of(key) {
                removed_indices.push(idx);
            }
        }
    }
    removed_indices.sort_unstable();
    removed_indices.dedup();
    for idx in removed_indices.into_iter().rev() {
        store.ui_models_mut().remove_scrarea_at(idx);
    }

    // 3. 追加新增的 area（slotmap 插入序 = 末尾，顺序与 convert_areas 一致）
    for key in added_keys {
        let data = crate::bridge::converters::convert_area(store.state(), *key);
        if let Some(data) = data {
            let at = store.ui_models().scrarea_count();
            store.ui_models_mut().insert_scrarea_at(at, data);
        }
    }

    let _ = merged_into;
    // edges 全量重算（边结构变化）
    rebuild_edges(store);
}

/// 全量重建 areas/edges 模型（`ScreenInitialized`/`AreasSplit`/`AreasJoined`）。
fn rebuild_all(store: &mut AppStore) {
    let areas = {
        let state = store.state();
        crate::bridge::converters::convert_areas(state)
    };
    store.ui_models_mut().set_scrareas(areas);

    rebuild_edges(store);
}

/// 全量重建 edges 模型（边结构变化时调用）。
fn rebuild_edges(store: &mut AppStore) {
    let (edges_display, edges_touch) = {
        let state = store.state();
        crate::bridge::converters::convert_all_edges_both(state)
    };
    {
        let ui_models = store.ui_models_mut();
        ui_models.set_scredge_displays(edges_display);
        ui_models.set_scredge_touch_areas(edges_touch);
    }
}

/// 局部几何更新（resize/缩放）：area/edge 数量不变，仅坐标变。
/// 逐个 `set_row_data`，零组件销毁重建。
fn update_geometry_locally(store: &mut AppStore) {
    // areas：遍历所有 area，逐个重算几何
    let area_keys: Vec<_> = store.state().screen_map.areas.keys().collect();
    for key in area_keys {
        if let Some(data) = crate::bridge::converters::convert_area(store.state(), key) {
            store.ui_models_mut().update_scrarea(&key, data);
        }
    }

    // edges：用 index map 反查所有行，逐行重算（不依赖 compute_edges_data 的
    // HashMap 行序，避免 resize 后行序漂移导致 index 错位）。
    let all_edge_keys: Vec<_> = store.state().screen_map.edges.keys().collect();
    let ui = store.ui_models_mut();
    let display_model = ui.scredge_displays();
    let recomputed = crate::bridge::converters::recompute_edges_by_indices(store, &all_edge_keys);
    for (idx, new_display, new_touch) in recomputed {
        if let Some(mut row) = display_model.row_data(idx) {
            row.x = new_display.x;
            row.y = new_display.y;
            row.width = new_display.width;
            row.height = new_display.height;
            row.active = new_display.active;
            row.edge_keys = new_display.edge_keys.clone();
            store.ui_models_mut().set_scredge_display_at(idx, row);
        }
        store.ui_models_mut().set_scredge_touch_at(idx, new_touch);
    }
}

/// 把当前拖拽状态写成 OverlayState 覆盖层。
fn write_overlays(store: &mut AppStore) {
    let Some(weak) = store.overlay_state.as_ref() else {
        return;
    };
    let Some(overlay) = weak.upgrade() else {
        return;
    };
    let state = store.state();
    let scale_factor = state.theme_settings.get_scale_factor();

    match &state.current_drag_operation {
        crate::store::state::DragOperation::Split { preview_data, .. } => {
            overlay.set_show_split_preview_overlay(true);
            overlay.set_split_preview_overlay_data(crate::bridge::types::scale_split_overlay(
                preview_data.clone(),
                scale_factor,
            ));
            overlay.set_show_join_highlight_overlay(false);
            overlay.set_show_close_area_overlays(false);
        }
        crate::store::state::DragOperation::Join {
            source_area_key,
            adjacency,
            ..
        } => {
            if let Ok((join_data, close_datas)) = state.screen_map.preview_join_adjacent_areas(
                *source_area_key,
                *adjacency,
                &state.theme_settings,
            ) {
                overlay.set_show_split_preview_overlay(false);
                overlay.set_show_join_highlight_overlay(true);
                overlay.set_join_highlight_overlay_data(crate::bridge::types::scale_join_overlay(
                    join_data,
                    scale_factor,
                ));
                overlay.set_close_area_overlay_datas(
                    crate::bridge::types::scale_close_overlays(close_datas, scale_factor)
                        .as_slice()
                        .into(),
                );
                overlay.set_show_close_area_overlays(true);
            } else {
                overlay.set_show_join_highlight_overlay(false);
                overlay.set_show_close_area_overlays(false);
            }
        }
        crate::store::state::DragOperation::None
        | crate::store::state::DragOperation::MoveEdge { .. } => {
            overlay.set_show_split_preview_overlay(false);
            overlay.set_show_join_highlight_overlay(false);
            overlay.set_show_close_area_overlays(false);
        }
    }
}
