use crate::actions::Action;
use crate::actions::sequence::SequenceAction;
use crate::bridge::types::{MainWindow, SlintSubtitleBox, SubtitleCallbacks, SubtitleState};

use crate::store::AppStore;
use aweft_model::ids::ItemId;
use log::{debug, warn};
// winit 事件只在桌面后端存在：Android 特性集里 winit 后端被彻底排除
//（见 aweft-gui/Cargo.toml），slint::winit_030 模块在该目标上不存在。
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 登记预览时钟的心跳回调（登记后立即停表，起停由 transport 决定）。
///
/// 心跳抓 store 的弱引用：时钟本身住在 `AppState` 里，抓强引用会构成循环。
pub(super) fn setup_preview_clock(instance: &MainWindow, store: &Rc<RefCell<AppStore>>) {
    let (weak_store, weak_instance) = (Rc::downgrade(store), instance.as_weak());
    store
        .borrow()
        .state()
        .preview_clock
        .install(&weak_store, &weak_instance);
}

/// 装好字幕拟合的后台通路：建通道、起 UI 线程 pump、把发送端交给 `AppState`。
///
/// 与波形批次同一套形状（后台增量直接写模型/浮层，不走 reducer）。没装这条通路的
/// 场合（无头测试）里 `request_rebuild` 会自己跳过，投影不会退回同步慢路径去冻界面。
pub(super) fn setup_subtitle_fit(store: Rc<RefCell<AppStore>>) {
    let (ui_sender, ui_receiver) =
        futures::channel::mpsc::unbounded::<crate::store::subtitle_fit::FitMessage>();
    let _ = slint::spawn_local(crate::store::subtitle_fit::ui_pump_fit_messages(
        Rc::clone(&store),
        ui_receiver,
    ));
    store.borrow_mut().state_mut().fit_ui_sender = Some(ui_sender);
}

/// 视口字幕交互回调：把行框模型交给 Slint，并实现「点中一行 → 选中」「按住 → 拖到别处」。
///
/// 坐标一律是排版像素（= 视口世界坐标）：`.slint` 侧已经做过屏幕→世界的换算，这里不再换算。
/// 命中查的是模型而不是 libass 的原始框——模型上压着提交后还没落进脚本的预览偏移，视口画的
/// 也正是这一份，两者必须同一口径，否则点中的框与拖动的框不会是同一条。
pub(super) fn setup_subtitle_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::store::subtitle::LineDragSession;

    let boxes = store.borrow().ui_models().subtitle.boxes.clone();
    instance
        .global::<SubtitleState>()
        .set_boxes(slint::ModelRc::from(boxes));

    // 点击一行字幕：选中交给序列的行选择（行编辑器与视口共用同一份选择集合）。
    // 返回 true 表示这一点归字幕，视口就不要再把它交给视频选中。
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<SubtitleCallbacks>()
            .on_line_picked(move |x, y, modifier| {
                let Some(line) = subtitle_line_at(&s, x, y) else {
                    return false;
                };
                let Some(item_id) = subtitle_line_id(&line) else {
                    return false;
                };
                dispatch_and_render(
                    &s,
                    Action::Sequence(SequenceAction::SelectRow { item_id, modifier }),
                    &weak,
                );
                crate::projections::ui_sync_subtitle_boxes(&mut s.borrow_mut());
                true
            });
    }

    // 按住一行想把它拖走：Rust 自己对这一点命中一次，记下命中的是哪一行，以及它此刻已经
    // 压着的偏移（上一次提交还没落进脚本时非零，接着拖才不会让框跳回旧位置）。
    // 命中判定留在这里：没命中的真实原因只有这一侧知道，视口只照返回值决定进不进字幕
    // 拖动分支。
    {
        let s = Rc::clone(&store);
        instance
            .global::<SubtitleCallbacks>()
            .on_line_drag_started(move |x, y| {
                let Some(line) = subtitle_line_at(&s, x, y) else {
                    debug!(
                        "视口：({x:.0}, {y:.0}) 没命中字幕行，当前画了 {} 个行框",
                        s.borrow().ui_models().subtitle.boxes.row_count()
                    );
                    return false;
                };
                let Some(item_id) = subtitle_line_id(&line) else {
                    warn!("视口字幕行身份无法解析: {}", line.item_id);
                    return false;
                };
                let previous = s.borrow().state().subtitle_line_drag;
                let (pending_x, pending_y) = match previous {
                    Some(drag) if drag.item_id == item_id => {
                        (drag.pending_x + drag.drag_x, drag.pending_y + drag.drag_y)
                    }
                    _ => (0.0, 0.0),
                };
                s.borrow_mut().state_mut().subtitle_line_drag = Some(LineDragSession {
                    item_id,
                    pending_x,
                    pending_y,
                    drag_x: 0.0,
                    drag_y: 0.0,
                    committed: false,
                });
                crate::projections::ui_sync_subtitle_boxes(&mut s.borrow_mut());
                true
            });
    }

    // 拖动中：只更新会话里的位移并重画框。重编一份脚本要几百毫秒，所以全程不写数据。
    {
        let s = Rc::clone(&store);
        instance
            .global::<SubtitleCallbacks>()
            .on_line_dragged(move |dx, dy| {
                {
                    let mut app = s.borrow_mut();
                    let Some(drag) = app.state_mut().subtitle_line_drag.as_mut() else {
                        return;
                    };
                    if drag.committed {
                        return;
                    }
                    drag.drag_x = dx as f64;
                    drag.drag_y = dy as f64;
                }
                crate::projections::ui_sync_subtitle_boxes(&mut s.borrow_mut());
            });
    }

    // 松手：把这一笔位移一次性提交到这一行的视口链（样式链的「设位置」基线）。提交之后
    // 脚本还是旧的，所以会话留在原地继续压着画，直到后台那份脚本落地才由同步出口清掉。
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<SubtitleCallbacks>()
            .on_line_released(move || {
                let drag = s.borrow().state().subtitle_line_drag;
                let Some(drag) = drag else { return };
                if drag.committed {
                    return;
                }
                let (dx, dy) = (drag.drag_x, drag.drag_y);
                debug!(
                    "视口：把 {} 的位置基线平移 ({dx:.1}, {dy:.1})",
                    drag.item_id
                );
                dispatch_and_render(
                    &s,
                    Action::Subtitle(
                        crate::actions::subtitle::SubtitleAction::ShiftLinePosition {
                            item_id: drag.item_id,
                            dx,
                            dy,
                        },
                    ),
                    &weak,
                );
                if let Some(current) = s.borrow_mut().state_mut().subtitle_line_drag.as_mut() {
                    current.committed = true;
                }
                crate::projections::ui_sync_subtitle_boxes(&mut s.borrow_mut());
            });
    }
}

/// 这一点命中第几行字幕的框（`SubtitleState.boxes` 的下标），没有则 -1。
/// 倒着查：列表后面那些框画在前面那些之上，罩住这一点的框里最上层那个才是意图。
fn hit_subtitle_line(
    boxes: &slint::VecModel<crate::bridge::types::SlintSubtitleBox>,
    x: f32,
    y: f32,
) -> i32 {
    (0..boxes.row_count())
        .rev()
        .find(|&index| {
            boxes.row_data(index).is_some_and(|line| {
                line.x <= x && x < line.x + line.width && line.y <= y && y < line.y + line.height
            })
        })
        .map_or(-1, |index| index as i32)
}

/// 这一点命中的那一行框（画面上的位置与偏移都在里面），没命中则 `None`。
fn subtitle_line_at(store: &Rc<RefCell<AppStore>>, x: f32, y: f32) -> Option<SlintSubtitleBox> {
    let app = store.borrow();
    let boxes = &app.ui_models().subtitle.boxes;
    let index = hit_subtitle_line(boxes, x, y);
    if index < 0 {
        return None;
    }
    boxes.row_data(index as usize)
}

/// 框上带的行身份：与序列编辑器共用的那个 `ItemId`。
fn subtitle_line_id(line: &SlintSubtitleBox) -> Option<ItemId> {
    uuid::Uuid::parse_str(line.item_id.as_str())
        .map(ItemId)
        .ok()
}
