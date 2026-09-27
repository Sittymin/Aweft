use crate::actions::Action;
use crate::bridge::types::{AnimationCallbacks, MainWindow};

use crate::store::AppStore;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 设置 NLA 编辑器回调：片断/轨道交互走 `Action::Nla` → dispatch → tag → 投影。
/// 命中检测（`nla-strip-pick`）为同步查询（拖拽需要立即结果）+ 选中修改：
/// 数据层就地更新后手动打 tag 并 evaluate（与 dispatch 同管线）。
pub(super) fn setup_nla_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::actions::animation::{AnimationAction, nla::NlaAction};

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_pressed(move |track, strip, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripPressed {
                    track: track as usize,
                    strip: strip as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_dragged(move |delta_x, delta_row| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripDragged {
                    delta_x,
                    delta_row,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_released(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripReleased)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripDeselectAll)),
                &weak,
            );
        });

    // 命中检测：纯命中（只读，不改数据层）→ 若命中则走标准 dispatch 管线选中并记录拖拽起点。
    // 返回编码供画布判定拖拽起点（track*10000+strip），未命中 -1。
    // 命中查询与 dispatch 分作用域，避免 RefCell 借用重叠。
    let s = Rc::clone(&store);
    let weak = instance.as_weak();
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_pick(move |x, row, modifier| -> i32 {
            let hit = {
                let store_ref = s.borrow();
                store_ref.ui_models.animation.nla.strip_hit_test(x, row)
            };
            if let Some((track, strip)) = hit {
                dispatch_and_render(
                    &s,
                    Action::Animation(AnimationAction::Nla(NlaAction::StripPressed {
                        track,
                        strip,
                        modifier,
                    })),
                    &weak,
                );
                (track as i32) * 10000 + (strip as i32)
            } else {
                -1
            }
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_track_toggle_enabled(move |track| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::ToggleTrackEnabled {
                    track: track as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_track_toggle_locked(move |track| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::ToggleTrackLocked {
                    track: track as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_track_push_down(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::PushDown)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_search_changed(move |text| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::SearchChanged {
                    text: text.to_string(),
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_drag_cancel(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripDragCancel)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_marquee_select(move |x_lo, x_hi, row_lo, row_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::MarqueeSelect {
                    x_lo,
                    x_hi,
                    row_lo,
                    row_hi,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_anim_selected(move |index| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::AnimDataSelected { index })),
                &weak,
            );
        });
}
