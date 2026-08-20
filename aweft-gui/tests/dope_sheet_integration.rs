//! 摄影表（Dope Sheet）无头集成测试。
//!
//! 与 depsgraph_integration.rs 的动画用例同模式：`dispatch(AnimationAction)`
//! → reducer 改数据层 + 打 tag → `evaluate_depsgraph()` → 投影更新派生层。
//! 摄影表与曲线编辑器共用同一份数据（channels/editing_keyframes/选中），
//! 拖拽路径 = 关键帧拖拽（dv=0，只移动时间）。
//!
//! 参考 Slint examples/todo 的测试模式：
//! `i_slint_backend_testing::init_no_event_loop()` + `MainWindow::new()`。

use aweft_gui::actions::Action;
use aweft_gui::actions::animation::{AnimationAction, graph::GraphAction};
use aweft_gui::bridge::types::{MainWindow, SlintSelectionModifier};
use aweft_gui::store::AppStore;
use slint::Model;

/// 摄影表关键帧拖拽 = 曲线编辑器同一条路径（dv=0）：只移动时间，值不变；
/// 松开写回 model + 拖拽结束整理（重排/补柄）。
#[test]
fn dope_keyframe_drag_moves_only_time_and_commits() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // 按下 (通道 0, 关键帧 1)（t=3）
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframePressed {
            curve: 0,
            key: 1,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let ce = &store.ui_models().animation.graph;
        assert_eq!(ce.selected_keyframe_count(), 1, "按下后选中 1 个关键帧");
    }

    // 拖拽 dt=1.0：时间 3 → 4，值 66 不变
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeDragged {
            curve: 0,
            key: 1,
            delta_time: 1.0,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let ce = &store.ui_models().animation.graph;
        let kf = ce
            .channels
            .row_data(0)
            .unwrap()
            .keyframes
            .row_data(1)
            .unwrap();
        assert_eq!(kf.time, 4.0, "摄影表拖拽应只移动时间");
        assert_eq!(kf.value, 66.0, "值不应变化");
        // 段跟随关键帧新位置（投影重建）
        let seg0 = ce.segments.row_data(0).unwrap();
        assert_eq!(seg0.x1, 4.0);
        // 汇总条带：新 ms(4000) 加入；旧 ms(3000) 的聚合更新（通道 3 BorderX 仍有 t=3 关键帧，count=1）
        let n = ce.summary_keyframes.row_count();
        assert!(
            (0..n).any(|i| {
                ce.summary_keyframes
                    .row_data(i)
                    .map(|sk| (sk.time - 4.0).abs() < 1e-6)
                    .unwrap_or(false)
            }),
            "汇总条带应出现新 ms(4000) 条目"
        );
        let sk3 = (0..n)
            .filter_map(|i| ce.summary_keyframes.row_data(i))
            .find(|sk| (sk.time - 3.0).abs() < 1e-6);
        assert!(sk3.is_some(), "ms(3000) 仍有通道 3 的关键帧，条目应保留");
        assert_eq!(sk3.unwrap().count, 1, "ms(3000) 聚合数应减为 1");
    }

    // 松开（moved=true）：写回 model + DragEnded 整理
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeReleased {
            curve: 0,
            key: 1,
            moved: true,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    let id = store.ui_models().animation.graph.current_action_id().unwrap();
    let action = store
        .state()
        .project
        .library
        .animation_library
        .actions
        .get(&id)
        .unwrap();
    let kf = action.curves[0]
        .curve
        .keyframes
        .iter()
        .find(|k| (k.time - 4.0).abs() < 1e-6);
    assert!(kf.is_some(), "model 应有 t=4 的关键帧（写回成功）");
}

/// 摄影表空白处框选：按世界时间范围 × 行区间选中关键帧（不含通道 0 的 t=0/t=3
/// 与通道 1 的 t=0，只命中通道 1 的 t=2.5）。
#[test]
fn dope_marquee_select_picks_keyframes_in_box() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeMarqueeSelect {
            x_lo: 0.5,
            x_hi: 2.8,
            row_lo: 0,
            row_hi: 1,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();

    let ce = &store.ui_models().animation.graph;
    assert_eq!(ce.selected_keyframe_count(), 1, "框内应只命中 1 个关键帧");
    let hit = ce
        .channels
        .row_data(1)
        .unwrap()
        .keyframes
        .iter()
        .filter(|k| k.selected)
        .collect::<Vec<_>>();
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].time, 2.5, "命中的应是通道 1 的 t=2.5 关键帧");
    // 通道选中：命中通道被选中，未命中的通道 0 不选中
    assert!(ce.channels.row_data(1).unwrap().selected);
    assert!(!ce.channels.row_data(0).unwrap().selected);
}

/// 摄影表汇总条带：按下选中该时刻全部通道的关键帧；拖拽跨通道同步移动；
/// 松开重算汇总条带（旧 ms 移除/新 ms 加入）+ 写回 model。
#[test]
fn dope_summary_press_drag_release_moves_all_channels() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // t=0 在全部 4 条通道都有关键帧 → 汇总条目 count=4（时间排序后第一个条目）
    let index = {
        let ce = &store.ui_models().animation.graph;
        let n = ce.summary_keyframes.row_count();
        (0..n)
            .find(|&i| {
                ce.summary_keyframes
                    .row_data(i)
                    .map(|sk| (sk.time - 0.0).abs() < 1e-6)
                    .unwrap_or(false)
            })
            .expect("汇总条带应有 t=0 条目")
    };

    // 按下（普通点击 = 单选该时刻全部）：跨通道全部选上
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryPressed {
            index,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let ce = &store.ui_models().animation.graph;
        assert_eq!(
            ce.selected_keyframe_count(),
            4,
            "t=0 时刻应有 4 个关键帧被选中"
        );
    }

    // 拖拽 dt=0.5：4 个关键帧同步移动到 t=0.5
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryDragged {
            index,
            delta_time: 0.5,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let ce = &store.ui_models().animation.graph;
        for c in 0..4 {
            let kf = ce
                .channels
                .row_data(c)
                .unwrap()
                .keyframes
                .row_data(0)
                .unwrap();
            assert_eq!(kf.time, 0.5, "通道 {c} 的关键帧应同步移动到 t=0.5");
        }
        // 被拖汇总标记就地跟随（拖拽期间不重建模型）
        let sk = ce.summary_keyframes.row_data(index).unwrap();
        assert!((sk.time - 0.5).abs() < 1e-6, "汇总标记应跟随光标到 t=0.5");
    }

    // 松开：汇总条带重算（t=0 空条目移除，t=0.5 条目 count=4）+ 写回 model
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryReleased {
            index,
            moved: true,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    {
        let ce = &store.ui_models().animation.graph;
        let n = ce.summary_keyframes.row_count();
        assert!(
            !(0..n).any(|i| {
                ce.summary_keyframes
                    .row_data(i)
                    .map(|sk| (sk.time - 0.0).abs() < 1e-6)
                    .unwrap_or(false)
            }),
            "松开后 t=0 空条目应移除"
        );
        let sk = (0..n)
            .filter_map(|i| ce.summary_keyframes.row_data(i))
            .find(|sk| (sk.time - 0.5).abs() < 1e-6)
            .expect("松开后应有 t=0.5 汇总条目");
        assert_eq!(sk.count, 4, "t=0.5 条目应聚合 4 个关键帧");
    }
    let id = store.ui_models().animation.graph.current_action_id().unwrap();
    let action = store
        .state()
        .project
        .library
        .animation_library
        .actions
        .get(&id)
        .unwrap();
    for curve in &action.curves {
        let kf = curve
            .curve
            .keyframes
            .iter()
            .find(|k| (k.time - 0.5).abs() < 1e-6);
        assert!(kf.is_some(), "model 应写入 t=0.5 关键帧（写回成功）");
    }
}

/// 框选竖直范围与行区无交集（行区下方空白）：不命中任何通道，关键帧不被选中
/// （clamp 旧行为会把空框映射到末行，水平范围一覆盖就误选）。
/// 替换模式仍清空既有选择（对齐"空白框选 = 清除"语义），且汇总条带掩码随之刷新。
#[test]
fn dope_marquee_below_rows_selects_nothing() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    // 先选中通道 0 的 t=3 关键帧（后续验证空框替换会清空它）
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframePressed {
            curve: 0,
            key: 1,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();
    assert_eq!(store.ui_models().animation.graph.selected_keyframe_count(), 1);

    // 框选行区间 [4, 6]（n=4，完全在行区外），水平范围覆盖全部关键帧
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeMarqueeSelect {
            x_lo: -1.0,
            x_hi: 20.0,
            row_lo: 4,
            row_hi: 6,
            modifier: SlintSelectionModifier::None,
        })))
        .unwrap();
    store.evaluate_depsgraph();

    let ce = &store.ui_models().animation.graph;
    assert_eq!(
        ce.selected_keyframe_count(),
        0,
        "空框替换应清空选择，且不误选末行"
    );
    // 汇总条带掩码刷新：t=3 条目的逐通道选中掩码应全部为 false
    let n = ce.summary_keyframes.row_count();
    let sk3 = (0..n)
        .filter_map(|i| ce.summary_keyframes.row_data(i))
        .find(|sk| (sk.time - 3.0).abs() < 1e-6)
        .expect("t=3 汇总条目应存在");
    for i in 0..sk3.selected.row_count() {
        assert!(
            !sk3.selected.row_data(i).unwrap(),
            "汇总条带 t=3 掩码第 {i} 位应清空（空框替换）"
        );
    }
}

/// 摄影表搜索独立于曲线编辑器：只影响 dope_filter（graph_filter 不受影响）。
#[test]
fn dope_search_filters_independently() {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let mut store = AppStore::new(&instance);
    store.create_test_animation_data();

    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeSearchChanged {
            text: "font".to_string(),
        })))
        .unwrap();
    store.evaluate_depsgraph();

    let ce = &store.ui_models().animation.graph;
    assert_eq!(
        ce.dope_filter.row_count(),
        2,
        "摄影表过滤视图应只剩 Font 开头通道"
    );
    assert_eq!(ce.graph_filter.row_count(), 4, "曲线编辑器过滤视图不受影响");

    // 空串恢复全部
    store
        .dispatch(Action::Animation(AnimationAction::Graph(GraphAction::DopeSearchChanged {
            text: String::new(),
        })))
        .unwrap();
    store.evaluate_depsgraph();
    assert_eq!(
        store.ui_models().animation.graph.dope_filter.row_count(),
        4,
        "空串应恢复全部通道"
    );
}
