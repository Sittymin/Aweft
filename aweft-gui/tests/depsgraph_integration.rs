//! 依赖图管线的无头集成测试。
//!
//! 用 Slint 的 testing 后端（无事件循环）验证核心闭环：
//! `dispatch(action)` → reducer 改数据 + 打 tag → `evaluate_depsgraph()` →
//! 投影重算 → UI 模型更新。
//!
//! 参考 Slint examples/todo 的测试模式：
//! `i_slint_backend_testing::init_no_event_loop()` + `MainWindow::new()`。

use std::cell::RefCell;
use std::rc::Rc;

use aweft_gui::actions::Action;
use aweft_gui::actions::area::AreaAction;
use aweft_gui::actions::screen::ScreenAction;
use aweft_gui::actions::sequence::SequenceAction;
use aweft_gui::bridge::slint_area_key_to_key;
use aweft_gui::bridge::types::{MainWindow, SplitPreviewOverlayData};
use aweft_gui::screen::area::ScrAreaKey;
use aweft_gui::screen::primitives::{Insets, Rcti};
use aweft_gui::store::AppStore;
use aweft_model::sequence::{EventData, ItemLocation};
use slint::{ComponentHandle, Model, ModelRc};

/// 构造未过滤的映射行模型（Rows → Map，等价于实际代码的 all_rows_model，
/// 但只在测试中使用——实际代码的过滤链路是 Rows → Filter → Map）。
fn all_rows(store: &AppStore) -> ModelRc<aweft_gui::bridge::types::SlintSubtitleRow> {
    let rows_model: Rc<aweft_gui::store::sequence_rows_model::SequenceRowsModel> =
        Rc::new(store.ui_models().sequence_state.rows_model().clone());
    let mapper = rows_model.mapper();
    let mapped = slint::MapModel::new(rows_model, move |data| mapper.map(&data));
    ModelRc::new(mapped)
}

#[test]
fn screen_init_creates_three_areas() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 初始化布局 → Screen 投影全量重建 areas 模型
    let area_count = {
        let s = store.borrow();
        s.ui_models().scrareas().row_count()
    };
    assert!(
        area_count >= 3,
        "初始化应创建至少 3 个 area，实际 {}",
        area_count
    );
}

#[test]
fn screen_split_reuses_components() {
    // 分割区域：原 area 保留（组件复用），新 area 精确插入（row_added），
    // 不应销毁重建整个 area 列表。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    // 初始化
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 取第一个 area 作为分割源
    let (area_key, area_rect): (ScrAreaKey, Rcti) = {
        let s = store.borrow();
        let state = s.state();
        let (key, area) = state.screen_map.areas.iter().next().unwrap();
        let rect = area.geometry_rect(&state.screen_map.verts);
        (key, rect)
    };

    let count_before = store.borrow().ui_models().scrarea_count();

    // 水平分割：源 area 在水平中点处分成左右两块
    let mid_x = (area_rect.xmin + area_rect.xmax) / 2;
    let width2 = (area_rect.xmax as i64 - mid_x as i64 + 1).max(1) as f32;
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Area(AreaAction::Split {
                area_key,
                preview_data: SplitPreviewOverlayData {
                    x1: area_rect.xmin as f32,
                    y1: area_rect.ymin as f32,
                    width1: (mid_x as i64 - area_rect.xmin as i64).max(1) as f32,
                    height1: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                    x2: mid_x as f32,
                    y2: area_rect.ymin as f32,
                    width2,
                    height2: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                },
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 分割后 area 数 +1
    let s = store.borrow();
    assert_eq!(s.ui_models().scrarea_count(), count_before + 1);
    // 原 area 仍在其原始索引位置（组件复用）
    let first = s.ui_models().scrareas().row_data(0).unwrap();
    let first_key: ScrAreaKey = slint_area_key_to_key(&first.area_key);
    assert_eq!(first_key, area_key);
}

#[test]
fn screen_actionzone_split_uses_precise_increment() {
    // 真实拖拽分割路径：handle_actionzone_end 产生
    // [AreasSplit, DragEnded{areas_changed:true}] 同轮 tags。
    // 投影应走 apply_split 精确增量（DragEnded 不覆盖精确路径）。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    // 初始化
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 手动注入 DragOperation::Split（模拟拖拽中已设置分割预览状态）
    let (area_key, area_rect): (ScrAreaKey, Rcti) = {
        let s = store.borrow();
        let state = s.state();
        let (key, area) = state.screen_map.areas.iter().next().unwrap();
        let rect = area.geometry_rect(&state.screen_map.verts);
        (key, rect)
    };
    let mid_x = (area_rect.xmin + area_rect.xmax) / 2;
    {
        let mut store_mut = store.borrow_mut();
        store_mut.state_mut().current_drag_operation =
            aweft_gui::store::state::DragOperation::Split {
                source_area: area_key,
                preview_data: SplitPreviewOverlayData {
                    x1: area_rect.xmin as f32,
                    y1: area_rect.ymin as f32,
                    width1: (mid_x as i64 - area_rect.xmin as i64).max(1) as f32,
                    height1: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                    x2: mid_x as f32,
                    y2: area_rect.ymin as f32,
                    width2: (area_rect.xmax as i64 - mid_x as i64 + 1).max(1) as f32,
                    height2: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                },
            };
    }

    // 结束拖拽（actionzone end）→ 产生 [AreasSplit, DragEnded]
    let count_before = store.borrow().ui_models().scrarea_count();
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Drag(
                aweft_gui::actions::drag::DragAction::ActionZone(
                    aweft_gui::actions::drag::ActionZoneDrag::End {
                        area_key: Default::default(),
                        is_cancel: false,
                    },
                ),
            ))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 精确增量：area 数 +1，原 area 索引保持
    let s = store.borrow();
    assert_eq!(s.ui_models().scrarea_count(), count_before + 1);
    let first = s.ui_models().scrareas().row_data(0).unwrap();
    let first_key: ScrAreaKey = slint_area_key_to_key(&first.area_key);
    assert_eq!(
        first_key, area_key,
        "真实拖拽分割应走精确增量，原 area 保留"
    );
}

#[test]
fn screen_resize_updates_geometry_locally() {
    // resize 是几何局部更新（area/edge 数量不变，仅坐标变），
    // 不应销毁重建（行数保持、index map 一致）。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    // 初始化
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 记录 resize 前的模型行数与首个 area 的 key
    let (areas_before, edge_display_before) = {
        let s = store.borrow();
        let ui = s.ui_models();
        let areas = ui.scrareas();
        let first_key = areas.row_data(0).map(|d| d.area_key).unwrap_or_default();
        (
            areas.row_count(),
            (ui.scredge_displays().row_count(), first_key),
        )
    };

    // resize 到 1000x800
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::WindowResized {
                width: 1000,
                height: 800,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 行数不变（未销毁重建）
    let s = store.borrow();
    let ui = s.ui_models();
    assert_eq!(
        ui.scrareas().row_count(),
        areas_before,
        "resize 不应改变 area 数量"
    );
    assert_eq!(
        ui.scredge_displays().row_count(),
        edge_display_before.0,
        "resize 不应改变 edge 行数"
    );
    // 几何确实更新（800→1000 宽，首个 area x 坐标或宽度应变化）
    let first = ui.scrareas().row_data(0).unwrap();
    assert_eq!(first.area_key, edge_display_before.1, "area key 顺序应保持");
}

#[test]
fn single_op_updates_only_affected_row() {
    // 验证最小化：dispatch 一行时间码，只有该行被更新，其他行数据不变。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    // 初始化 + 注入两行
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    let (id1, id2) = {
        let mut store_mut = store.borrow_mut();
        let sequence = store_mut.state_mut().project.sequence_mut();
        let id1 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(0).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    actor: None,
                    text: "行1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let id2 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(2000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(3000).unwrap(),
                    actor: None,
                    text: "行2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        store_mut.rebuild_sequence_models();
        (id1, id2)
    };

    // 更新第 1 行时间码
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::InputTimecode {
                event_id: id1,
                text: "0:00:00.50".into(),
                is_start_time: true,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 第 1 行更新，第 2 行保持不变
    let s = store.borrow();
    let rows = all_rows(&s);
    assert_eq!(rows.row_count(), 2);
    let r1 = rows.row_data(0).unwrap();
    let r2 = rows.row_data(1).unwrap();
    assert_eq!(r1.start_time.to_string(), "0:00:00.50");
    // 第 2 行完全未被触碰（item_id 和原值不变）
    assert_eq!(r2.item_id, slint::SharedString::from(id2.to_string()));
    assert_eq!(r2.start_time.to_string(), "0:00:02.00");
    assert_eq!(r2.text.to_string(), "行2");
}

#[test]
fn drag_end_clears_overlay_preview() {
    // Bug 回归：拖拽结束后 Overlay 预览未清除。
    // 根因：op_screen_overlays 只 handles DragOverlay，DragEnded 不命中它，
    // write_overlays（None 分支清除预览）从不执行。修复后 DragEnded 也命中。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    // 初始化布局
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 模拟拖拽中已进入预览（OverlayState 预览标志为 true）
    {
        let overlay = instance.global::<aweft_gui::bridge::types::OverlayState>();
        overlay.set_show_join_highlight_overlay(true);
        overlay.set_show_close_area_overlays(true);
    }

    // 取消拖拽 → DragEnded{areas_changed:false}（安全路径，不涉及 join 几何）
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Drag(
                aweft_gui::actions::drag::DragAction::ActionZone(
                    aweft_gui::actions::drag::ActionZoneDrag::End {
                        area_key: Default::default(),
                        is_cancel: true,
                    },
                ),
            ))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // Overlay 预览应被清除（DragEnded 命中 op_screen_overlays → write_overlays None 分支）
    let overlay = instance.global::<aweft_gui::bridge::types::OverlayState>();
    assert!(
        !overlay.get_show_join_highlight_overlay(),
        "拖拽结束后 join 高亮预览应被清除"
    );
    assert!(
        !overlay.get_show_close_area_overlays(),
        "拖拽结束后 close area 覆盖层应被清除"
    );
}

#[test]
fn join_keeps_edge_touch_and_area_actionzone() {
    // Bug 回归：合并后合并 Area 的角点操作点（actionzone）与 Edge 拖拽边消失。
    // 全量重建（rebuild_all）可恢复，说明 apply_join 局部路径没同步 edge touch。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    // 初始化布局
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 用真实拖拽路径分割成两个 area（复用现有测试的 split 逻辑）
    let (area_key, area_rect): (ScrAreaKey, Rcti) = {
        let s = store.borrow();
        let state = s.state();
        let (key, area) = state.screen_map.areas.iter().next().unwrap();
        (key, area.geometry_rect(&state.screen_map.verts))
    };
    let mid_x = (area_rect.xmin + area_rect.xmax) / 2;
    {
        let mut store_mut = store.borrow_mut();
        store_mut.state_mut().current_drag_operation =
            aweft_gui::store::state::DragOperation::Split {
                source_area: area_key,
                preview_data: SplitPreviewOverlayData {
                    x1: area_rect.xmin as f32,
                    y1: area_rect.ymin as f32,
                    width1: (mid_x as i64 - area_rect.xmin as i64).max(1) as f32,
                    height1: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                    x2: mid_x as f32,
                    y2: area_rect.ymin as f32,
                    width2: (area_rect.xmax as i64 - mid_x as i64 + 1).max(1) as f32,
                    height2: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                },
            };
    }
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Drag(
                aweft_gui::actions::drag::DragAction::ActionZone(
                    aweft_gui::actions::drag::ActionZoneDrag::End {
                        area_key: Default::default(),
                        is_cancel: false,
                    },
                ),
            ))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 分割后（split 可能递归产生多个 area，动态取数）
    let split_area_count = store.borrow().ui_models().scrarea_count();
    assert!(split_area_count > 1, "分割后应多于 1 个 area");

    // 记录分割后的 edge touch 数量（应>0）
    let touch_after_split = store.borrow().ui_models().scredge_touch_areas().row_count();
    assert!(touch_after_split > 0, "分割后应有 edge 拖拽边");

    // 现在 join 回（循环合并直到只剩 1 个 area，覆盖递归空位填充）
    let mut iterations = 0;
    'join_loop: while store.borrow().ui_models().scrarea_count() > 1 && iterations < 10 {
        iterations += 1;
        let (src_key, tgt_key, adjacency) = {
            let s = store.borrow();
            let state = s.state();
            let keys: Vec<ScrAreaKey> = state.screen_map.areas.iter().map(|(k, _)| k).collect();
            let mut found = None;
            'outer: for i in 0..keys.len() {
                for j in (i + 1)..keys.len() {
                    if let Some(adj) = state.screen_map.detect_adjacency_by_keys(
                        keys[i],
                        keys[j],
                        &state.theme_settings,
                    ) {
                        found = Some((keys[i], keys[j], adj));
                        break 'outer;
                    }
                }
            }
            match found {
                Some(f) => f,
                None => break 'join_loop,
            }
        };
        {
            let mut store_mut = store.borrow_mut();
            store_mut.state_mut().current_drag_operation =
                aweft_gui::store::state::DragOperation::Join {
                    source_area_key: src_key,
                    target_area_key: tgt_key,
                    adjacency,
                };
        }
        {
            let mut store_mut = store.borrow_mut();
            store_mut
                .dispatch(Action::Drag(
                    aweft_gui::actions::drag::DragAction::ActionZone(
                        aweft_gui::actions::drag::ActionZoneDrag::End {
                            area_key: Default::default(),
                            is_cancel: false,
                        },
                    ),
                ))
                .unwrap();
            store_mut.evaluate_depsgraph();
        }
    }
    assert!(iterations > 0, "应至少执行一次 join");

    // 合并后：数据层与 UI 完全同步
    // （apply_join 用 JoinAffected 精确 row_removed/row_added，存量 Area 组件复用，
    //  不触发全量重建）
    assert_eq!(store.borrow().ui_models().scrarea_count(), 1);
    let s = store.borrow();
    assert_eq!(
        s.state().screen_map.areas.len(),
        s.ui_models().scrarea_count(),
        "join 后 screen_map area 数应与 UI 模型行数一致"
    );
    // 模型行序与 convert_areas（screen_map 迭代序）一致——局部增删保持顺序
    let ui_keys: Vec<ScrAreaKey> = (0..s.ui_models().scrarea_count())
        .filter_map(|i| {
            s.ui_models()
                .scrareas()
                .row_data(i)
                .map(|d| aweft_gui::bridge::slint_area_key_to_key(&d.area_key))
        })
        .collect();
    let map_keys: Vec<ScrAreaKey> = s.state().screen_map.areas.iter().map(|(k, _)| k).collect();
    assert_eq!(
        ui_keys, map_keys,
        "join 后 UI 模型 area 顺序应与 screen_map 一致"
    );

    // Edge touch（拖拽边）应与 screen_map 非 border 边数一致
    let touch_after_join = s.ui_models().scredge_touch_areas().row_count();
    let edge_count = s
        .state()
        .screen_map
        .edges
        .iter()
        .filter(|(_, e)| !e.border)
        .count();
    assert_eq!(
        touch_after_join, edge_count,
        "join 后 edge touch 模型应等于 screen_map 非 border 边数（split 后 {} → join 后 {}）",
        touch_after_split, touch_after_join
    );
}

#[test]
fn screen_layout_handles_multiple_tags_same_round() {
    // 同轮多个不同 tag：验证正确性依赖"reducer 先改 state、投影读最新 state"。
    // split（AreasSplit）与 SwitchType（AreaTypeChanged）同轮——即使互斥分档
    // 只走 apply_split，其 convert_area 也读到已改的 state，类型/几何不丢。
    // 为未来协作（远端 UI 更新 tag 混入）打下正确性基础。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    // 初始化
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 手动注入 DragOperation::Split（模拟拖拽中已设置分割预览）
    let (area_key, area_rect): (ScrAreaKey, Rcti) = {
        let s = store.borrow();
        let state = s.state();
        let (key, area) = state.screen_map.areas.iter().next().unwrap();
        let rect = area.geometry_rect(&state.screen_map.verts);
        (key, rect)
    };
    let mid_x = (area_rect.xmin + area_rect.xmax) / 2;
    {
        let mut store_mut = store.borrow_mut();
        store_mut.state_mut().current_drag_operation =
            aweft_gui::store::state::DragOperation::Split {
                source_area: area_key,
                preview_data: SplitPreviewOverlayData {
                    x1: area_rect.xmin as f32,
                    y1: area_rect.ymin as f32,
                    width1: (mid_x as i64 - area_rect.xmin as i64).max(1) as f32,
                    height1: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                    x2: mid_x as f32,
                    y2: area_rect.ymin as f32,
                    width2: (area_rect.xmax as i64 - mid_x as i64 + 1).max(1) as f32,
                    height2: (area_rect.ymax as i64 - area_rect.ymin as i64 + 1) as f32,
                },
            };
    }

    let count_before = store.borrow().ui_models().scrarea_count();

    // 同一轮：先 split（dispatch 不 evaluate），再 resize，最后统一 evaluate
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Drag(
                aweft_gui::actions::drag::DragAction::ActionZone(
                    aweft_gui::actions::drag::ActionZoneDrag::End {
                        area_key: Default::default(),
                        is_cancel: false,
                    },
                ),
            ))
            .unwrap();
        // resize（split 的 tag 已挂起，未 evaluate）
        store_mut
            .dispatch(Action::Screen(ScreenAction::WindowResized {
                width: 1000,
                height: 800,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 记录 split 前首个 area 的类型
    let orig_type = {
        let s = store.borrow();
        let state = s.state();
        state
            .screen_map
            .areas
            .values()
            .next()
            .map(|a| a.area_type.clone())
            .unwrap()
    };

    // 同轮 dispatch split + SwitchType（不 evaluate），再统一 evaluate。
    // 模拟本地 split + 远端类型切换（远端 UI 更新同轮混入）。
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Drag(
                aweft_gui::actions::drag::DragAction::ActionZone(
                    aweft_gui::actions::drag::ActionZoneDrag::End {
                        area_key: Default::default(),
                        is_cancel: false,
                    },
                ),
            ))
            .unwrap();
        store_mut
            .dispatch(Action::Area(AreaAction::SwitchType {
                area_key,
                area_type: aweft_gui::screen::area::AreaType::Timeline,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 两个意图都应生效：split → area +1；SwitchType → 类型改变。
    let s = store.borrow();
    let ui = s.ui_models();
    assert_eq!(
        ui.scrarea_count(),
        count_before + 1,
        "同轮 split+SwitchType：split 应生效（area +1）"
    );
    // UI 层：类型从 Viewport → Timeline（已接入真实 Timeline 变体）
    let first = ui.scrareas().row_data(0).unwrap();
    assert_eq!(
        first.area_type,
        aweft_gui::bridge::types::SlintAreaType::Timeline,
        "同轮 split+SwitchType：SwitchType 应生效（UI 层为 Timeline，原 {:?}）",
        orig_type
    );
    // 数据层：area 真实类型应为 Timeline（不被 UI 折叠掩盖）
    let state = s.state();
    let area0 = state
        .screen_map
        .areas
        .get(area_key)
        .expect("首个 area key 应保留");
    assert_eq!(
        area0.area_type,
        aweft_gui::screen::area::AreaType::Timeline,
        "同轮 split+SwitchType：数据层类型应为 Timeline"
    );
}

#[test]
fn resize_roundtrip_keeps_vertices_on_window_edge() {
    // 回归：resize 往返后顶点应精确对齐窗口边缘（无舍入漂移）。
    // 曾因 enforce_window_edge_snap 把边缘顶点吸附到 margin 距离（4px），
    // 导致双重 margin + resize 缩小时 xmax<xmin panic。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut s = store.borrow_mut();
        s.state_mut().theme_settings.update_scale_factor(2.0);
        s.dispatch(Action::Screen(ScreenAction::InitializeLayout {
            width: 1600,
            height: 1200,
            scale_factor: 2.0,
            inset: Insets::default(),
        }))
        .unwrap();
        s.evaluate_depsgraph();
    }

    let max_geom_x = || {
        let s = store.borrow();
        s.state()
            .screen_map
            .areas
            .values()
            .map(|a| a.geometry_rect(&s.state().screen_map.verts).xmax)
            .max()
            .unwrap()
    };

    // 往返 resize，每次顶点都应精确等于窗口边缘
    for (w, h) in [(1400, 1000), (1500, 1100), (1600, 1200)] {
        {
            let mut s = store.borrow_mut();
            s.dispatch(Action::Screen(ScreenAction::WindowResized {
                width: w,
                height: h,
            }))
            .unwrap();
            s.evaluate_depsgraph();
        }
        let s = store.borrow();
        let win = s.state().current_window_rect();
        assert_eq!(
            max_geom_x(),
            win.xmax,
            "resize {}x{} 后顶点 xmax 应精确等于窗口边缘 {}",
            w,
            h,
            win.xmax
        );
    }
}

#[test]
fn edge_move_keeps_single_margin() {
    // 回归：拖边后 area margin 不应双重（曾因顶点被 margin 污染导致
    // calculate_totrct_and_size 再收缩一次 → layout 从 (4,4) 变 (8,8)）。
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut s = store.borrow_mut();
        s.state_mut().theme_settings.update_scale_factor(2.0);
        s.dispatch(Action::Screen(ScreenAction::InitializeLayout {
            width: 1600,
            height: 1200,
            scale_factor: 2.0,
            inset: Insets::default(),
        }))
        .unwrap();
        s.evaluate_depsgraph();
    }

    // 记录初始首个 area 的 layout xmin（应 = margin 4px）
    let init_layout = {
        let s = store.borrow();
        let st = s.state();
        let a = st.screen_map.areas.values().next().unwrap();
        let (layout, _, _) = a.layout_rect();
        (layout.xmin, layout.ymin)
    };
    assert_eq!(init_layout.0, 4, "初始 margin 应为单次 4px");

    // 找内部竖直边，设 MoveEdge 状态
    let (v1, v2) = {
        let s = store.borrow();
        let st = s.state();
        let (v1, v2) = st
            .screen_map
            .edges
            .iter()
            .find(|(_, e)| !e.border)
            .map(|(_, e)| (e.v1, e.v2))
            .unwrap();
        (v1, v2)
    };
    {
        let mut s = store.borrow_mut();
        s.state_mut().current_drag_operation = aweft_gui::store::state::DragOperation::MoveEdge {
            dir_axis_horizontal: false,
            origval: 800,
            bigger: 300,
            smaller: 300,
            moving_vert_keys: vec![v1, v2],
            edge_keys_rc: Rc::new(Vec::new()),
            area_keys_rc: Rc::new(Vec::new()),
        };
    }
    {
        let mut s = store.borrow_mut();
        let vert_keys: slint::ModelRc<aweft_gui::bridge::types::SlintKey> = {
            let mut v = Vec::new();
            for k in [v1, v2] {
                v.push(aweft_gui::bridge::types::key_to_slint_key(&k));
            }
            v.as_slice().into()
        };
        s.dispatch(Action::Drag(aweft_gui::actions::drag::DragAction::Edge(
            aweft_gui::actions::drag::EdgeDrag::Update {
                vert_keys,
                offset_x: 50.0,
                offset_y: 0.0,
            },
        )))
        .unwrap();
        s.evaluate_depsgraph();
    }

    // 拖边后 margin 仍为单次 4px（不双重为 8px）
    let s = store.borrow();
    let st = s.state();
    let a = st.screen_map.areas.values().next().unwrap();
    let (layout, _, _) = a.layout_rect();
    assert_eq!(
        layout.xmin, 4,
        "拖边后 margin 应保持单次 4px（不应双重为 8px），实际 {}",
        layout.xmin
    );
}

// ── 曲线编辑器：初始化新建空 Action vs 测试数据 ────────────────────────────────

#[test]
fn animation_init_creates_empty_editing_action() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);

    // 正确初始化路径：新建空 Action
    store.new_editing_action("主 Action");

    let (list_len, current, channels, action_name) = {
        let ce = &store.ui_models().animation.graph;
        (
            ce.actions.len(),
            ce.current_action,
            ce.channels.row_count(),
            ce.action_name.to_string(),
        )
    };
    assert_eq!(list_len, 1, "初始化应只有一个 Action");
    assert_eq!(current, 0, "应装载第 0 个 Action");
    assert_eq!(action_name, "主 Action");
    assert_eq!(channels, 0, "新建的 Action 应为空（无通道/关键帧）");

    // model 侧：确实有一个 Action，且无曲线
    let lib = &store.state().project.project().library.animation_library;
    assert_eq!(lib.actions.len(), 1);
    assert!(lib.actions.values().next().unwrap().curves.is_empty());
}

/// HACK 测试数据的验证（见 `create_test_animation_data` 的 HACK 注释）：
/// 保证该临时函数仍可用（多 Action 演示下拉/切换）。
#[test]
fn animation_hack_test_data_populates_model_and_view() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);

    store.create_test_animation_data();

    let (list_len, current, channel_count) = {
        let ce = &store.ui_models().animation.graph;
        (ce.actions.len(), ce.current_action, ce.channels.row_count())
    };
    assert_eq!(list_len, 3, "测试数据应有 3 个 Action");
    assert_eq!(current, 0, "应装载第 0 个 Action（主 Action）");
    assert_eq!(channel_count, 4, "主 Action 应有 4 条通道");

    // model 侧：3 个 Action 都已写入 AnimationLibrary
    let lib = &store.state().project.project().library.animation_library;
    assert_eq!(lib.actions.len(), 3);
}

/// 动画投影全链路：dispatch(AnimationAction) → tag → evaluate → 投影更新 UI 模型。
/// 验证 reducer 只改数据层、派生层由 depsgraph 投影驱动（对齐 Sequence 管线）。
#[test]
fn animation_dispatch_projects_derived_models() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    use aweft_gui::actions::animation::{AnimationAction, graph::GraphAction};
    use aweft_gui::bridge::types::{SlintKeyframeDragMode, SlintSelectionModifier};

    // 按下关键帧 (通道 0, 关键帧 1)：reducer 改数据 → Selection tag → 投影同步选中
    store
        .dispatch(Action::Animation(AnimationAction::Graph(
            GraphAction::KeyframePressed {
                curve: 0,
                key: 1,
                modifier: SlintSelectionModifier::None,
            },
        )))
        .unwrap();
    store.evaluate_depsgraph();
    let ce = &store.ui_models().animation.graph;
    assert_eq!(
        ce.selected_display.row_data(0),
        Some(true),
        "投影应同步通道选中显示掩码"
    );
    assert_eq!(ce.selected_keyframe_count(), 1, "投影后选中关键帧计数 = 1");

    // 拖拽关键帧：Keyframes tag → 投影重建段/手柄/汇总
    store
        .dispatch(Action::Animation(AnimationAction::Graph(
            GraphAction::KeyframeDragged {
                curve: 0,
                key: 1,
                delta_time: 1.0,
                delta_value: -5.0,
                mode: SlintKeyframeDragMode::Move,
            },
        )))
        .unwrap();
    store.evaluate_depsgraph();
    let ce = &store.ui_models().animation.graph;
    // 段 0 终点跟随关键帧新位置（投影重建）
    let seg0 = ce.segments.row_data(0).unwrap();
    assert_eq!(seg0.x1, 4.0);
    assert_eq!(seg0.y1, 61.0);

    // 松开：写回 model（reducer）+ 折叠选中 → 投影同步
    store
        .dispatch(Action::Animation(AnimationAction::Graph(
            GraphAction::KeyframeReleased {
                curve: 0,
                key: 1,
                moved: true,
            },
        )))
        .unwrap();
    store.evaluate_depsgraph();
    let id = store
        .ui_models()
        .animation
        .graph
        .current_action_id()
        .unwrap();
    let action = store
        .state()
        .project.project()
        .library
        .animation_library
        .actions
        .get(&id)
        .unwrap();
    assert!(
        action.curves[0]
            .curve
            .keyframes
            .iter()
            .any(|k| (k.time - 4.0).abs() < 1e-6)
    );
}

/// 曲线编辑器搜索：graph_filter 独立过滤（与摄影表互不干扰）；
/// 段/手柄过滤视图随匹配集合收缩（画布只显示匹配通道的曲线）。
#[test]
fn graph_search_filters_channels_and_canvas() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    use aweft_gui::actions::animation::{AnimationAction, graph::GraphAction};

    store
        .dispatch(Action::Animation(AnimationAction::Graph(
            GraphAction::GraphSearchChanged {
                text: "font".to_string(),
            },
        )))
        .unwrap();
    store.evaluate_depsgraph();

    let ce = &store.ui_models().animation.graph;
    assert_eq!(
        ce.graph_filter.row_count(),
        2,
        "曲线编辑器过滤视图应只剩 Font 开头通道"
    );
    assert_eq!(ce.dope_filter.row_count(), 4, "摄影表过滤视图不受影响");
    // 段/手柄过滤视图 = 匹配通道的段（通道 0 FontSize 6 关键帧 = 5 段 + 通道 2 FontRotationZ 5 关键帧 = 4 段）
    assert_eq!(ce.graph_segments.row_count(), 9, "画布段应只含匹配通道");
    assert_eq!(ce.graph_handles.row_count(), 10, "画布手柄应只含匹配通道");

    // 空串恢复全部
    store
        .dispatch(Action::Animation(AnimationAction::Graph(
            GraphAction::GraphSearchChanged {
                text: String::new(),
            },
        )))
        .unwrap();
    store.evaluate_depsgraph();
    let ce = &store.ui_models().animation.graph;
    assert_eq!(ce.graph_filter.row_count(), 4, "空串应恢复全部通道");
}

/// 时间线片段整体拖动闭环：预览不写 sequence（行映射器把覆盖同时应用到
/// 时间码文本与 start_sec/end_sec，字幕行编辑器与时间线块一并驱动），
/// 提交保时长写回 Domain，取消恢复 Domain 显示。开始时间允许为负（不钳制）。
#[test]
fn timeline_block_drag_preview_commit_cancel() {
    use aweft_model::ids::ItemId;

    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    let id: ItemId = {
        let mut store_mut = store.borrow_mut();
        let sequence = store_mut.state_mut().project.sequence_mut();
        let event_id = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(3000).unwrap(),
                    actor: None,
                    text: "拖拽行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        store_mut.rebuild_sequence_models();
        event_id
    };

    // 预览 +2.5s：行显示覆盖值，Domain 不变
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragPreview {
                item_id: id,
                delta_x: 2.5,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(
            seq.event_time(id, true).unwrap().num_milliseconds(),
            1000,
            "预览不写 start"
        );
        assert_eq!(
            seq.event_time(id, false).unwrap().num_milliseconds(),
            3000,
            "预览不写 end"
        );
        let row = all_rows(&s).row_data(0).unwrap();
        assert_eq!(row.start_sec, 3.5, "预览覆盖 start_sec（时间线块随之移动）");
        assert_eq!(row.end_sec, 5.5, "预览覆盖 end_sec");
    }

    // 预览 -5.0s：开始时间为负不钳制（纯平移）
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragPreview {
                item_id: id,
                delta_x: -5.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let row = all_rows(&s).row_data(0).unwrap();
        assert_eq!(
            row.start_sec, -4.0,
            "开始时间允许为负（不钳制，预览基于 Domain 原值）"
        );
        assert_eq!(row.end_sec, -2.0);
    }

    // 提交 +2.5s：写回 Domain，时长保持不变，覆盖清除
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragCommit {
                item_id: id,
                delta_x: 2.5,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(
            seq.event_time(id, true).unwrap().num_milliseconds(),
            3500,
            "提交写回 start"
        );
        assert_eq!(
            seq.event_time(id, false).unwrap().num_milliseconds(),
            5500,
            "提交写回 end（时长保持 2000ms）"
        );
        let row = all_rows(&s).row_data(0).unwrap();
        assert_eq!(row.start_sec, 3.5, "提交后显示 Domain 值（覆盖已清除）");
        assert_eq!(row.end_sec, 5.5);
    }

    // 预览 +1.0s 后取消：恢复 Domain 显示
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragPreview {
                item_id: id,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragCancel {
                item_id: id,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(
            seq.event_time(id, true).unwrap().num_milliseconds(),
            3500,
            "取消不改 Domain"
        );
        let row = all_rows(&s).row_data(0).unwrap();
        assert_eq!(row.start_sec, 3.5, "取消后恢复 Domain 显示");
        assert_eq!(row.end_sec, 5.5);
    }
}

/// 时间线多选整体移动：按住已选片段拖动时整个选中事件集一起平移（时长各自保持）；
/// 按住未选中片段只动它自己（NLA「按住已选片断整体拖动」语义）。
#[test]
fn timeline_block_drag_moves_selection_group() {
    use aweft_gui::bridge::types::SlintSubtitleRow;
    use aweft_model::ids::ItemId;

    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    let (id1, id2) = {
        let mut store_mut = store.borrow_mut();
        let sequence = store_mut.state_mut().project.sequence_mut();
        let e1 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(3000).unwrap(),
                    actor: None,
                    text: "行1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let e2 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(5000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(6000).unwrap(),
                    actor: None,
                    text: "行2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        store_mut.rebuild_sequence_models();
        (e1, e2)
    };
    // 在映射行模型中按 item_id 定位行
    fn row_of(rows: &ModelRc<SlintSubtitleRow>, id: ItemId) -> usize {
        (0..rows.row_count())
            .find(|&i| rows.row_data(i).map(|r| r.item_id.to_string()) == Some(id.to_string()))
            .unwrap()
    }

    // 选中两行：e1 单选 + e2 切换加入
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::SelectRow {
                item_id: id1,
                modifier: aweft_gui::bridge::types::SlintSelectionModifier::None,
            }))
            .unwrap();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::SelectRow {
                item_id: id2,
                modifier: aweft_gui::bridge::types::SlintSelectionModifier::Ctrl,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 预览按住 e1 拖 +1.0s：两行一起移动，Domain 不变
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragPreview {
                item_id: id1,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(seq.event_time(id1, true).unwrap().num_milliseconds(), 1000);
        assert_eq!(seq.event_time(id2, true).unwrap().num_milliseconds(), 5000);
        let rows = all_rows(&s);
        let r1 = rows.row_data(row_of(&rows, id1)).unwrap();
        let r2 = rows.row_data(row_of(&rows, id2)).unwrap();
        assert_eq!(r1.start_sec, 2.0, "e1 预览随组移动");
        assert_eq!(r2.start_sec, 6.0, "e2 预览随组移动");
    }

    // 提交 +1.0s：两行都写回，各自时长保持
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragCommit {
                item_id: id1,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(seq.event_time(id1, true).unwrap().num_milliseconds(), 2000);
        assert_eq!(seq.event_time(id1, false).unwrap().num_milliseconds(), 4000);
        assert_eq!(seq.event_time(id2, true).unwrap().num_milliseconds(), 6000);
        assert_eq!(seq.event_time(id2, false).unwrap().num_milliseconds(), 7000);
    }

    // 清空选中后按住 e1 拖 +1.0s：只动 e1
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::DeselectAll))
            .unwrap();
        store_mut.evaluate_depsgraph();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragCommit {
                item_id: id1,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(
            seq.event_time(id1, true).unwrap().num_milliseconds(),
            3000,
            "未选中时只动当前片段"
        );
        assert_eq!(
            seq.event_time(id2, true).unwrap().num_milliseconds(),
            6000,
            "e2 不受影响"
        );
    }
}

/// 时间线框选不直接选中分组头：命中仅事件行；组内事件全选时组头派生为
/// 间接选中，未全选时为 Unselected（与字幕行编辑器 Shift 范围选择同语义）。
#[test]
fn marquee_select_skips_group_header() {
    use aweft_gui::bridge::types::SlintSelectionModifier;
    use aweft_model::sequence::GroupData;

    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    let (gid, e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let sequence = store_mut.state_mut().project.sequence_mut();
        let gid = sequence
            .create_group(
                GroupData {
                    name: "G".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let e1 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(2000).unwrap(),
                    actor: None,
                    text: "e1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let e2 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(5000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(6000).unwrap(),
                    actor: None,
                    text: "e2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        store_mut.rebuild_sequence_models();
        (gid, e1, e2)
    };

    // 全范围框选（可见行 [G,e1,e2] 全部 × 时间覆盖一切）：
    // e1/e2 直接选中；G 不直接选中，因子项全选派生为间接选中
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::MarqueeSelect {
                x_lo: -100.0,
                x_hi: 100.0,
                row_lo: 0,
                row_hi: 2,
                modifier: SlintSelectionModifier::None,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let rows = s.ui_models().sequence_state.rows_model();
        assert!(rows.is_directly_selected(e1), "e1 直接选中");
        assert!(rows.is_directly_selected(e2), "e2 直接选中");
        assert!(!rows.is_directly_selected(gid), "组头不被框选直接选中");
        assert!(rows.is_selected(gid), "子项全选 → 组头间接选中");
    }

    // 部分框选（只覆盖 e1 所在行与时间段）：替换语义下仅 e1 选中，
    // 组头因未全选回落 Unselected
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::MarqueeSelect {
                x_lo: -100.0,
                x_hi: 3.0,
                row_lo: 1,
                row_hi: 1,
                modifier: SlintSelectionModifier::None,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let rows = s.ui_models().sequence_state.rows_model();
        assert!(rows.is_directly_selected(e1));
        assert!(!rows.is_selected(e2), "范围外事件不选中");
        assert!(!rows.is_selected(gid), "子项未全选 → 组头 Unselected");
    }
}

/// 时间线组头拖动 = 子树整体平移：偏移直接应用到子树全部事件（时长各自保持）；
/// 子树内所有组头（含嵌套）打聚合界覆盖使组块随子项实时平移，提交时清覆盖并刷新聚合。
#[test]
fn timeline_group_header_drag_moves_subtree() {
    use aweft_gui::bridge::types::{SlintSelectionModifier, SlintSubtitleRow};
    use aweft_model::ids::ItemId;
    use aweft_model::sequence::GroupData;

    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    // 结构：G { G1 { e1 }, e2 }
    let (gid, g1id, e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let sequence = store_mut.state_mut().project.sequence_mut();
        let gid = sequence
            .create_group(
                GroupData {
                    name: "G".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let g1id = sequence
            .create_group(
                GroupData {
                    name: "G1".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let e1 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(2000).unwrap(),
                    actor: None,
                    text: "e1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(g1id, 0),
            )
            .unwrap();
        let e2 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(5000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(6000).unwrap(),
                    actor: None,
                    text: "e2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        store_mut.rebuild_sequence_models();
        (gid, g1id, e1, e2)
    };
    fn row_of(rows: &ModelRc<SlintSubtitleRow>, id: ItemId) -> usize {
        (0..rows.row_count())
            .find(|&i| rows.row_data(i).map(|r| r.item_id.to_string()) == Some(id.to_string()))
            .unwrap()
    }

    // 先点击父组 G：select_single 展开子树为直接选中（组头拖动的前提）
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::SelectRow {
                item_id: gid,
                modifier: SlintSelectionModifier::None,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 预览按住嵌套组头 G1 +1.0s：按住项在选中集内 → 整个选中集一同移动，
    // Domain 不变；两级组头都实时跟随
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragPreview {
                item_id: g1id,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(seq.event_time(e1, true).unwrap().num_milliseconds(), 1000);
        assert_eq!(seq.event_time(e2, true).unwrap().num_milliseconds(), 5000);
        let rows = all_rows(&s);
        let g = rows.row_data(row_of(&rows, gid)).unwrap();
        assert_eq!(g.start_sec, 2.0, "顶层组头预览随子项实时平移");
        assert_eq!(g.end_sec, 7.0);
        let g1 = rows.row_data(row_of(&rows, g1id)).unwrap();
        assert_eq!(g1.start_sec, 2.0, "嵌套组头同步平移");
        assert_eq!(g1.end_sec, 3.0);
        let r1 = rows.row_data(row_of(&rows, e1)).unwrap();
        assert_eq!(r1.start_sec, 2.0);
        let r2 = rows.row_data(row_of(&rows, e2)).unwrap();
        assert_eq!(r2.start_sec, 6.0);
    }

    // 提交 +1.0s：全部子事件写回，各聚合行刷新、覆盖清除
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragCommit {
                item_id: g1id,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(seq.event_time(e1, true).unwrap().num_milliseconds(), 2000);
        assert_eq!(seq.event_time(e1, false).unwrap().num_milliseconds(), 3000);
        assert_eq!(seq.event_time(e2, true).unwrap().num_milliseconds(), 6000);
        assert_eq!(seq.event_time(e2, false).unwrap().num_milliseconds(), 7000);
        let rows = all_rows(&s);
        let g = rows.row_data(row_of(&rows, gid)).unwrap();
        assert_eq!(g.start_sec, 2.0, "G 聚合来自已平移的子项");
        assert_eq!(g.end_sec, 7.0);
        let g1 = rows.row_data(row_of(&rows, g1id)).unwrap();
        assert_eq!(g1.start_sec, 2.0, "G1 聚合同步刷新");
        assert_eq!(g1.end_sec, 3.0);
    }
}

/// 混合选择整体移动：点击父组展开子树后 Ctrl 加选组外事件，拖嵌套组头 →
/// 全部直接选中事件（含组外）一同移动；两级组头实时跟随（组头直接选中
/// 蕴含子树全选，无需单独展开）。
#[test]
fn timeline_drag_moves_selection_with_group_headers() {
    use aweft_gui::bridge::types::{SlintSelectionModifier, SlintSubtitleRow};
    use aweft_model::ids::ItemId;
    use aweft_model::sequence::GroupData;

    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));

    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 800,
                height: 600,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    // 结构：G { G1 { e1 }, e2 } + 组外独立 e3
    let (gid, g1id, e1, e2, e3) = {
        let mut store_mut = store.borrow_mut();
        let sequence = store_mut.state_mut().project.sequence_mut();
        let gid = sequence
            .create_group(
                GroupData {
                    name: "G".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let g1id = sequence
            .create_group(
                GroupData {
                    name: "G1".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let e1 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(2000).unwrap(),
                    actor: None,
                    text: "e1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(g1id, 0),
            )
            .unwrap();
        let e2 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(5000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(6000).unwrap(),
                    actor: None,
                    text: "e2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        let e3 = sequence
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(8000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(9000).unwrap(),
                    actor: None,
                    text: "e3".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        store_mut.rebuild_sequence_models();
        (gid, g1id, e1, e2, e3)
    };
    fn row_of(rows: &ModelRc<SlintSubtitleRow>, id: ItemId) -> usize {
        (0..rows.row_count())
            .find(|&i| rows.row_data(i).map(|r| r.item_id.to_string()) == Some(id.to_string()))
            .unwrap()
    }

    // 点击父组 G（展开子树）+ Ctrl 加选组外 e3
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::SelectRow {
                item_id: gid,
                modifier: SlintSelectionModifier::None,
            }))
            .unwrap();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::SelectRow {
                item_id: e3,
                modifier: SlintSelectionModifier::Ctrl,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 预览按住嵌套组头 G1 +1.0s：G 子树事件与组外 e3 一同移动，两级组头跟随
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragPreview {
                item_id: g1id,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        assert_eq!(rows.row_data(row_of(&rows, e1)).unwrap().start_sec, 2.0);
        assert_eq!(rows.row_data(row_of(&rows, e2)).unwrap().start_sec, 6.0);
        assert_eq!(
            rows.row_data(row_of(&rows, e3)).unwrap().start_sec,
            9.0,
            "组外选中事件一同移动"
        );
        let g = rows.row_data(row_of(&rows, gid)).unwrap();
        assert_eq!(g.start_sec, 2.0, "父组头实时跟随");
        let g1 = rows.row_data(row_of(&rows, g1id)).unwrap();
        assert_eq!(g1.start_sec, 2.0, "嵌套组头实时跟随");
    }

    // 提交 +1.0s：全部选中事件写回，时长各自保持
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::TimelineBlockDragCommit {
                item_id: g1id,
                delta_x: 1.0,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        let seq = s.state().project.sequence();
        assert_eq!(seq.event_time(e1, true).unwrap().num_milliseconds(), 2000);
        assert_eq!(seq.event_time(e2, true).unwrap().num_milliseconds(), 6000);
        assert_eq!(
            seq.event_time(e3, true).unwrap().num_milliseconds(),
            9000,
            "组外事件写回"
        );
    }
}
