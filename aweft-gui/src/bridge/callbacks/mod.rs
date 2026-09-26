//! Slint 回调装配。每个功能域占一个文件，入口是 `pub(super) fn setup_*`；本文件的
//! `setup_callbacks` 是唯一知道完整清单的地方，新增一个功能域就是加一个文件加一行调用。
//! 跨域共用的只有 `dispatch_and_render`，它留在本文件里。

use crate::actions::Action;
use crate::bridge::types::MainWindow;

use crate::store::AppStore;
use std::{cell::RefCell, rc::Rc};

mod animation;
mod area;
mod audio;
mod nla;
mod node_graph;
mod render;
mod sequence;
mod video;
mod window;

/// 设置所有回调
pub fn setup_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    area::setup_area_type_callback(instance, Rc::clone(&store));
    render::setup_rendering_notifier_callback(instance, Rc::clone(&store));
    window::setup_winit_window_event_callback(instance, Rc::clone(&store));
    area::setup_actionzone_drag_callback(instance, Rc::clone(&store));
    area::setup_actionzone_drag_end_callback(instance, Rc::clone(&store));
    area::setup_edge_drag_callback(instance, Rc::clone(&store));
    area::setup_edge_drag_end_callback(instance, Rc::clone(&store));
    sequence::setup_toggle_group_collapsed_callback(instance, Rc::clone(&store));
    sequence::setup_sequence_timecode_editor_callback(instance, Rc::clone(&store));
    sequence::setup_sequence_timecode_step_callbacks(instance, Rc::clone(&store));
    sequence::setup_sequence_drag_timecode_callbacks(instance, Rc::clone(&store));
    sequence::setup_sequence_drag_timecode_cancel_callback(instance, Rc::clone(&store));
    sequence::setup_sequence_selection_callbacks(instance, Rc::clone(&store));
    sequence::setup_sequence_add_line_callback(instance, Rc::clone(&store));
    sequence::setup_sequence_add_group_callback(instance, Rc::clone(&store));
    sequence::setup_sequence_delete_selected_items_callback(instance, Rc::clone(&store));
    sequence::setup_sequence_clipboard_callbacks(instance, Rc::clone(&store));
    sequence::setup_sequence_ungroup_callbacks(instance, Rc::clone(&store));
    node_graph::setup_node_graph_callbacks(instance, Rc::clone(&store));
    animation::setup_animation_callbacks(instance, Rc::clone(&store));
    nla::setup_nla_callbacks(instance, Rc::clone(&store));
    video::setup_video_callbacks(instance, Rc::clone(&store));
    audio::setup_audio_callbacks(instance, Rc::clone(&store));
}

/// 统一的 dispatch 和渲染逻辑。
///
/// reducer 修改数据并向依赖图打 tag；`evaluate_depsgraph` 只重算受影响的投影。
pub(super) fn dispatch_and_render(
    store: &Rc<RefCell<AppStore>>,
    action: Action,
    _weak_instance: &slint::Weak<MainWindow>,
) {
    let mut store_mut = store.borrow_mut();

    // 1. 分发 Action：修改数据 + 打 tag
    if let Err(e) = store_mut.dispatch(action) {
        log::error!("❌ Action 处理失败: {}", e);
        return;
    }

    // 2. 依赖图求值：只重算受影响投影
    store_mut.evaluate_depsgraph();
}
