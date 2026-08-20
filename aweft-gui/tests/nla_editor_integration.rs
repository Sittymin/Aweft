//! NLA 编辑器无头集成测试。
//!
//! 与 depsgraph_integration.rs 同模式：`dispatch(NlaAction)` → reducer 改数据层
//! + model + 打 tag → `evaluate_depsgraph()` → 投影更新派生层（Slint 模型）。
//!
//! 参考 Slint examples/todo 的测试模式：
//! `i_slint_backend_testing::init_no_event_loop()` + `MainWindow::new()`。

use aweft_gui::actions::Action;
use aweft_gui::actions::animation::{AnimationAction, nla::NlaAction};
use aweft_gui::bridge::types::{MainWindow, SlintSelectionModifier};
use aweft_gui::store::AppStore;
use slint::Model;

/// 装载后派生层与 model 对齐：第 0 轨全局 Action + 3 条 model 轨道，
/// 片断时间解析（Full/Absolute placement → start/end）。
#[test]
fn nla_load_from_model_builds_derived_models() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    let nla = &store.ui_models().animation.nla;
    assert_eq!(
        nla.tracks_filter.row_count(),
        4,
        "全局 Action 轨 + 3 条 NLA 轨道"
    );
    assert!(nla.tracks_filter.row_data(0).unwrap().is_action);
    assert_eq!(nla.tracks_filter.row_data(1).unwrap().name, "Idle");
    assert_eq!(nla.tracks_filter.row_data(2).unwrap().name, "动作");
    assert_eq!(nla.tracks_filter.row_data(3).unwrap().name, "Meta");

    // 轨道 2"动作"：奔跑 [2,6] + 跳跃 [8,12]
    let strips = nla.strip_models[2].clone();
    assert_eq!(strips.row_count(), 2);
    let s0 = strips.row_data(0).unwrap();
    assert_eq!(s0.name, "奔跑");
    assert_eq!(s0.start, 2.0);
    assert_eq!(s0.end, 6.0);

    // model 是权威：AnimData 已存库
    let anim_data = store
        .state()
        .project
        .library
        .animation_library
        .anim_datas
        .get(&nla.anim_data_id.unwrap())
        .unwrap();
    assert_eq!(anim_data.tracks.len(), 3);
}

/// 拖拽全闭环：按下（数据层选中 + 投影同步）→ 拖拽（就地更新，保住 grab）→
/// 松开（数据层整理 + 写回 model）。
#[test]
fn nla_strip_drag_commits_to_model() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // 按下（轨道 2"动作"的奔跑片断，权威下标 2 过滤视图行 2）
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripPressed {
            track: 2,
            strip: 0,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let nla = &store.ui_models().animation.nla;
        let s = nla.strip_models[2].row_data(0).unwrap();
        assert!(s.selected, "按下后选中");
        assert!(!nla.strip_models[2].row_data(1).unwrap().selected);
    }

    // 拖拽 dt=1.0：奔跑 [2,6] → [3,7]（与跳跃 [8,12] 不重叠）
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripDragged {
            track: 2,
            strip: 0,
            delta_x: 1.0,
            delta_row: 0.0,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let nla = &store.ui_models().animation.nla;
        let s = nla.strip_models[2].row_data(0).unwrap();
        assert_eq!(s.start, 3.0, "拖拽中派生层就地更新");
        assert_eq!(s.end, 7.0);
        assert!(s.dragging, "拖拽中显示移动色");
    }

    // 松开：写回 model（placement = Absolute{3,7}）
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripReleased { track: 2, strip: 0 })))
        .unwrap();
    store.evaluate_depsgraph();
    let nla = &store.ui_models().animation.nla;
    assert!(
        !nla.strip_models[2].row_data(0).unwrap().dragging,
        "松开后退出移动色"
    );
    let anim_data = store
        .state()
        .project
        .library
        .animation_library
        .anim_datas
        .get(&nla.anim_data_id.unwrap())
        .unwrap();
    let strip = &anim_data.tracks[1].strips[0];
    assert_eq!(
        strip.placement,
        aweft_model::animation::strip::StripPlacement::Absolute {
            start: 3.0,
            end: 7.0,
        },
        "松开后 model 写回新时间范围"
    );
}

/// 跨轨拖拽：垂直位移越过轨道边界 → 拖动中瞬间移动到目标轨道，
/// 松开后 model 轨道结构同步。
#[test]
fn nla_strip_drag_moves_across_tracks_and_commits() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // 按下轨道 1（Idle）的片断
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripPressed {
            track: 1,
            strip: 0,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();

    // 向下拖 1.2 行（从轨道 1 到轨道 2）
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripDragged {
            track: 1,
            strip: 0,
            delta_x: 0.0,
            delta_row: 1.2,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let nla = &store.ui_models().animation.nla;
        assert_eq!(nla.strip_models[1].row_count(), 0, "源轨已移出");
        assert_eq!(
            nla.strip_models[2].row_count(),
            3,
            "目标轨插入（按 start 排序）"
        );
    }

    // 松开 → 写回 model
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripReleased { track: 1, strip: 0 })))
        .unwrap();
    store.evaluate_depsgraph();
    let nla = &store.ui_models().animation.nla;
    let anim_data = store
        .state()
        .project
        .library
        .animation_library
        .anim_datas
        .get(&nla.anim_data_id.unwrap())
        .unwrap();
    assert_eq!(anim_data.tracks[0].strips.len(), 0, "model 源轨已清空");
    assert_eq!(
        anim_data.tracks[1].strips.len(),
        3,
        "model 目标轨含移入片断"
    );
    assert_eq!(
        anim_data.tracks[1].strips[0].name, "奔跑",
        "按 start 排序后在最前"
    );
}

/// 轨道开关/搜索/向下插入：结构变化走投影全量重建，model 同步。
#[test]
fn nla_track_ops_and_push_down() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // 轨道启用开关：model muted 同步
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::ToggleTrackEnabled { track: 2 })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let nla = &store.ui_models().animation.nla;
        assert!(
            !nla.tracks_filter.row_data(2).unwrap().enabled,
            "投影同步关闭态"
        );
        let anim_data = store
            .state()
            .project
            .library
            .animation_library
            .anim_datas
            .get(&nla.anim_data_id.unwrap())
            .unwrap();
        assert!(anim_data.tracks[1].muted, "model NlaTrack.muted 同步");
    }

    // 搜索：只显示匹配轨道
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::SearchChanged {
            text: "Idle".into(),
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let nla = &store.ui_models().animation.nla;
        assert_eq!(nla.tracks_filter.row_count(), 1);
        assert_eq!(nla.tracks_filter.row_data(0).unwrap().orig_index, 1);
    }

    // 向下插入：把当前 Action 作为片断加入能容纳的轨道（Idle 已满 0-15 → 自动新建）
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::SearchChanged { text: "".into() })))
        .unwrap();
    store.evaluate_depsgraph();
    store.dispatch(Action::Animation(AnimationAction::Nla(NlaAction::PushDown))).unwrap();
    store.evaluate_depsgraph();
    let nla = &store.ui_models().animation.nla;
    assert_eq!(nla.tracks_filter.row_count(), 5, "新建一条 NLA 片段轨道");
    let anim_data = store
        .state()
        .project
        .library
        .animation_library
        .anim_datas
        .get(&nla.anim_data_id.unwrap())
        .unwrap();
    assert_eq!(anim_data.tracks.len(), 4, "model 同步新建轨道");
    let last = anim_data.tracks.last().unwrap();
    assert_eq!(last.strips.len(), 1, "新轨道含下插片断");
    assert_eq!(last.strips[0].name, "主 Action");
}

/// 过滤视图命中：strip-pick（同步回调路径）经过滤行映射回权威轨道。
/// 选中修改的数据层/投影细节由 store 单测覆盖（`strip_pick_maps_filter_row_to_authoritative`）。
#[test]
fn nla_strip_pick_maps_filtered_row() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // 搜索过滤后只剩"动作"轨道（过滤行 0 = 权威轨 2）
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::SearchChanged {
            text: "动作".into(),
        })))
        .unwrap();
    store.evaluate_depsgraph();
    let (hit, _tags) =
        store
            .ui_models_mut().animation.nla
            .strip_pick(3.0, 0, SlintSelectionModifier::None);
    assert_eq!(hit, 2 * 10000 + 0, "过滤行 → 权威轨道映射");
}

/// 回归：水平穿越重排后跨轨——dispatch 全管线（reducer → tag → 投影）下
/// 派生层与数据层保持一致，不误激活/误移动其他片断。
#[test]
fn nla_cross_track_after_reorder_keeps_layers_in_sync() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    let dispatch = |store: &mut AppStore, action: NlaAction| {
        store
            .dispatch(Action::Animation(AnimationAction::Nla(action)))
            .unwrap();
        store.evaluate_depsgraph();
    };

    // 轨道 2"动作"：奔跑 [2,6] + 跳跃 [8,12]。按下奔跑（下标 0）
    dispatch(
        &mut store,
        NlaAction::StripPressed {
            track: 2,
            strip: 0,
            modifier: SlintSelectionModifier::None,
        },
    );

    // 水平拖到 [13,17]：穿越跳跃，重排后奔跑下标 1
    dispatch(
        &mut store,
        NlaAction::StripDragged {
            track: 2,
            strip: 0,
            delta_x: 11.0,
            delta_row: 0.0,
        },
    );
    {
        let nla = &store.ui_models().animation.nla;
        assert_eq!(
            nla.strip_models[2].row_data(1).unwrap().name,
            "奔跑",
            "派生层同轨重排同步"
        );
    }

    // 跨轨到轨道 3（Meta 轨，空）：奔跑移入
    dispatch(
        &mut store,
        NlaAction::StripDragged {
            track: 2,
            strip: 0,
            delta_x: 11.0,
            delta_row: 1.2,
        },
    );
    {
        let nla = &store.ui_models().animation.nla;
        assert_eq!(nla.strip_models[2].row_count(), 1, "源轨只剩跳跃");
        assert_eq!(
            nla.strip_models[2].row_data(0).unwrap().name,
            "跳跃",
            "跳跃未被误移"
        );
        assert_eq!(nla.strip_models[3].row_count(), 2, "目标轨 = 组合 + 奔跑");
        let run = (0..nla.strip_models[3].row_count())
            .find_map(|i| nla.strip_models[3].row_data(i).filter(|s| s.name == "奔跑"))
            .unwrap();
        assert_eq!(run.start, 13.0, "奔跑保持水平位置");
        assert!(run.dragging, "拖动标记跟随奔跑");
        assert!(
            !nla.strip_models[3].row_data(0).unwrap().dragging,
            "组合不受影响"
        );
    }

    // 松开：写回 model，拖动标记清除
    dispatch(&mut store, NlaAction::StripReleased { track: 2, strip: 0 });
    let nla = &store.ui_models().animation.nla;
    assert!(
        !nla.strip_models[3].row_data(1).unwrap().dragging,
        "松开后退出移动色"
    );
    let anim_data = store
        .state()
        .project
        .library
        .animation_library
        .anim_datas
        .get(&nla.anim_data_id.unwrap())
        .unwrap();
    assert_eq!(
        anim_data.tracks[2].strips.len(),
        2,
        "model 目标轨 = 组合 + 奔跑"
    );
    assert_eq!(anim_data.tracks[1].strips.len(), 1, "model 源轨只剩跳跃");
    assert_eq!(anim_data.tracks[1].strips[0].name, "跳跃");
}

/// 点击空白取消选中：选中片断后，空白点击 → deselect-all → 派生层全部清除选中。
#[test]
fn nla_blank_click_deselects_all() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // 选中奔跑片断
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripPressed {
            track: 2,
            strip: 0,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    assert!(
        store.ui_models().animation.nla.strip_models[2]
            .row_data(0)
            .unwrap()
            .selected
    );

    // 空白点击（dispatch 层 = deselect-all）
    store
        .dispatch(Action::Animation(AnimationAction::Nla(NlaAction::StripDeselectAll)))
        .unwrap();
    store.evaluate_depsgraph();
    let nla = &store.ui_models().animation.nla;
    for track in 0..nla.strip_models.len() {
        for i in 0..nla.strip_models[track].row_count() {
            assert!(
                !nla.strip_models[track].row_data(i).unwrap().selected,
                "轨道 {track} 片断 {i} 应取消选中"
            );
        }
    }
}
