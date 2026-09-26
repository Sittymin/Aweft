use crate::actions::Action;
use crate::actions::sequence::SequenceAction;
use crate::bridge::types::{MainWindow, SequenceCallbacks};

use crate::bridge::parse_time;
use crate::store::AppStore;
use aweft_model::ids::ItemId;
use log::warn;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 设置分组折叠/展开回调
pub(super) fn setup_toggle_group_collapsed_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_toggle_group_collapsed(move |group_id_str| {
            use aweft_model::ids::ItemId;
            let Ok(group_id) = uuid::Uuid::parse_str(group_id_str.as_str()).map(ItemId) else {
                warn!("无效的 group_id: {}", group_id_str);
                return;
            };

            let action = Action::Sequence(
                crate::actions::sequence::SequenceAction::ToggleGroupCollapsed { group_id },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}

pub(super) fn setup_sequence_timecode_editor_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance.global::<SequenceCallbacks>().on_timecode_input(
        move |event_id_str, is_start_time, text, is_batch| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            log::debug!("输入文本: {text}");
            if is_batch {
                // 遍历所有选中的 event 项，应用时间修改
                let selected_ids = store
                    .borrow()
                    .ui_models()
                    .sequence_state
                    .directly_selected_items();
                for &id in &selected_ids {
                    let action = Action::Sequence(SequenceAction::InputTimecode {
                        event_id: id,
                        is_start_time,
                        text: text.to_string(),
                    });
                    // dispatch each
                    dispatch_and_render(&store, action, &weak_instance);
                }
            } else {
                let action = Action::Sequence(SequenceAction::InputTimecode {
                    event_id,
                    is_start_time,
                    text: text.to_string(),
                });
                dispatch_and_render(&store, action, &weak_instance);
            }
        },
    );
}

pub(super) fn setup_sequence_timecode_step_callbacks(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    let register = |setter: fn(&MainWindow, Box<dyn FnMut(slint::SharedString, bool, bool)>),
                    build: fn(
        aweft_model::ids::ItemId,
        bool,
        bool,
    ) -> crate::actions::sequence::SequenceAction| {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        setter(
            instance,
            Box::new(move |event_id_str, is_start_time, is_batch| {
                use aweft_model::ids::ItemId;
                let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                    warn!("无效的 event_id: {}", event_id_str);
                    return;
                };
                let action = Action::Sequence(build(event_id, is_start_time, is_batch));
                dispatch_and_render(&s, action, &w);
            }),
        );
    };

    register(
        |inst, cb| inst.global::<SequenceCallbacks>().on_increment_timecode(cb),
        |event_id, is_start_time, is_batch| {
            crate::actions::sequence::SequenceAction::IncrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            }
        },
    );

    register(
        |inst, cb| inst.global::<SequenceCallbacks>().on_decrement_timecode(cb),
        |event_id, is_start_time, is_batch| {
            crate::actions::sequence::SequenceAction::DecrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            }
        },
    );
}

pub(super) fn setup_sequence_drag_timecode_callbacks(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    let register =
        |setter: fn(&MainWindow, Box<dyn FnMut(slint::SharedString, bool, f32, bool)>),
         build: fn(
            aweft_model::ids::ItemId,
            bool,
            f32,
            bool,
        ) -> crate::actions::sequence::SequenceAction| {
            let s = Rc::clone(&store);
            let w = weak_instance.clone();
            setter(
                instance,
                Box::new(move |event_id_str, is_start_time, delta_x, is_batch| {
                    use aweft_model::ids::ItemId;
                    let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId)
                    else {
                        warn!("无效的 event_id: {}", event_id_str);
                        return;
                    };
                    let action =
                        Action::Sequence(build(event_id, is_start_time, delta_x, is_batch));
                    dispatch_and_render(&s, action, &w);
                }),
            );
        };

    register(
        |inst, cb| {
            inst.global::<SequenceCallbacks>()
                .on_drag_timecode_preview(cb)
        },
        |event_id, is_start_time, delta_x, is_batch| {
            crate::actions::sequence::SequenceAction::DragTimecodePreview {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            }
        },
    );

    register(
        |inst, cb| {
            inst.global::<SequenceCallbacks>()
                .on_drag_timecode_commit(cb)
        },
        |event_id, is_start_time, delta_x, is_batch| {
            crate::actions::sequence::SequenceAction::DragTimecodeCommit {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            }
        },
    );
}

pub(super) fn setup_sequence_drag_timecode_cancel_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_drag_timecode_cancel(move |event_id_str, is_start_time, is_batch| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::Sequence(
                crate::actions::sequence::SequenceAction::DragTimecodeCancel {
                    event_id,
                    is_start_time,
                    is_batch,
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}

pub(super) fn setup_sequence_selection_callbacks(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    use aweft_model::ids::ItemId;
    let weak_instance = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_select_row(move |item_id_str, modifier| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            let action = Action::Sequence(SequenceAction::SelectRow { item_id, modifier });
            dispatch_and_render(&s, action, &weak_instance);
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_marquee_select(move |x_lo, x_hi, row_lo, row_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::MarqueeSelect {
                    x_lo,
                    x_hi,
                    row_lo,
                    row_hi,
                    modifier,
                }),
                &weak,
            );
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_deselect_all(move || {
            dispatch_and_render(&s, Action::Sequence(SequenceAction::DeselectAll), &weak);
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_block_dragged(move |item_id_str, delta_x| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::TimelineBlockDragPreview { item_id, delta_x }),
                &weak,
            );
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_block_drag_commit(move |item_id_str, delta_x| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::TimelineBlockDragCommit { item_id, delta_x }),
                &weak,
            );
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_block_drag_cancel(move |item_id_str| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::TimelineBlockDragCancel { item_id }),
                &weak,
            );
        });
}

pub(super) fn setup_sequence_add_line_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance.global::<SequenceCallbacks>().on_add_line(
        move |item_id_str, _start_time_str, end_time_str, _text| {
            use aweft_model::ids::ItemId;
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                log::warn!("[callback] 无效的 item_id: {}", item_id_str);
                return;
            };

            let end_time = if let Some(end_time) = parse_time(&end_time_str) {
                end_time
            } else {
                log::warn!("[callback] 无效的 : end_time_str {}", end_time_str);
                return;
            };

            let action = Action::Sequence(crate::actions::sequence::SequenceAction::AddLine {
                item_id,
                end_time,
                actor: None,
                // 新行不要延续参考行文本
                // TODO: 如果为明显序列 （1,2,3）可以尝试按规则延续
                // text: text.into(),
                text: String::new(),
                segment_timing: None,
                style: None,
            });

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}

pub(super) fn setup_sequence_add_group_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_add_group(move |item_id_str| {
            use aweft_model::ids::ItemId;
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                log::warn!("[callback] 无效的 item_id: {}", item_id_str);
                return;
            };

            let action =
                Action::Sequence(crate::actions::sequence::SequenceAction::AddGroup { item_id });

            dispatch_and_render(&store, action, &weak_instance);
        });
}

pub(super) fn setup_sequence_delete_selected_items_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_delete_selected_items(move || {
            let action = Action::Sequence(crate::actions::sequence::SequenceAction::DeleteItems);

            dispatch_and_render(&store, action, &weak_instance);
        });
}

pub(super) fn setup_sequence_clipboard_callbacks(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();

    {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        instance
            .global::<SequenceCallbacks>()
            .on_copy_items(move || {
                let action = Action::Sequence(SequenceAction::CopyItems);
                dispatch_and_render(&s, action, &w);
            });
    }

    {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        instance
            .global::<SequenceCallbacks>()
            .on_paste_items(move |anchor_item_id_str| {
                use aweft_model::ids::ItemId;
                let Ok(anchor) = uuid::Uuid::parse_str(anchor_item_id_str.as_str()).map(ItemId)
                else {
                    warn!("无效的 paste target item_id: {}", anchor_item_id_str);
                    return;
                };
                let action = Action::Sequence(SequenceAction::PasteItems { anchor });
                dispatch_and_render(&s, action, &w);
            });
    }

    {
        let s = Rc::clone(&store);
        let w = weak_instance;
        instance
            .global::<SequenceCallbacks>()
            .on_cut_items(move || {
                let action = Action::Sequence(SequenceAction::CutItems);
                dispatch_and_render(&s, action, &w);
            });
    }
}

pub(super) fn setup_sequence_ungroup_callbacks(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_ungroups(move |group_item_id_str| {
            let Ok(group_item_id) = uuid::Uuid::parse_str(group_item_id_str.as_str()).map(ItemId)
            else {
                log::warn!("[callback] ungroup 无效的 item_id: {}", group_item_id_str);
                return;
            };

            let action = Action::Sequence(crate::actions::sequence::SequenceAction::UnGroup {
                group_item_id,
            });

            dispatch_and_render(&store, action, &weak_instance);
        });
}
