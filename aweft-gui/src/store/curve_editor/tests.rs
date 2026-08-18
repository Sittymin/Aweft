use super::*;
use crate::depsgraph::{HandleLoc, Tag};
use aweft_model::animation::{
    action::ActionCurve, fcurve::FCurve, Action as ModelAction, Keyframe as Mk,
    PropertyPath,
};
use crate::bridge::types::{
    SlintBezierHandle, SlintColorMode, SlintHandleDragMode, SlintInterpolation,
    SlintInterpolationState, SlintKeyframe, SlintKeyframeDragMode, SlintSegmentKind,
    SlintSelectionModifier,
};
use slint::{Color, Model};

/// 调用 reducer 后手动跑投影（测试中替代 depsgraph evaluate）。
fn project(st: &mut CurveEditorState, tags: Vec<Tag>) {
    st.apply_tags(&tags);
}

fn key(time: f32, value: f32, interpolation: SlintInterpolation) -> SlintKeyframe {
    SlintKeyframe {
        time,
        value,
        interpolation,
        handle_in: SlintBezierHandle {
            time_offset: 0.0,
            value_offset: 0.0,
        },
        handle_out: SlintBezierHandle {
            time_offset: 0.0,
            value_offset: 0.0,
        },
        selected: false,
    }
}

#[test]
fn segments_derived_for_each_interpolation() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle {
        time_offset: 1.0,
        value_offset: 5.0,
    };
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            k0,
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 10.0, SlintInterpolation::Hold),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 3 个关键帧 → 2 段
    assert_eq!(st.segments.row_count(), 2);

    let seg0 = st.segments.row_data(0).unwrap();
    // 第一段为 bezier（首关键帧插值），控制点 = 关键帧 + 出柄偏移
    assert_eq!(seg0.kind, SlintSegmentKind::Bezier);
    assert_eq!(seg0.cx1, 1.0);
    assert_eq!(seg0.cy1, 5.0);
    assert_eq!(seg0.x0, 0.0);
    assert_eq!(seg0.x1, 2.0);
    assert_eq!(seg0.curve_index, 0);

    let seg1 = st.segments.row_data(1).unwrap();
    // 第二段 linear：控制点位于直线上
    assert_eq!(seg1.kind, SlintSegmentKind::Linear);
    assert_eq!(seg1.cx1, 2.0 + 2.0 / 3.0);
    assert_eq!(seg1.cy1, 10.0);
    assert_eq!(seg1.x1, 4.0);
    assert_eq!(seg1.y1, 10.0);
}

#[test]
fn keyframe_drag_updates_model_and_segments() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 20.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 1, 1.0, -5.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);

    let kf = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf.time, 3.0);
    assert_eq!(kf.value, 5.0);
    assert!(kf.selected);

    // 段同步更新：段 0 终点 = 新位置
    let seg0 = st.segments.row_data(0).unwrap();
    assert_eq!(seg0.x1, 3.0);
    assert_eq!(seg0.y1, 5.0);

    // 多次拖拽累加（与节点拖拽一致）
    let tags = st.keyframe_dragged(0, 1, 0.5, 2.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf.time, 3.5);
    assert_eq!(kf.value, 7.0);
    let tags = st.keyframe_released(0, 1, true);
    project(&mut st, tags);
}

#[test]
fn action_range_derived_from_keyframe_times() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(2.0, 0.0, SlintInterpolation::Linear),
            key(5.0, 10.0, SlintInterpolation::Linear),
            key(9.0, 20.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    // push 走 Structure 投影 → 派生层立即重建（含时长范围）
    assert_eq!((st.action_start, st.action_end), (2.0, 9.0));
}

#[test]
fn action_selection_switches_curves() {
    use aweft_model::animation::{
        action::ActionCurve, fcurve::FCurve, Action as ModelAction, Keyframe as Mk,
    };
    use crate::store::curve_editor::ActionData;

    fn action_with(name: &str, property: PropertyPath, kfs: Vec<Mk>) -> ModelAction {
        let mut fcurve = FCurve::default();
        fcurve.keyframes = kfs;
        let mut a = ModelAction::new(name);
        a.curves = vec![ActionCurve::new(property, fcurve)];
        a
    }

    let mut st = CurveEditorState::default();
    // 默认：无 Action
    assert_eq!(st.current_action, -1);
    st.apply_tags(&[Tag::AnimationStructureChanged]);
    assert_eq!(st.action_items.row_count(), 0);

    let act_a = action_with(
        "主 Action",
        PropertyPath::FontSize,
        vec![Mk::linear(0.0, 0.0), Mk::linear(4.0, 4.0)],
    );
    let act_b = action_with(
        "奔跑",
        PropertyPath::PositionX,
        vec![Mk::linear(1.0, 10.0), Mk::linear(2.0, 20.0)],
    );
    let id_a = act_a.id;
    let id_b = act_b.id;

    st.actions = vec![
        ActionData {
            name: "主 Action".into(),
            id: id_a,
        },
        ActionData {
            name: "奔跑".into(),
            id: id_b,
        },
    ];
    st.apply_tags(&[Tag::AnimationStructureChanged]);
    assert_eq!(st.action_items.row_count(), 2);

    let tags = st.load_action(0, &act_a);
    st.apply_tags(&tags);
    assert_eq!(st.current_action, 0);
    assert_eq!(st.action_name.as_str(), "主 Action");
    assert_eq!(st.channels.row_count(), 1);
    assert_eq!(st.channels.row_data(0).unwrap().name.as_str(), "FontSize");
    assert_eq!((st.action_start, st.action_end), (0.0, 4.0));

    // 切换 Action：通道/关键帧/时长范围全部换掉
    let tags = st.load_action(1, &act_b);
    project(&mut st, tags);
    assert_eq!(st.current_action, 1);
    assert_eq!(st.action_name.as_str(), "奔跑");
    assert_eq!(st.channels.row_count(), 1);
    assert_eq!(st.channels.row_data(0).unwrap().name.as_str(), "PositionX");
    assert_eq!((st.action_start, st.action_end), (1.0, 2.0));

    // 重置：清空当前 Action
    let tags = st.reset_action();
    project(&mut st, tags);
    assert_eq!(st.current_action, -1);
    assert_eq!(st.action_name.as_str(), "");
    assert_eq!(st.channels.row_count(), 0);
}

#[test]
fn load_action_preserves_model_bezier_handles() {
    use aweft_model::animation::{
        action::ActionCurve, fcurve::FCurve, Action as ModelAction, BezierHandle,
        Interpolation as ModelInterp, Keyframe as Mk,
    };

    let k0 = Mk::bezier(
        0.0,
        5.0,
        BezierHandle::new(-0.3, -1.5),
        BezierHandle::new(0.4, 2.0),
    );
    let mut fcurve = FCurve::default();
    fcurve.keyframes = vec![k0, Mk::hold(2.0, 8.0), Mk::linear(4.0, 12.0)];
    let mut a = ModelAction::new("测试");
    a.curves = vec![ActionCurve::new(PropertyPath::FontSize, fcurve)];

    let mut st = CurveEditorState::default();
    let tags = st.load_action(0, &a);
    project(&mut st, tags);
    assert_eq!(st.channels.row_count(), 1);
    assert_eq!(st.channels.row_data(0).unwrap().name.as_str(), "FontSize");

    let model = st.keyframe_models.get(&0).unwrap();
    assert_eq!(model.row_count(), 3);
    let first = model.row_data(0).unwrap();
    assert_eq!(first.time, 0.0);
    assert_eq!(first.value, 5.0);
    assert_eq!(first.interpolation, SlintInterpolation::Bezier);
    assert_eq!(first.handle_in.time_offset, -0.3);
    assert_eq!(first.handle_in.value_offset, -1.5);
    assert_eq!(first.handle_out.time_offset, 0.4);
    assert_eq!(first.handle_out.value_offset, 2.0);
    assert_eq!(model.row_data(1).unwrap().interpolation, SlintInterpolation::Hold);
    assert_eq!(model.row_data(2).unwrap().interpolation, SlintInterpolation::Linear);
}

#[test]
fn commit_to_model_writes_view_keyframes_sorted() {
    use aweft_model::animation::{
        action::ActionCurve, fcurve::FCurve, Action as ModelAction, Keyframe as Mk,
    };

    // model 初始关键帧
    let mut fcurve = FCurve::default();
    fcurve.keyframes = vec![
        Mk::linear(0.0, 0.0),
        Mk::linear(2.0, 2.0),
        Mk::linear(4.0, 4.0),
    ];
    let mut a = ModelAction::new("测试");
    a.curves = vec![ActionCurve::new(PropertyPath::PositionX, fcurve)];

    let mut st = CurveEditorState::default();
    let tags = st.load_action(0, &a);
    project(&mut st, tags);

    // 选中关键帧 0 并拖拽：time 0.0 → 3.0（越过 2.0，视图会重排）
    let ce = &mut st;
    ce.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    ce.keyframe_dragged(0, 0, 3.0, 1.0, SlintKeyframeDragMode::Move);
    ce.keyframe_released(0, 0, true);

    // 提交：视图状态写回 model，且按时间排序
    ce.commit_to_model(&mut a);
    let kfs = &a.curves[0].curve.keyframes;
    assert_eq!(kfs.len(), 3);
    let times: Vec<f64> = kfs.iter().map(|k| k.time).collect();
    let mut sorted = times.clone();
    sorted.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(times, sorted, "提交后 model 关键帧必须按时间排序");
    // 被拖关键帧（含增量）出现在 3.0；原 2.0 关键帧仍在
    assert!(kfs.iter().any(|k| (k.time - 3.0).abs() < 1e-6 && (k.value - 1.0).abs() < 1e-6));
    assert!(kfs.iter().any(|k| (k.time - 2.0).abs() < 1e-6));
}

#[test]
fn commit_to_model_round_trips_bezier_handles() {
    use aweft_model::animation::{
        action::ActionCurve, fcurve::FCurve, Action as ModelAction, BezierHandle,
        Interpolation as ModelInterp, Keyframe as Mk,
    };

    let k0 = Mk::bezier(
        0.0,
        1.0,
        BezierHandle::new(-0.2, -1.0),
        BezierHandle::new(0.3, 1.5),
    );
    let mut fcurve = FCurve::default();
    fcurve.keyframes = vec![k0];
    let mut a = ModelAction::new("测试");
    a.curves = vec![ActionCurve::new(PropertyPath::FontSize, fcurve)];

    let mut st = CurveEditorState::default();
    let tags = st.load_action(0, &a);
    project(&mut st, tags);
    st.commit_to_model(&mut a);

    let kfs = &a.curves[0].curve.keyframes;
    assert_eq!(kfs.len(), 1);
    match kfs[0].interpolation {
        ModelInterp::Bezier { handle_in, handle_out } => {
            // f32 → f64 有精度损失，用容差比较
            assert!((handle_in.time_offset - (-0.2)).abs() < 1e-5);
            assert!((handle_in.value_offset - (-1.0)).abs() < 1e-5);
            assert!((handle_out.time_offset - 0.3).abs() < 1e-5);
            assert!((handle_out.value_offset - 1.5).abs() < 1e-5);
        }
        other => panic!("应保持贝塞尔插值，得 {other:?}"),
    }
}

#[test]
fn handle_drag_updates_bezier_offset() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Bezier),
            key(2.0, 10.0, SlintInterpolation::Bezier),
            key(4.0, 20.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // 3 个 bezier 关键帧：首只画出柄、末只画入柄、中间两个 → 共 4 条控制柄
    assert_eq!(st.handles.row_count(), 4);

    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 0, false, 0.5, 3.0, SlintHandleDragMode::Free);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 0, false, 0.2, 1.0, SlintHandleDragMode::Free);
    project(&mut st, tags);

    let kf = st.keyframe_models[&0].row_data(0).unwrap();
    assert_eq!(kf.handle_out.time_offset, 0.7);
    assert_eq!(kf.handle_out.value_offset, 4.0);

    let tags = st.handle_released(0, 0, false, true);
    project(&mut st, tags);
}

#[test]
fn handle_drag_modes_constrain_opposite() {
    fn setup() -> CurveEditorState {
        let mut st = CurveEditorState::default();
        let tags = st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Bezier),
                key(2.0, 10.0, SlintInterpolation::Bezier),
            ],
        );
        project(&mut st, tags);
        st.rebuild_all();
        st
    }

    // 模式 0 自由：拖出柄，入柄不动
    let mut st = setup();
    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 0, false, 2.0, 3.0, SlintHandleDragMode::Free);
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(0).unwrap();
    assert_eq!(kf.handle_out.time_offset, 2.0);
    assert_eq!(kf.handle_out.value_offset, 3.0);
    assert_eq!(kf.handle_in.time_offset, 0.0);
    assert_eq!(kf.handle_in.value_offset, 0.0);
    let tags = st.handle_released(0, 0, false, true);
    project(&mut st, tags);

    // 模式 1 对齐：被拖柄自由移动；对手柄旋转共线、长度保持自身
    let mut st = setup();
    st.editing_keyframes.get_mut(&0).unwrap()[0] = SlintKeyframe {
        time: 0.0,
        value: 0.0,
        interpolation: SlintInterpolation::Bezier,
        handle_in: SlintBezierHandle { time_offset: 0.0, value_offset: 4.0 },
        handle_out: SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 },
        selected: false,
    };
    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    // 拖出柄 (1,0) → 累加 (1,1) 得 (2,1)；入柄旋转到共线方向、长度保持 4
    let tags = st.handle_dragged(0, 0, false, 1.0, 1.0, SlintHandleDragMode::Aligned);
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(0).unwrap();
    assert_eq!(kf.handle_out.time_offset, 2.0);
    assert_eq!(kf.handle_out.value_offset, 1.0);
    let nl = (2f32 * 2f32 + 1f32).sqrt();
    assert!((kf.handle_in.time_offset - (-2.0 / nl * 4.0)).abs() < 1e-4);
    assert!((kf.handle_in.value_offset - (-1.0 / nl * 4.0)).abs() < 1e-4);
    let tags = st.handle_released(0, 0, false, true);
    project(&mut st, tags);

    // 模式 2 原点对称：入柄 = -出柄（角度+长度都镜像）
    let mut st = setup();
    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 0, false, 2.0, 3.0, SlintHandleDragMode::Symmetric);
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(0).unwrap();
    assert_eq!(kf.handle_in.time_offset, -2.0);
    assert_eq!(kf.handle_in.value_offset, -3.0);
    let tags = st.handle_released(0, 0, false, true);
    project(&mut st, tags);
}

#[test]
fn endpoint_handles_only_outward_side() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Bezier),
            key(2.0, 10.0, SlintInterpolation::Bezier),
            key(4.0, 20.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    let h0 = st.handles.row_data(0).unwrap();
    let h1 = st.handles.row_data(1).unwrap();
    let h2 = st.handles.row_data(2).unwrap();
    let h3 = st.handles.row_data(3).unwrap();
    // 首关键帧只有出柄
    assert_eq!((h0.keyframe_index, h0.is_in), (0, false));
    // 中间关键帧 入 + 出
    assert_eq!((h1.keyframe_index, h1.is_in), (1, true));
    assert_eq!((h2.keyframe_index, h2.is_in), (1, false));
    // 末关键帧只有入柄
    assert_eq!((h3.keyframe_index, h3.is_in), (2, true));
}

#[test]
fn segment_render_soft_clamps_to_keyframe_interval() {
    let mut st = CurveEditorState::default();
    // k0 出柄 +5（P1.x=5），k1 入柄 -3（P2.x=1）→ 各自钳到 [0,4]，不取中点
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 5.0, value_offset: 0.0 };
    let mut k1 = key(4.0, 10.0, SlintInterpolation::Bezier);
    k1.handle_in = SlintBezierHandle { time_offset: -3.0, value_offset: 0.0 };
    let tags = st.push_channel("t", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
    project(&mut st, tags);
    st.rebuild_all();

    let seg = st.segments.row_data(0).unwrap();
    // 越过相邻关键帧的钳到相邻关键帧时间（无缝隙）
    assert_eq!(seg.cx1, 4.0);
    assert_eq!(seg.cx2, 1.0);
    assert!(seg.cx1 >= 0.0 && seg.cx2 <= 4.0);
    // 值保持不变
    assert_eq!(seg.cy1, 0.0);
    assert_eq!(seg.cy2, 10.0);
}

#[test]
fn handle_dragged_own_keyframe_hard_clamped() {
    // 手柄拖拽时只有"自己关键帧"是硬钳制；相邻关键帧边界是软钳制（渲染期处理）
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Bezier),
            key(4.0, 10.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 出柄拖 -10（越过自己关键帧之前）→ 硬钳到 0
    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 0, false, -10.0, 0.0, SlintHandleDragMode::Free);
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(0).unwrap();
    assert_eq!(kf.handle_out.time_offset, 0.0);
    let tags = st.handle_released(0, 0, false, true);
    project(&mut st, tags);

    // 入柄拖 +10（越过自己关键帧之后）→ 硬钳到 0
    let tags = st.handle_pressed(0, 1, true, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 1, true, 10.0, 0.0, SlintHandleDragMode::Free);
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf.handle_in.time_offset, 0.0);
    let tags = st.handle_released(0, 1, true, true);
    project(&mut st, tags);

    // 出柄拖 +10（越过相邻关键帧）→ 软钳制，手柄本身保持 10（曲线由渲染期钳制）
    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 0, false, 10.0, 0.0, SlintHandleDragMode::Free);
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(0).unwrap();
    assert_eq!(kf.handle_out.time_offset, 10.0);
    let tags = st.handle_released(0, 0, false, true);
    project(&mut st, tags);
}

#[test]
fn keyframe_drag_past_neighbor_resorts_curve() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 20.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 把中间关键帧（2,10）拖到时间 5，越过最后关键帧（4,20）
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 1, 3.0, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);

    // 模型顺序不变（索引 1 仍是它），时间变成 5
    let kf = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf.time, 5.0);

    // 曲线段按时间排序重绘：(0,0)-(4,20) 然后 (4,20)-(5,10)
    let seg0 = st.segments.row_data(0).unwrap();
    assert_eq!((seg0.x0, seg0.y0, seg0.x1, seg0.y1), (0.0, 0.0, 4.0, 20.0));
    let seg1 = st.segments.row_data(1).unwrap();
    assert_eq!((seg1.x0, seg1.y0, seg1.x1, seg1.y1), (4.0, 20.0, 5.0, 10.0));
    let tags = st.keyframe_released(0, 1, true);
    project(&mut st, tags);
}

#[test]
fn dope_drag_moves_only_time_and_is_shared_source() {
    // 摄影表拖拽路径 = keyframe_dragged(curve, key, dt, dv=0)：只移动时间。
    // 摄影表与曲线编辑器必须同源：channels[i].keyframes 指向与权威 keyframe_models 同一 inner 模型，
    // 拖拽后曲线编辑器的 segments 也同步重算。
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 5.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 20.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 1, 2.5, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);

    // 权威模型：时间移动 2.5、值不变（少一个维度）
    let kf = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf.time, 4.5);
    assert_eq!(kf.value, 10.0);

    // 同源：渲染给 Slint 的通道投影指向同一 inner 模型（摄影表/曲线编辑器读到同一关键帧）
    let ch = st.channels.row_data(0).unwrap();
    let proj = ch.keyframes.row_data(1).unwrap();
    assert_eq!(proj.time, 4.5);
    assert_eq!(proj.value, 10.0);

    // 曲线编辑器视图同步：松开后段按新时间排序重算
    let tags = st.keyframe_released(0, 1, true);
    project(&mut st, tags);
    let seg0 = st.segments.row_data(0).unwrap();
    let seg1 = st.segments.row_data(1).unwrap();
    assert_eq!((seg0.x0, seg0.y0, seg0.x1, seg0.y1), (0.0, 5.0, 4.0, 20.0));
    assert_eq!((seg1.x0, seg1.y0, seg1.x1, seg1.y1), (4.0, 20.0, 4.5, 10.0));
}

#[test]
fn handles_respect_neighbor_interpolation() {
    // A(0) 贝塞尔、B(2) 线性、C(4) 贝塞尔
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Bezier),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 20.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 段 [A,B]：A 贝塞尔 → 用 A.handle_out + B.handle_in；[B,C]：B 线性 → 无贝塞尔柄
    // 所以只有 A.out 与 B.in 两个手柄
    assert_eq!(st.handles.row_count(), 2);
    let h0 = st.handles.row_data(0).unwrap();
    assert_eq!((h0.keyframe_index, h0.is_in), (0, false)); // A.out
    let h1 = st.handles.row_data(1).unwrap();
    assert_eq!((h1.keyframe_index, h1.is_in), (1, true)); // B.in
}

#[test]
fn pressing_controls_activates_channel() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    assert_eq!(st.current_channel, -1);
    let tags = st.channel_pressed(1, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert_eq!(st.current_channel, 1);
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert_eq!(st.current_channel, 0);
    let tags = st.deselect_channel();
    project(&mut st, tags);
    assert_eq!(st.current_channel, -1);
}

#[test]
fn display_selection_updates_at_press_without_touching_channels_model() {
    use slint::Model as _;

    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.sync_selected_display();
    assert_eq!(st.selected_display.row_count(), 2);
    assert_eq!(st.selected_display.row_data(0), Some(false));
    assert_eq!(st.selected_display.row_data(1), Some(false));

    // 按下通道 a 的关键帧：显示掩码立即更新（曲线高亮），
    // 但 channels 外层模型的 selected **不**变（拖拽中更新会破坏 grab）。
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert_eq!(st.selected_display.row_data(0), Some(true), "按下即高亮 a");
    assert_eq!(st.selected_display.row_data(1), Some(false));
    assert_eq!(st.channels.row_data(0).unwrap().selected, false, "channels.selected 应延迟写回");

    // Shift 追加 b：两个都高亮（显示掩码即时反映多选）
    let tags = st.keyframe_pressed(1, 0, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    assert_eq!(st.selected_display.row_data(0), Some(true));
    assert_eq!(st.selected_display.row_data(1), Some(true));
    assert_eq!(st.channels.row_data(1).unwrap().selected, false);

    // 松开（拖拽过，moved=true 保持多选）：channels.selected 才写回，显示掩码保持一致
    let tags = st.keyframe_released(1, 0, true);
    project(&mut st, tags);
    assert_eq!(st.channels.row_data(0).unwrap().selected, true);
    assert_eq!(st.channels.row_data(1).unwrap().selected, true);
    assert_eq!(st.selected_display.row_data(0), Some(true));
    assert_eq!(st.selected_display.row_data(1), Some(true));

    // 空白清除：显示掩码立即清空
    let tags = st.deselect_channel();
    project(&mut st, tags);
    assert_eq!(st.selected_display.row_data(0), Some(false));
    assert_eq!(st.selected_display.row_data(1), Some(false));
}

#[test]
fn channel_plain_click_selects_only_one() {
    let mut st = CurveEditorState::default();
    for name in ["a", "b", "c", "d"] {
        let tags = st.push_channel(
            name,
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear)],
        );
        project(&mut st, tags);
    }
    let tags = st.channel_pressed(2, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(st.channels.row_data(2).unwrap().selected);
    for i in [0, 1, 3] {
        assert!(!st.channels.row_data(i).unwrap().selected, "channel {i} 不应被选中");
    }
    assert_eq!(st.selected_channel_count(), 1);
    // 再点另一个：普通点击只选它
    let tags = st.channel_pressed(0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(st.channels.row_data(0).unwrap().selected);
    assert!(!st.channels.row_data(2).unwrap().selected);
    assert_eq!(st.current_channel, 0);
}

#[test]
fn channel_shift_selects_range_from_anchor() {
    let mut st = CurveEditorState::default();
    for name in ["a", "b", "c", "d", "e"] {
        let tags = st.push_channel(
            name,
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear)],
        );
        project(&mut st, tags);
    }
    // 锚点 = 最近普通点击的通道（1）
    let tags = st.channel_pressed(1, SlintSelectionModifier::None);
    project(&mut st, tags);
    // Shift 点 4 → 选中 1..=4
    let tags = st.channel_pressed(4, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    for i in 1..=4 {
        assert!(st.channels.row_data(i).unwrap().selected, "channel {i} 应在区间内");
    }
    assert!(!st.channels.row_data(0).unwrap().selected);
    assert_eq!(st.selected_channel_count(), 4);
    assert_eq!(st.current_channel, 4);
    // 再 Shift 点 0（反向区间 0..=4 仍全选，锚点更新为 0）
    let tags = st.channel_pressed(0, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    for i in 0..=4 {
        assert!(st.channels.row_data(i).unwrap().selected, "channel {i} 应在区间内");
    }
}

#[test]
fn channel_ctrl_toggles_selection() {
    let mut st = CurveEditorState::default();
    for name in ["a", "b", "c"] {
        let tags = st.push_channel(
            name,
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear)],
        );
        project(&mut st, tags);
    }
    let tags = st.channel_pressed(0, SlintSelectionModifier::None);
    project(&mut st, tags);
    // Ctrl 逐个追加
    let tags = st.channel_pressed(1, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    let tags = st.channel_pressed(2, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    for i in 0..=2 {
        assert!(st.channels.row_data(i).unwrap().selected);
    }
    // Ctrl 再点已选中的 → 取消
    let tags = st.channel_pressed(1, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    assert!(!st.channels.row_data(1).unwrap().selected);
    assert!(st.channels.row_data(0).unwrap().selected);
    assert!(st.channels.row_data(2).unwrap().selected);
    // Ctrl 取消后 current_channel 指向最后仍选中的（这里保持最后一次有效选中）
    assert_eq!(st.current_channel, 2);
    assert_eq!(st.selected_channel_count(), 2);
}

#[test]
fn deselect_channel_clears_all_selection() {
    let mut st = CurveEditorState::default();
    for name in ["a", "b", "c"] {
        let tags = st.push_channel(
            name,
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear)],
        );
        project(&mut st, tags);
    }
    let tags = st.channel_pressed(0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.channel_pressed(1, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    let tags = st.deselect_channel();
    project(&mut st, tags);
    assert_eq!(st.selected_channel_count(), 0);
    assert_eq!(st.current_channel, -1);
}

#[test]
fn curve_color_mode_switches_auto_rainbow_and_user() {
    let mut st = CurveEditorState::default();
    let user = Color::from_rgb_u8(12, 34, 56);
    let tags = st.push_channel(
        "t",
        user,
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(4.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 默认自动彩虹：显示色 = 调色板按下标取，用户色被保存
    let ch = st.channels.row_data(0).unwrap();
    assert_eq!(ch.color_mode, SlintColorMode::AutoRainbow);
    assert_eq!(ch.user_color, user);
    assert_ne!(ch.color, user);
    assert_eq!(ch.color, CurveEditorState::rainbow_color(0));

    // 切到用户设置：显示色恢复为用户色
    let tags = st.set_curve_color_mode(0, SlintColorMode::User);
    project(&mut st, tags);
    let ch = st.channels.row_data(0).unwrap();
    assert_eq!(ch.color_mode, SlintColorMode::User);
    assert_eq!(ch.color, user);
    // 段颜色同步更新
    let seg = st.segments.row_data(0).unwrap();
    assert_eq!(seg.color, user);

    // 切回自动彩虹：回到调色板色
    let tags = st.set_curve_color_mode(0, SlintColorMode::AutoRainbow);
    project(&mut st, tags);
    let ch = st.channels.row_data(0).unwrap();
    assert_eq!(ch.color_mode, SlintColorMode::AutoRainbow);
    assert_eq!(ch.color, CurveEditorState::rainbow_color(0));
}

#[test]
fn selected_keyframes_interpolation_change() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(4.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "u",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(2.0, 5.0, SlintInterpolation::Hold)],
    );
    project(&mut st, tags);
    st.rebuild_all();

    assert_eq!(st.selected_keyframe_count(), 0);
    assert_eq!(st.selected_interpolation_state(), SlintInterpolationState::None);

    // 无选中时修改无效
    let tags = st.set_selected_keyframes_interpolation(SlintInterpolation::Bezier);
    project(&mut st, tags);
    assert_eq!(st.selected_keyframe_count(), 0);

    // 选中通道 0 的两个关键帧
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    assert_eq!(st.selected_keyframe_count(), 2);
    assert_eq!(st.selected_interpolation_state(), SlintInterpolationState::Linear);

    // 改成贝塞尔：全部选中关键帧生效，未选中的不受影响
    let tags = st.set_selected_keyframes_interpolation(SlintInterpolation::Bezier);
    project(&mut st, tags);
    let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
    let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
    let kf2 = st.keyframe_models[&1].row_data(0).unwrap();
    assert_eq!(kf0.interpolation, SlintInterpolation::Bezier);
    assert_eq!(kf1.interpolation, SlintInterpolation::Bezier);
    assert_eq!(kf2.interpolation, SlintInterpolation::Hold);
    assert_eq!(st.selected_interpolation_state(), SlintInterpolationState::Bezier);

    // 改成常量（hold）
    let tags = st.set_selected_keyframes_interpolation(SlintInterpolation::Hold);
    project(&mut st, tags);
    assert_eq!(
        st.keyframe_models[&0].row_data(0).unwrap().interpolation,
        SlintInterpolation::Hold
    );
    assert_eq!(st.selected_interpolation_state(), SlintInterpolationState::Hold);
}

#[test]
fn linear_to_bezier_creates_out_handle_with_default_offset() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(4.0, 10.0, SlintInterpolation::Linear),
            key(8.0, 20.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();
    assert_eq!(st.handles.row_count(), 0); // 全线性 → 无控制柄

    // 选中中间关键帧 → 切贝塞尔
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.set_selected_keyframes_interpolation(SlintInterpolation::Bezier);
    project(&mut st, tags);

    // 中间关键帧出柄（右段变贝塞尔）+ 末关键帧入柄（其左段变贝塞尔）；首关键帧（仍线性）无柄
    assert_eq!(st.handles.row_count(), 2);
    let h_out = st.handles.row_data(0).unwrap();
    assert!(!h_out.is_in);
    assert_eq!(h_out.keyframe_index, 1);
    // 默认外伸 1/2 间距（4→8 为 4，外伸 4/2 = 2.0）
    assert!((h_out.handle_time - (4.0 + 4.0 / 2.0)).abs() < 0.001);
    assert!((h_out.handle_value - 10.0).abs() < 0.001);

    let h_in = st.handles.row_data(1).unwrap();
    assert!(h_in.is_in);
    assert_eq!(h_in.keyframe_index, 2);

    // 出柄在模型中的值同步更新
    let kf = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf.interpolation, SlintInterpolation::Bezier);
    assert!((kf.handle_out.time_offset - 4.0 / 2.0).abs() < 0.001);

    // 右侧段应为贝塞尔，且控制点不再与端点重合（曲线呈弧形）
    let seg = st.segments.row_data(1).unwrap();
    assert_eq!(seg.kind, SlintSegmentKind::Bezier);
    assert!(seg.cx1 > seg.x0);
    assert!(seg.cx1 < seg.x1);
}

#[test]
fn dope_marquee_select_picks_keyframes_in_box() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 5.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 15.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![
            key(1.0, 5.0, SlintInterpolation::Linear),
            key(3.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "c",
        Color::from_rgb_u8(0, 0, 255),
        vec![key(0.0, 5.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 框选时间 [0, 2.5] × 行 [0,1]：
    // 行 0 (a)：time 0、2 命中，4 不命中；行 1 (b)：time 1 命中、3 不命中；行 2 (c) 不在框内
    let tags = st.dope_marquee_select(0.0, 2.5, 0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);

    let a0 = st.keyframe_models[&0].row_data(0).unwrap().selected;
    let a1 = st.keyframe_models[&0].row_data(1).unwrap().selected;
    let a2 = st.keyframe_models[&0].row_data(2).unwrap().selected;
    let b0 = st.keyframe_models[&1].row_data(0).unwrap().selected;
    let b1 = st.keyframe_models[&1].row_data(1).unwrap().selected;
    let c0 = st.keyframe_models[&2].row_data(0).unwrap().selected;
    assert_eq!((a0, a1, a2), (true, true, false));
    assert_eq!((b0, b1), (true, false));
    assert!(!c0, "框外的行不应被选中");
    // 通道选中：只命中行 0、1
    assert_eq!(st.selected_channel_count(), 2);
    assert!(st.channel_selected(0) && st.channel_selected(1) && !st.channel_selected(2));
    assert_eq!(st.current_channel, 0);
}

#[test]
fn dope_marquee_select_additive_keeps_existing() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 5.0, SlintInterpolation::Linear), key(2.0, 10.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(1.0, 5.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 先替换选 a0；再用 Shift(mode=1) 追加选 b0
    let tags = st.dope_marquee_select(0.0, 0.5, 0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.dope_marquee_select(1.0, 1.5, 1, 1, SlintSelectionModifier::Shift);
    project(&mut st, tags);

    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(st.keyframe_models[&1].row_data(0).unwrap().selected);
    assert_eq!(st.selected_channel_count(), 2);
}

#[test]
fn dope_marquee_select_ignores_invisible_channels() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 5.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(0.0, 5.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // 关掉 b 的眼睛（数据层 + 投影）
    st.channels_data[1].visible = false;
    let tags = vec![Tag::AnimationChannelChanged { curve: 1 }];
    project(&mut st, tags);

    let tags = st.dope_marquee_select(-1.0, 10.0, 0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(!st.keyframe_models[&1].row_data(0).unwrap().selected, "不可见通道的关键帧不应被框选");
    assert_eq!(st.selected_channel_count(), 1);
}

#[test]
fn keyframe_drag_moves_all_selected_keyframes() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![
            key(1.0, 5.0, SlintInterpolation::Linear),
            key(3.0, 15.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 跨通道选中 a0、a1、b0，拖 a0：三个都移动，未选中的 b1 不动
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(1, 0, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 0, 0.5, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);

    assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 0.5);
    assert_eq!(st.keyframe_models[&0].row_data(1).unwrap().time, 2.5);
    assert_eq!(st.keyframe_models[&1].row_data(0).unwrap().time, 1.5);
    assert_eq!(st.keyframe_models[&1].row_data(1).unwrap().time, 3.0, "未选中的关键帧不应移动");
    // 值不变（dope 拖拽 dv=0 路径）
    assert_eq!(st.keyframe_models[&1].row_data(0).unwrap().value, 5.0);
}

#[test]
fn keyframe_drag_skips_locked_channels() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(1.0, 5.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // b 上锁：它的选中关键帧不随 a 的拖拽移动
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let mut b = st.channels.row_data(1).unwrap();
    b.editable = false;
    st.channels.set_row_data(1, b);
    // 直接给 b0 打上选中（跨通道模拟）
    let mut kf = st.keyframe_models[&1].row_data(0).unwrap();
    kf.selected = true;
    st.keyframe_models[&1].set_row_data(0, kf);

    let tags = st.keyframe_dragged(0, 0, 1.0, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 1.0);
    assert_eq!(st.keyframe_models[&1].row_data(0).unwrap().time, 1.0, "锁定通道的关键帧不应移动");
}

#[test]
fn keyframe_press_on_selected_keeps_multi_selection() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 多选 a0、a1，然后普通点击已选中的 a0 再拖：多选保持，两个一起移动
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    let tags = st.keyframe_released(0, 1, true);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);

    st.keyframe_pressed(0, 0, SlintSelectionModifier::None); // 点已选中的 a0，不应清空选择
    assert!(st.keyframe_models[&0].row_data(1).unwrap().selected, "点已选中的关键帧不应清空多选");
    let tags = st.keyframe_dragged(0, 0, 0.5, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 0.5);
    assert_eq!(st.keyframe_models[&0].row_data(1).unwrap().time, 2.5, "多选中被拖的关键帧外的其它选中关键帧也应移动");
}

#[test]
fn shift_press_adds_never_deselects_grabbed() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // Shift 连续追加 a0、a1；再 Shift 点 a0（已选中）→ 仍保持选中（不会切掉）
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);

    // Shift 点过的被拖关键帧保持选中 → 拖拽不会反馈放大（其余选中关键帧按正常速度移动）
    let tags = st.keyframe_dragged(0, 0, 0.5, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 0.5);
    assert_eq!(st.keyframe_models[&0].row_data(1).unwrap().time, 2.5);
}

#[test]
fn graph_marquee_select_picks_keyframes_and_handles() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 5.0 };
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            k0,
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 20.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 框选时间 [0,1.5] × 值 [0,10]：A 关键帧中心 (0,0) 命中，A 出柄控制点 (1,5) 命中；
    // B(2,10) 与 C(4,20) 不在框内
    let tags = st.graph_marquee_select(0.0, 1.5, 0.0, 10.0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(!st.keyframe_models[&0].row_data(1).unwrap().selected);
    assert!(!st.keyframe_models[&0].row_data(2).unwrap().selected);
    assert!(st.selected_handles.contains(&HandleLoc { curve: 0, key: 0, is_in: false }), "A 的出柄应被选中");
    assert!(!st.selected_handles.contains(&HandleLoc { curve: 0, key: 2, is_in: false }), "C 的出柄不应被选中");

    // selected 反映到 Slint 手柄模型
    let h = (0..st.handles.row_count())
        .filter_map(|i| st.handles.row_data(i))
        .find(|h| h.keyframe_index == 0 && !h.is_in)
        .expect("A 出柄应在模型中");
    assert!(h.selected);
}

#[test]
fn graph_marquee_select_additive_keeps_existing() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    let tags = st.graph_marquee_select(0.0, 0.5, -1.0, 1.0, SlintSelectionModifier::None); // 只选 a0
    project(&mut st, tags);
    let tags = st.graph_marquee_select(2.0, 2.5, 9.0, 11.0, SlintSelectionModifier::Shift); // Shift 追加 a1
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);
    assert_eq!(st.selected_channel_count(), 1);
}

#[test]
fn multi_handle_drag_same_screen_offset() {
    let mut st = CurveEditorState::default();
    // 两个贝塞尔关键帧，各自有出柄
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 }; // (1,0)
    let mut k1 = key(2.0, 0.0, SlintInterpolation::Bezier);
    k1.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 1.0 }; // (1,1)
    let tags = st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
    project(&mut st, tags);
    st.rebuild_all();

    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_pressed(0, 1, false, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    assert!(st.selected_handles.contains(&HandleLoc { curve: 0, key: 0, is_in: false }) && st.selected_handles.contains(&HandleLoc { curve: 0, key: 1, is_in: false }));

    // 拖 A 出柄 +1s,+1 值：A (1,0)→(2,1)，B 相同屏幕偏移 (1,1)→(2,2)
    let tags = st.handle_dragged(0, 0, false, 1.0, 1.0, SlintHandleDragMode::Free);
    project(&mut st, tags);
    let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
    assert!((kf0.handle_out.time_offset - 2.0).abs() < 1e-3);
    assert!((kf0.handle_out.value_offset - 1.0).abs() < 1e-3);
    let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
    assert!(
        (kf1.handle_out.time_offset - 2.0).abs() < 1e-3
            && (kf1.handle_out.value_offset - 2.0).abs() < 1e-3,
        "B 出柄应保持相同屏幕偏移，got ({}, {}) expected (2, 2)",
        kf1.handle_out.time_offset,
        kf1.handle_out.value_offset
    );
}

#[test]
fn multi_handle_drag_clamps_and_previews_synced_handles() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
    let mut k1 = key(2.0, 0.0, SlintInterpolation::Bezier);
    k1.handle_out = SlintBezierHandle { time_offset: -1.0, value_offset: 1.0 }; // 出柄时间偏移为负（会被硬钳到 ≥0）
    let tags = st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
    project(&mut st, tags);
    st.rebuild_all();

    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_pressed(0, 1, false, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    // 拖 A 向右 +2s：A (1,0)→(3,0)。B 同偏移 (-1,1)→(1,1)，再被硬钳制到 (≥0)：已 ≥0
    // 若把 B 拖向负方向（dt<0）则应被钳到 0
    let tags = st.handle_dragged(0, 0, false, 2.0, 0.0, SlintHandleDragMode::Free);
    project(&mut st, tags);
    st.handle_dragged(0, 0, false, -4.0, 0.0, SlintHandleDragMode::Free); // 累计 A：(3,0)→(-1,0)→硬钳到 (0,0)
    let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
    assert!(kf0.handle_out.time_offset >= 0.0, "主手柄出柄应硬钳到 ≥0，got {}", kf0.handle_out.time_offset);
    let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
    // B 跟随 A：(-1,1)→(1,1)→(-3,1)→硬钳到 (0,1)
    assert!(
        kf1.handle_out.time_offset >= 0.0,
        "非主手柄出柄也应硬钳到 ≥0，got {}",
        kf1.handle_out.time_offset
    );
    assert!((kf1.handle_out.value_offset - 1.0).abs() < 1e-3);
}

#[test]
fn click_on_selected_keyframe_collapses_to_single() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 多选 a0、a1（拖拽结束 moved=true 保持多选）
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    let tags = st.keyframe_released(0, 1, true);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);

    // 单击 a0（未拖动 moved=false）→ 折叠为单选
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_released(0, 0, false);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(!st.keyframe_models[&0].row_data(1).unwrap().selected, "单击已选中的关键帧应折叠为单选");
}

#[test]
fn click_on_selected_handle_collapses_to_single() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
    let mut k1 = key(2.0, 10.0, SlintInterpolation::Bezier);
    k1.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 1.0 };
    let tags = st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
    project(&mut st, tags);
    st.rebuild_all();

    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_pressed(0, 1, false, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    assert_eq!(st.selected_handles.len(), 2);

    // 单击 a0 出柄（未拖动）→ 折叠为单选该手柄
    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_released(0, 0, false, false);
    project(&mut st, tags);
    assert_eq!(st.selected_handles.len(), 1);
    assert!(st.selected_handles.contains(&HandleLoc { curve: 0, key: 0, is_in: false }));
    assert!(!st.selected_handles.contains(&HandleLoc { curve: 0, key: 1, is_in: false }));
}

#[test]
fn shift_drag_pulls_out_handle_on_bezier_keyframe() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Bezier),
            key(2.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // Shift+拖动 a0：关键帧元素不动，上报的 dt/dv 为光标相对关键帧的累计偏移，
    // 直接作为手柄偏移（不再累加），且出柄/入柄原点对称
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 0, 1.0, 2.0, SlintKeyframeDragMode::DragOutHandle);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 0, 2.0, 3.0, SlintKeyframeDragMode::DragOutHandle); // 累计光标位置，直接覆盖
    project(&mut st, tags);
    let kf = st.keyframe_models[&0].row_data(0).unwrap();
    assert_eq!(kf.handle_out.time_offset, 2.0);
    assert_eq!(kf.handle_out.value_offset, 3.0);
    assert_eq!(kf.handle_in.time_offset, -2.0, "入柄应与出柄原点对称");
    assert_eq!(kf.handle_in.value_offset, -3.0);
    assert_eq!(kf.time, 0.0, "关键帧本体不应移动");
    assert_eq!(kf.value, 0.0);

    // 非贝塞尔关键帧不拖出手柄
    let tags = st.keyframe_pressed(0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 1, 1.0, 1.0, SlintKeyframeDragMode::DragOutHandle);
    project(&mut st, tags);
    let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf1.handle_out.time_offset, 0.0);
    assert_eq!(kf1.handle_in.time_offset, 0.0);
}

#[test]
fn multi_handle_drag_applies_mode_to_synced_handles() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
    k0.handle_in = SlintBezierHandle { time_offset: -1.0, value_offset: 0.0 };
    let mut k1 = key(2.0, 0.0, SlintInterpolation::Bezier);
    k1.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
    k1.handle_in = SlintBezierHandle { time_offset: -1.0, value_offset: 0.0 };
    let tags = st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
    project(&mut st, tags);
    st.rebuild_all();

    // 多选两个出柄，用 mode 2（原点对称）拖 A 出柄 +1,+1
    let tags = st.handle_pressed(0, 0, false, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_pressed(0, 1, false, SlintSelectionModifier::Shift);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 0, false, 1.0, 1.0, SlintHandleDragMode::Symmetric);
    project(&mut st, tags);

    // 主手柄 A 出柄 (1,0)→(2,1)，其入柄原点对称 = -(2,1)
    let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
    assert!((kf0.handle_out.time_offset - 2.0).abs() < 1e-3);
    assert!((kf0.handle_in.time_offset + 2.0).abs() < 1e-3, "主手柄入柄应原点对称");
    assert!((kf0.handle_in.value_offset + 1.0).abs() < 1e-3);

    // 被同步的 B 出柄同样 +1,+1 → (2,1)，其入柄同样原点对称 = -(2,1)（模式一致）
    let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
    assert!((kf1.handle_out.time_offset - 2.0).abs() < 1e-3);
    assert!((kf1.handle_in.time_offset + 2.0).abs() < 1e-3, "被同步手柄的对手柄也应原点对称（模式一致）");
    assert!((kf1.handle_in.value_offset + 1.0).abs() < 1e-3);
}

#[test]
fn click_cycles_through_coincident_keyframes() {
    let mut st = CurveEditorState::default();
    // 三个通道在相同位置 (1, 5) 有几乎重合的关键帧
    let tags = st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![key(1.0, 5.0, SlintInterpolation::Linear)]);
    project(&mut st, tags);
    let tags = st.push_channel("b", Color::from_rgb_u8(0, 255, 0), vec![key(1.0, 5.0, SlintInterpolation::Linear)]);
    project(&mut st, tags);
    let tags = st.push_channel("c", Color::from_rgb_u8(0, 0, 255), vec![key(1.0, 5.0, SlintInterpolation::Linear)]);
    project(&mut st, tags);
    st.rebuild_all();

    // 第一次点击 a0（未选中）→ 单选 a0
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_released(0, 0, false);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(!st.keyframe_models[&1].row_data(0).unwrap().selected);

    // 第二次点击该位置（已有选中的重合点）→ 循环到 b0
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(st.pressed_was_selected, "该位置已有选中的重合点");
    let tags = st.keyframe_released(0, 0, false);
    project(&mut st, tags);
    assert!(!st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(st.keyframe_models[&1].row_data(0).unwrap().selected);

    // 第三次 → c0
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_released(0, 0, false);
    project(&mut st, tags);
    assert!(st.keyframe_models[&2].row_data(0).unwrap().selected);

    // 第四次 → 回到 a0
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_released(0, 0, false);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
}

#[test]
fn channel_search_filters_per_editor_independently() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel("Position X", Color::from_rgb_u8(255, 0, 0), vec![key(0.0, 0.0, SlintInterpolation::Linear)]);
    project(&mut st, tags);
    let tags = st.push_channel("Rotation Y", Color::from_rgb_u8(0, 255, 0), vec![key(0.0, 0.0, SlintInterpolation::Linear)]);
    project(&mut st, tags);
    st.rebuild_all();
    assert_eq!(st.graph_filter.row_count(), 2);
    assert_eq!(st.dope_filter.row_count(), 2);

    // 曲线编辑器搜索只影响自己的过滤视图
    let tags = st.set_graph_search("pos"); // 忽略大小写子串匹配
    project(&mut st, tags);
    assert_eq!(st.graph_filter.row_count(), 1, "曲线编辑器过滤只影响自己");
    assert_eq!(st.dope_filter.row_count(), 2, "摄影表不受曲线编辑器搜索影响");
    assert_eq!(st.graph_filter.row_data(0).unwrap().curve_index, 0, "过滤视图元素保留真实索引");
    assert_eq!(st.graph_filter.row_data(0).unwrap().name.as_str(), "Position X");

    // 摄影表搜索独立
    let tags = st.set_dope_search("rot");
    project(&mut st, tags);
    assert_eq!(st.graph_filter.row_count(), 1);
    assert_eq!(st.dope_filter.row_count(), 1);
    assert_eq!(st.dope_filter.row_data(0).unwrap().curve_index, 1);

    // 清空恢复全部
    let tags = st.set_graph_search("");
    project(&mut st, tags);
    let tags = st.set_dope_search("");
    project(&mut st, tags);
    assert_eq!(st.graph_filter.row_count(), 2);
    assert_eq!(st.dope_filter.row_count(), 2);
}

#[test]
fn graph_segments_and_handles_filtered_by_search() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 1.0 };
    let tags = st.push_channel(
        "Position X",
        Color::from_rgb_u8(255, 0, 0),
        vec![k0, key(2.0, 5.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "Rotation Y",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear), key(2.0, 5.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // 两个通道都有段；只有 Position X 有贝塞尔出柄
    assert_eq!(st.segments.row_count(), 2);
    assert_eq!(st.graph_segments.row_count(), 2);

    let tags = st.set_graph_search("pos");
    project(&mut st, tags);
    // 段过滤到只含 Position X；手柄 = Position X 的贝塞尔段两端各一个（出柄+入柄）
    let segs: Vec<i32> = (0..st.graph_segments.row_count())
        .filter_map(|i| st.graph_segments.row_data(i).map(|s| s.curve_index))
        .collect();
    let hdls: Vec<i32> = (0..st.graph_handles.row_count())
        .filter_map(|i| st.graph_handles.row_data(i).map(|h| h.curve_index))
        .collect();
    assert_eq!(segs, vec![0]);
    assert_eq!(hdls, vec![0, 0], "手柄都应来自 Position X");
    assert_eq!(st.graph_handles.row_count(), 2);

    let tags = st.set_graph_search("");
    project(&mut st, tags);
    assert_eq!(st.graph_segments.row_count(), 2);
    assert_eq!(st.graph_handles.row_count(), 2);
}

#[test]
fn dope_connections_cover_linear_and_hold_segments() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Hold),
            key(4.0, 10.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // 段由左侧关键帧插值决定：[0,2] 线性、[2,4] 常量；最后一个贝塞尔无右侧段
    assert_eq!(st.dope_connections.row_count(), 2);
    let c0 = st.dope_connections.row_data(0).unwrap();
    assert_eq!((c0.x0, c0.x1), (0.0, 2.0));
    assert!(!c0.is_hold, "线性段不标记 hold");
    assert!(c0.visible);
    let c1 = st.dope_connections.row_data(1).unwrap();
    assert_eq!((c1.x0, c1.x1), (2.0, 4.0));
    assert!(c1.is_hold, "常量段应标记 hold");
}

#[test]
fn dope_connections_skip_bezier_and_follow_visibility() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Bezier),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 10.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // [0,2] 贝塞尔（左侧关键帧 Bezier）→ 无连接；[2,4] 线性 → 连接
    assert_eq!(st.dope_connections.row_count(), 1);
    let c0 = st.dope_connections.row_data(0).unwrap();
    assert_eq!((c0.x0, c0.x1), (2.0, 4.0));

    // 关掉眼睛 → 连接 visible=false（数据层 + 投影）
    st.channels_data[0].visible = false;
    let tags = vec![Tag::AnimationChannelChanged { curve: 0 }];
    project(&mut st, tags);
    let tags = st.toggle_channel_visible(0); // 先恢复（false→true）
    project(&mut st, tags);
    let tags = st.toggle_channel_visible(0); // 再关掉（true→false）
    project(&mut st, tags);
    assert!(!st.dope_connections.row_data(0).unwrap().visible, "眼睛关闭后连接应隐藏");
}

#[test]
fn keyframe_times_snap_to_integer_ms() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.1234, 1.0, SlintInterpolation::Linear),
            key(2.9996, 2.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();
    let m = st.keyframe_models.get(&0).unwrap();
    let k0 = m.row_data(0).unwrap();
    let k1 = m.row_data(1).unwrap();
    // 0.1234 → 123ms → 0.123；2.9996 → 3000ms → 3.0
    assert!((k0.time - 0.123).abs() < 1e-6, "应吸附到 123ms，得 {}", k0.time);
    assert!((k1.time - 3.0).abs() < 1e-6, "应吸附到 3000ms，得 {}", k1.time);
}

#[test]
fn summary_aggregates_coincident_keyframes_by_ms() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 1.0, SlintInterpolation::Linear),
            key(2.0, 2.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![
            key(0.0, 3.0, SlintInterpolation::Linear), // 与 a 同时刻
            key(2.5, 4.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // 时间 0ms（两条）、2000ms（一条）、2500ms（一条）
    let mut times: Vec<f32> = (0..st.summary_keyframes.row_count())
        .map(|i| st.summary_keyframes.row_data(i).unwrap().time)
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(times.len(), 3);
    assert_eq!(st.summary_keyframes.row_data(0).unwrap().count, 2, "同一毫秒应合并计数");
    assert!((times[0] - 0.0).abs() < 1e-6);
    assert!((times[1] - 2.0).abs() < 1e-6);
    assert!((times[2] - 2.5).abs() < 1e-6);
}

#[test]
fn reset_action_clears_current_action() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 1.0, SlintInterpolation::Linear),
            key(2.0, 3.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();
    assert!(st.action_start < st.action_end, "应有非零时长范围");
    assert_eq!(st.summary_keyframes.row_count(), 2);

    // 下插到 NLA 后重置全局 Action：通道/关键帧清空、名称清空、时长归零
    let tags = st.reset_action();
    project(&mut st, tags);
    assert_eq!(st.channels.row_count(), 0);
    assert_eq!(st.summary_keyframes.row_count(), 0);
    assert_eq!((st.action_start, st.action_end), (0.0, 0.0));
    assert!(st.action_name.is_empty(), "重置后名称清空，UI 显示<无动作>");
    assert_eq!(st.current_action, -1);
}

#[test]
fn summary_selected_reflects_any_underlying() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 1.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(0.0, 3.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // 初始无选中（逐通道掩码全 false）
    assert!(!st.summary_keyframes.row_data(0).unwrap().selected.iter().any(|s| s));

    // 普通点击汇总关键帧：按下立即高亮（掩码全 true），选中该时刻全部（两通道）
    let tags = st.summary_keyframe_pressed(0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(
        st.summary_keyframes.row_data(0).unwrap().selected.iter().all(|s| s),
        "按下应立即高亮汇总标记，不用等松开"
    );
    assert!(st.channels.row_data(0).unwrap().selected, "按下应立即亮起关联轨道");
    assert!(st.channels.row_data(1).unwrap().selected, "按下应立即亮起全部关联轨道");
    assert!(st.keyframe_models.get(&0).unwrap().row_data(0).unwrap().selected);
    assert!(st.keyframe_models.get(&1).unwrap().row_data(0).unwrap().selected);
    assert!(
        st.summary_keyframes.row_data(0).unwrap().selected.iter().all(|s| s),
        "按下后仍全部高亮"
    );

    // 拖拽：两通道同时刻的关键帧一起移动
    let tags = st.summary_keyframe_dragged(0, 1.0);
    project(&mut st, tags);
    let k0 = st.keyframe_models.get(&0).unwrap().row_data(0).unwrap();
    let k1 = st.keyframe_models.get(&1).unwrap().row_data(0).unwrap();
    assert!((k0.time - 1.0).abs() < 1e-6);
    assert!((k1.time - 1.0).abs() < 1e-6);
}

#[test]
fn summary_drag_snaps_to_integer_ms() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 1.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();
    let tags = st.summary_keyframe_pressed(0, SlintSelectionModifier::None);
    project(&mut st, tags);
    // 拖 1.2345 秒 → 应吸附到 1235ms
    let tags = st.summary_keyframe_dragged(0, 1.2345);
    project(&mut st, tags);
    let k0 = st.keyframe_models.get(&0).unwrap().row_data(0).unwrap();
    assert!((k0.time - 1.235).abs() < 1e-6, "拖拽应吸附整数毫秒，得 {}", k0.time);
}

#[test]
fn summary_drag_moves_whole_group_even_if_partially_selected() {
    // 回归：先单独选中某个通道的关键帧，再去拖它上面的汇总帧——
    // 应移动该时刻的全部关键帧（而不是只移动已选中的那个）。
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 1.0, SlintInterpolation::Linear),
            key(4.0, 2.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(0.0, 3.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.rebuild_all();
    // 先只选中通道 a 的 t=0 关键帧（模拟用户先点它）
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_released(0, 0, false);
    project(&mut st, tags);
    assert!(st.keyframe_models.get(&0).unwrap().row_data(0).unwrap().selected);
    assert!(!st.keyframe_models.get(&1).unwrap().row_data(0).unwrap().selected);

    // 按下汇总帧（t=0）并拖 1 秒：应跨通道移动 a、b 全部同时刻关键帧
    let tags = st.summary_keyframe_pressed(0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.summary_keyframe_dragged(0, 1.0);
    project(&mut st, tags);
    let a0 = st.keyframe_models.get(&0).unwrap().row_data(0).unwrap();
    let b0 = st.keyframe_models.get(&1).unwrap().row_data(0).unwrap();
    assert!((a0.time - 1.0).abs() < 1e-6, "a 通道 t=0 关键帧应随汇总帧移到 1s，得 {}", a0.time);
    assert!((b0.time - 1.0).abs() < 1e-6, "b 通道同时刻关键帧也应一起移动，得 {}", b0.time);
}

#[test]
fn marquee_selects_channel_when_box_covers_curve() {
    // 对齐 Blender：框选矩形覆盖到曲线本身（即使没有关键帧在框内）也应选中该通道。
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "c",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(4.0, 0.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 框住曲线中段（x∈[1,3], y∈[-1,1]），关键帧在框外 → 仅曲线命中，通道被选中
    let tags = st.graph_marquee_select(1.0, 3.0, -1.0, 1.0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(
        st.channels.row_data(0).unwrap().selected,
        "框住曲线本身应选中该通道"
    );
}

#[test]
fn marquee_selects_channel_when_curve_bends_into_box() {
    // 贝塞尔曲线弯曲进框内（框内无关键帧）也应选中通道。
    let mut st = CurveEditorState::default();
    // 曲线 (0,0)→(4,0) 凸到 y≈1.73；框取 y∈[1,2] 只罩住弯起部分
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 2.0, value_offset: 6.0 };
    let mut k1 = key(4.0, 0.0, SlintInterpolation::Bezier);
    k1.handle_in = SlintBezierHandle { time_offset: -2.0, value_offset: -6.0 };
    let tags = st.push_channel("c", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
    project(&mut st, tags);
    st.rebuild_all();

    // 关键帧在 (0,0)/(4,0)，都远离框；曲线在 x∈[1.0,1.3] 处 y≈1.73 落入框内
    let tags = st.graph_marquee_select(1.0, 1.5, 1.5, 2.0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(
        st.channels.row_data(0).unwrap().selected,
        "框住曲线弯起的部分应选中该通道"
    );
}

#[test]
fn marquee_ignores_box_above_below_curve() {
    // 框选矩形完全在曲线上方/下方（没有罩住曲线）→ 不选中该通道
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "c",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(4.0, 0.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    st.rebuild_all();

    // 曲线上方大框（y∈[5,10]，曲线 y=0 完全在下方）→ 不选中
    let tags = st.graph_marquee_select(0.0, 4.0, 5.0, 10.0, SlintSelectionModifier::None);
    project(&mut st, tags);
    assert!(
        !st.channels.row_data(0).unwrap().selected,
        "框在曲线上方不应选中通道"
    );
}

#[test]
fn ctrl_toggle_keyframe_activates_display_mask() {
    use slint::Model as _;

    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "a",
        Color::from_rgb_u8(255, 0, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    let tags = st.push_channel(
        "b",
        Color::from_rgb_u8(0, 255, 0),
        vec![key(0.0, 0.0, SlintInterpolation::Linear)],
    );
    project(&mut st, tags);
    st.sync_selected_display();

    // 初始：全空选
    assert_eq!(st.selected_display.row_data(0), Some(false));
    assert_eq!(st.selected_display.row_data(1), Some(false));
    assert!(!st.keyframe_models[&0].row_data(0).unwrap().selected);

    // Ctrl 点击通道 0 关键帧 0
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected, "关键帧应被选中");
    assert_eq!(st.selected_display.row_data(0), Some(true), "通道 0 曲线应激活");
    assert_eq!(st.selected_display.row_data(1), Some(false), "通道 1 不应激活");

    // Ctrl 点击通道 0 关键帧 0（取消选中）→ 通道应取消激活
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    assert!(!st.keyframe_models[&0].row_data(0).unwrap().selected, "关键帧应取消选中");
    assert_eq!(st.selected_display.row_data(0), Some(false), "通道 0 应取消激活");
    assert_eq!(st.selected_display.row_data(1), Some(false));

    // Ctrl 点击通道 0 关键帧 0 + 通道 1 关键帧 0 → 两个通道都激活
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    let tags = st.keyframe_pressed(1, 0, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert!(st.keyframe_models[&1].row_data(0).unwrap().selected);
    assert_eq!(st.selected_display.row_data(0), Some(true));
    assert_eq!(st.selected_display.row_data(1), Some(true));

    // Ctrl 取消通道 0 → 通道 0 取消激活，通道 1 保持
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    assert!(!st.keyframe_models[&0].row_data(0).unwrap().selected);
    assert_eq!(st.selected_display.row_data(0), Some(false), "通道 0 应取消激活");
    assert_eq!(st.selected_display.row_data(1), Some(true), "通道 1 应保持激活");
}

/// 回归：移动关键帧产生的 preview 钳制（handles 区间替换）在取消选中/松开时换回普通手柄。
#[test]
fn keyframe_drag_preview_restored_on_release() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 5.0 };
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![k0, key(2.0, 10.0, SlintInterpolation::Bezier)],
    );
    project(&mut st, tags);
    // 选中并拖第一个关键帧（出柄越过第二个 → 被软钳制 → handles 区间被 preview 替换）
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 0, 1.5, 8.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    let has_preview = (0..st.handles.row_count())
        .any(|i| st.handles.row_data(i).map(|h| h.preview).unwrap_or(false));
    assert!(has_preview, "拖拽中应产生钳制预览标记");
    // 松开：投影恢复普通手柄
    let tags = st.keyframe_released(0, 0, true);
    project(&mut st, tags);
    let has_preview = (0..st.handles.row_count())
        .any(|i| st.handles.row_data(i).map(|h| h.preview).unwrap_or(false));
    assert!(!has_preview, "松开后 preview 钳制应恢复为普通手柄");
}

/// 回归：Aligned（共线）拖手柄时，对向手柄被约束后越过段边界 → 也显示钳制预览。
#[test]
fn handle_drag_preview_includes_opposite_handle() {
    let mut st = CurveEditorState::default();
    // 中间关键帧：出柄向左长柄 (-6,0)。拖入柄 (-10,0) Aligned → 出柄共线反向 (+6,0)，
    // 控制点 (10,10) 越过段 [4,8] 右边界 → 软钳制预览（入柄自身向左侧也被钳制）
    let mut k1 = key(4.0, 10.0, SlintInterpolation::Bezier);
    k1.handle_out = SlintBezierHandle { time_offset: -6.0, value_offset: 0.0 };
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Bezier),
            k1,
            key(8.0, 0.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    let tags = st.handle_pressed(0, 1, true, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.handle_dragged(0, 1, true, -10.0, 0.0, SlintHandleDragMode::Aligned);
    project(&mut st, tags);
    let found = (0..st.preview_handles.row_count()).any(|i| {
        let m = st.preview_handles.row_data(i).unwrap();
        m.curve_index == 0 && m.keyframe_index == 1 && !m.is_in && m.preview
    });
    assert!(found, "Aligned 模式对向出柄应显示钳制预览");
}

/// 回归：拖第一个关键帧越过第二个 → 松开后重排，下标 0 = 时间最小
/// （Slint 外推线读 keyframes[0]/[len-1]，保序模型下必须恢复有序）。
#[test]
fn drag_first_keyframe_past_neighbor_resorts_on_release() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 20.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    // 拖第一个关键帧到 3.0（越过第二个 2.0）
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 0, 3.0, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    // 拖拽中保序（下标 0 仍是被拖关键帧，时间 3.0——grab 抓手按原下标命中）
    assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 3.0);
    let tags = st.keyframe_released(0, 0, true);
    project(&mut st, tags);
    // 松开后重排：下标 0 = 时间最小
    let kfs = &st.editing_keyframes[&0];
    assert_eq!(kfs[0].time, 2.0);
    assert_eq!(kfs[1].time, 3.0);
    assert_eq!(kfs[2].time, 4.0);
    assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 2.0);
    // 段模型同步重建（首段从 2.0 开始）
    let seg0 = st.segments.row_data(0).unwrap();
    assert_eq!(seg0.x0, 2.0);
}

/// 回归：拖拽改变首/末关键帧时，外推线数据（extrapolations，排序后首末）
/// 在拖拽期间实时更新——不依赖松开后的重排。
#[test]
fn extrapolation_follows_first_last_during_drag() {
    let mut st = CurveEditorState::default();
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            key(0.0, 0.0, SlintInterpolation::Linear),
            key(2.0, 10.0, SlintInterpolation::Linear),
            key(4.0, 20.0, SlintInterpolation::Linear),
        ],
    );
    project(&mut st, tags);
    let ex = st.extrapolations.row_data(0).unwrap();
    assert_eq!(ex.first_time, 0.0);
    assert_eq!(ex.last_time, 4.0);

    // 拖第一个关键帧到 3.0（越过第二个 2.0）：拖拽中（未松开）外推已切换
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 0, 3.0, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    let ex = st.extrapolations.row_data(0).unwrap();
    assert_eq!(ex.first_time, 2.0, "拖拽中左外推应切到新的时间最小关键帧");
    assert_eq!(ex.first_value, 10.0);
    assert_eq!(ex.last_time, 4.0);

    // 松开（重排）后外推保持一致
    let tags = st.keyframe_released(0, 0, true);
    project(&mut st, tags);
    let ex = st.extrapolations.row_data(0).unwrap();
    assert_eq!(ex.first_time, 2.0);
    assert_eq!(ex.last_time, 4.0);
}

/// 回归：Move 拖拽把边缘关键帧变成中间后，新的一侧补默认水平柄
/// （贝塞尔段相邻但偏移为零；边缘延伸是水平的 → 默认柄水平延伸 1/3 间距）。
#[test]
fn edge_keyframe_becomes_interior_gets_default_handle() {
    let mut st = CurveEditorState::default();
    let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
    k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
    let tags = st.push_channel(
        "t",
        Color::from_rgb_u8(255, 0, 0),
        vec![
            k0,
            key(2.0, 10.0, SlintInterpolation::Bezier),
            key(4.0, 20.0, SlintInterpolation::Bezier),
        ],
    );
    project(&mut st, tags);
    // 边缘关键帧 kf0：无入柄
    assert_eq!(st.editing_keyframes[&0][0].handle_in.time_offset, 0.0);

    // 拖 kf0 到 3.0（越过 kf1）→ kf0 变中间：左侧段 [kf1(2), kf0(3)] 贝塞尔
    let tags = st.keyframe_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.keyframe_dragged(0, 0, 3.0, 0.0, SlintKeyframeDragMode::Move);
    project(&mut st, tags);
    // 拖拽中（未松开）：不补柄（移动过程无必要，避免干扰拖拽）
    assert_eq!(st.editing_keyframes[&0][0].handle_in.time_offset, 0.0);

    // 松开 → DragEnded 整理：重排后 [kf1(2), kf0(3), kf2(4)]，kf0 在下标 1
    let tags = st.keyframe_released(0, 0, true);
    project(&mut st, tags);
    // kf0 变中间：新入柄补上——水平向左延伸 1/2 间距
    let kf0 = &st.editing_keyframes[&0][1];
    assert!(
        (kf0.handle_in.time_offset - (-1.0 / 2.0)).abs() < 1e-4,
        "新入柄应水平向左延伸间距/2，实际 {}",
        kf0.handle_in.time_offset
    );
    assert_eq!(kf0.handle_in.value_offset, 0.0);
    // 嵌套模型同步（重排后下标 1）
    let kf0m = st.keyframe_models[&0].row_data(1).unwrap();
    assert_eq!(kf0m.handle_in.time_offset, kf0.handle_in.time_offset);
    // 原 kf1 变为第一（边缘，下标 0）：它自身的出柄（贝塞尔且为零）被补上
    let kf1 = &st.editing_keyframes[&0][0];
    assert!(
        kf1.handle_out.time_offset > 0.0,
        "中间关键帧缺出柄也应补默认水平柄"
    );
}
