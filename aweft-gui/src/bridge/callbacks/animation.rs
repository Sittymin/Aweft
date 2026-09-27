use crate::actions::Action;
use crate::bridge::types::{AnimationCallbacks, MainWindow};

use crate::store::AppStore;
// winit 事件只在桌面后端存在：Android 特性集里 winit 后端被彻底排除
//（见 aweft-gui/Cargo.toml），slint::winit_030 模块在该目标上不存在。
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 设置动画编辑器（曲线编辑器）回调：全部交互走 `Action::Animation` → dispatch → tag → 投影。
pub(super) fn setup_animation_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::actions::animation::{AnimationAction, graph::GraphAction};

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_pressed(move |curve, key, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::KeyframePressed {
                    curve: curve as usize,
                    key: key as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<AnimationCallbacks>().on_keyframe_dragged(
        move |curve, key, delta_time, delta_value, mode| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::KeyframeDragged {
                    curve: curve as usize,
                    key: key as usize,
                    delta_time,
                    delta_value,
                    mode,
                })),
                &weak,
            );
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_released(move |curve, key, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::KeyframeReleased {
                    curve: curve as usize,
                    key: key as usize,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DeselectKeyframes)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<AnimationCallbacks>().on_handle_pressed(
        move |curve, key, is_in, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::HandlePressed {
                    curve: curve as usize,
                    key: key as usize,
                    is_in,
                    modifier,
                })),
                &weak,
            );
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<AnimationCallbacks>().on_handle_dragged(
        move |curve, key, is_in, delta_time, delta_value, mode| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::HandleDragged {
                    curve: curve as usize,
                    key: key as usize,
                    is_in,
                    delta_time,
                    delta_value,
                    mode,
                })),
                &weak,
            );
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_handle_released(move |curve, key, is_in, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::HandleReleased {
                    curve: curve as usize,
                    key: key as usize,
                    is_in,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_graph_marquee_select(move |x_lo, x_hi, y_lo, y_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::GraphMarqueeSelect {
                    x_lo,
                    x_hi,
                    y_lo,
                    y_hi,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_pressed(move |curve, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelPressed {
                    curve: curve as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelDeselectAll)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_graph_search_changed(move |text| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::GraphSearchChanged {
                    text: text.to_string(),
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_search_changed(move |text| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSearchChanged {
                    text: text.to_string(),
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_channel_pressed(move |view_row, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeChannelPressed {
                    view_row: view_row as usize,
                    modifier,
                })),
                &weak,
            );
        });

    // —— 摄影表（Dope Sheet）：关键帧（共用曲线编辑器数据/选中，拖拽只调时间） ——
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_pressed(move |curve, key, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframePressed {
                    curve: curve as usize,
                    key: key as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_dragged(move |curve, key, delta_time| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeDragged {
                    curve: curve as usize,
                    key: key as usize,
                    delta_time,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_released(move |curve, key, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeReleased {
                    curve: curve as usize,
                    key: key as usize,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeDeselectAll)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_marquee_select(move |x_lo, x_hi, row_lo, row_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeMarqueeSelect {
                    x_lo,
                    x_hi,
                    row_lo,
                    row_hi,
                    modifier,
                })),
                &weak,
            );
        });

    // —— 摄影表：汇总条带（聚合所有通道同一时刻的关键帧；跨通道同步拖拽） ——
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_summary_pressed(move |index, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryPressed {
                    index: index as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_summary_dragged(move |index, delta_time| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryDragged {
                    index: index as usize,
                    delta_time,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_summary_released(move |index, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryReleased {
                    index: index as usize,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_curve_color_mode_changed(move |curve, mode| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::CurveColorModeChanged {
                    curve: curve as usize,
                    mode,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_interpolation_changed(move |interp| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(
                    GraphAction::KeyframeInterpolationChanged {
                        interpolation: interp,
                    },
                )),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_visible(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleVisible {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_modifier(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleModifier {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_active(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleActive {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_editable(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleEditable {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_set_current_time(move |time| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::SetCurrentTime { time })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_view_changed(move |zoom_x, zoom_y| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ViewChanged {
                    zoom_x,
                    zoom_y,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_action_selected(move |index| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ActionSelected {
                    index,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_drag_cancel(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DragCancel)),
                &weak,
            );
        });
}
