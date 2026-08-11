use crate::actions::line_editor::LineEditorAction;
use crate::actions::{Action, area::AreaAction};
use crate::bridge::types::{
    AnimationCallbacks, AreaCallbacks, DragCallbacks, LineEditorCallbacks, MainWindow,
    VideoCallbacks,
};
use crate::bridge::{parse_time, slint_area_key_to_key, slint_area_type_to_rust};
use crate::store::AppStore;
use crate::store::reducers::media::connect_video_to_wgpu;
use crate::ui_update::{UIUpdateCommand, UIUpdater};
use aweft_model::ids::ItemId;
use log::warn;
use slint::winit_030::winit::{self, event::WindowEvent};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, RenderingState, SharedString};
use std::{cell::RefCell, rc::Rc};

/// 设置所有回调
pub fn setup_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    setup_area_type_callback(instance, Rc::clone(&store));
    setup_rendering_notifier_callback(instance, Rc::clone(&store));
    setup_winit_window_event_callback(instance, Rc::clone(&store));
    setup_actionzone_drag_callback(instance, Rc::clone(&store));
    setup_actionzone_drag_end_callback(instance, Rc::clone(&store));
    setup_edge_drag_callback(instance, Rc::clone(&store));
    setup_edge_drag_end_callback(instance, Rc::clone(&store));
    setup_toggle_group_collapsed_callback(instance, Rc::clone(&store));
    setup_line_editor_timecode_editor_callback(instance, Rc::clone(&store));
    setup_line_editor_timecode_step_callbacks(instance, Rc::clone(&store));
    setup_line_editor_drag_timecode_callbacks(instance, Rc::clone(&store));
    setup_line_editor_drag_timecode_cancel_callback(instance, Rc::clone(&store));
    setup_line_editor_selection_callbacks(instance, Rc::clone(&store));
    setup_line_editor_add_line_callback(instance, Rc::clone(&store));
    setup_line_editor_add_group_callback(instance, Rc::clone(&store));
    setup_line_editor_delete_selected_items_callback(instance, Rc::clone(&store));
    setup_line_editor_clipboard_callbacks(instance, Rc::clone(&store));
    setup_line_editor_ungroup_callbacks(instance, Rc::clone(&store));
    setup_node_graph_callbacks(instance, Rc::clone(&store));
    setup_animation_callbacks(instance, Rc::clone(&store));
    setup_video_callbacks(instance, Rc::clone(&store));
}

/// 设置视口回调：打开视频（当前为占位——文件选择器接入后在此调用
/// `initialize_media` 并连接 wgpu 输出）。
fn setup_video_callbacks(instance: &MainWindow, _store: Rc<RefCell<AppStore>>) {
    instance.global::<VideoCallbacks>().on_open_video(move || {
        log::info!("打开视频：待接入文件选择对话框（rfd 等）后调用 initialize_media");
    });
}

/// 设置节点图编辑器回调：图结构操作全部在 Rust 侧完成，Slint 只上报事件。
fn setup_node_graph_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::bridge::types::{NodeGraphCallbacks, SocketHit};

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_node_dragged(
        move |graph_type, node_index, x, y| {
            s.borrow_mut()
                .set_node_position(graph_type, node_index as usize, x, y);
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_add_node(
        move |graph_type, template_index, x, y| -> i32 {
            s.borrow_mut()
                .add_node(graph_type, template_index as usize, x, y)
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_delete_node(
        move |graph_type, node_index| {
            s.borrow_mut()
                .delete_node(graph_type, node_index as usize);
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_find_link_for_input(
        move |graph_type, node_index, socket_index| -> i32 {
            s.borrow()
                .find_link_for_input(graph_type, node_index as usize, socket_index as usize)
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_find_drop_socket(
        move |graph_type, x, y, data_type| -> SocketHit {
            s.borrow()
                .find_drop_socket(graph_type, x, y, data_type)
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_socket_drag_ended(
        move |graph_type, node_index, socket_index, is_input, x, y| {
            s.borrow_mut().socket_drag_ended(
                graph_type,
                node_index as usize,
                socket_index as usize,
                is_input,
                x,
                y,
            );
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_load_chain(
        move |graph_type, index| {
            s.borrow_mut().load_chain(graph_type, index as usize);
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_new_chain(
        move |graph_type| -> i32 {
            s.borrow_mut().new_chain(graph_type)
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_menu_selected(
        move |graph_type, node_index, menu_index| {
            s.borrow_mut().menu_selected(
                graph_type,
                node_index as usize,
                menu_index as usize,
            );
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_socket_option_selected(
        move |graph_type, node_index, socket_row, option_index| {
            s.borrow_mut().socket_option_selected(
                graph_type,
                node_index as usize,
                socket_row as usize,
                option_index as usize,
            );
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_create_actor(
        move |graph_type, node_index, socket_row, name| {
            s.borrow_mut().create_actor(
                graph_type,
                node_index as usize,
                socket_row as usize,
                name,
            );
        },
    );

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_socket_input_changed(
        move |graph_type, node_index, socket_row, text| {
            s.borrow_mut().set_socket_text_default(
                graph_type,
                node_index as usize,
                socket_row as usize,
                &text,
            );
        },
    );
}

/// 设置动画编辑器回调（曲线编辑器 + NLA 编辑器）：数据操作全部在 Rust 侧完成。
fn setup_animation_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::bridge::types::AnimationState;

    // 同步当前激活通道到 Slint 全局（点击曲线/关键帧/控制柄后）
    let sync_current_channel =
        |s: &Rc<RefCell<AppStore>>, weak: &slint::Weak<MainWindow>| {
            if let Some(inst) = weak.upgrade() {
                let cur = s.borrow().ui_models().curve_editor.current_channel;
                inst.global::<AnimationState>().set_current_channel(cur as i32);
            }
        };

    // 同步关键帧选中状态到 Slint 全局（侧边栏插值面板依赖）
    let sync_keyframe_selection =
        |s: &Rc<RefCell<AppStore>>, weak: &slint::Weak<MainWindow>| {
            if let Some(inst) = weak.upgrade() {
                let s_ref = s.borrow();
                let curve = &s_ref.ui_models().curve_editor;
                inst.global::<AnimationState>()
                    .set_selected_keyframes(curve.selected_keyframe_count() as i32);
                inst.global::<AnimationState>()
                    .set_selected_interpolation(curve.selected_interpolation_index());
            }
        };

    // 把曲线编辑器当前 Action 的名称与是否有数据同步到 NLA 全局 Action 轨
    //（左侧列表第一行显示实际 Action 名；空 Action 时"向下插入"按钮隐藏）。
    let sync_nla_global_action = |s: &Rc<RefCell<AppStore>>| {
        let (name, has) = {
            let s_ref = s.borrow();
            let ce = &s_ref.ui_models().curve_editor;
            (ce.action_name.clone(), ce.has_keyframes())
        };
        s.borrow_mut()
            .ui_models_mut()
            .nla_editor
            .set_global_action(name, has);
    };

    // ── 曲线编辑器：关键帧 ──
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_keyframe_pressed(
            move |curve, key, additive| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .keyframe_pressed(curve as usize, key as usize, additive);
                sync_current_channel(&s, &weak);
                sync_keyframe_selection(&s, &weak);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_keyframe_dragged(
            move |curve, key, delta_time, delta_value, mode| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .keyframe_dragged(curve as usize, key as usize, delta_time, delta_value, mode);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_keyframe_released(
            move |curve, key, moved| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .keyframe_released(curve as usize, key as usize, moved);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_keyframe_deselect_all(move || {
            s.borrow_mut().ui_models_mut().curve_editor.deselect_all();
            sync_keyframe_selection(&s, &weak);
        });
    }
    // ── 曲线编辑器：贝塞尔控制柄 ──
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_handle_pressed(
            move |curve, key, is_in, additive| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .handle_pressed(curve as usize, key as usize, is_in, additive);
                sync_current_channel(&s, &weak);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_handle_dragged(
            move |curve, key, is_in, delta_time, delta_value, mode| {
                s.borrow_mut().ui_models_mut().curve_editor.handle_dragged(
                    curve as usize,
                    key as usize,
                    is_in,
                    delta_time,
                    delta_value,
                    mode,
                );
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_handle_released(
            move |curve, key, is_in, moved| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .handle_released(curve as usize, key as usize, is_in, moved);
            },
        );
    }
    {
        // 曲线编辑器空白框选：按世界时间范围 × 值范围选中关键帧与控制柄
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_graph_marquee_select(
            move |x_lo, x_hi, y_lo, y_hi, mode| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .graph_marquee_select(x_lo, x_hi, y_lo, y_hi, mode);
                sync_keyframe_selection(&s, &weak);
                sync_current_channel(&s, &weak);
            },
        );
    }
    // ── 曲线编辑器：通道 ──
    // 通道/曲线按下（曲线本体或列表行）：mode 0=普通，1=Shift 区间，2=Ctrl 切换
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_channel_pressed(move |curve, mode| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .channel_pressed(curve as usize, mode);
            sync_current_channel(&s, &weak);
        });
    }
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_channel_deselect_all(move || {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .deselect_channel();
            sync_current_channel(&s, &weak);
        });
    }
    {
        // 曲线编辑器搜索框：只过滤曲线编辑器自己的通道视图（重建 FilterModel 并重新绑定）
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_graph_search_changed(move |text| {
            s.borrow_mut().ui_models_mut().curve_editor.set_graph_search(text.as_str());
            if let Some(inst) = weak.upgrade() {
                let s = s.borrow();
                let ce = &s.ui_models().curve_editor;
                inst.global::<AnimationState>()
                    .set_graph_channels(ce.graph_filter.clone());
                inst.global::<AnimationState>()
                    .set_graph_segments(ce.graph_segments.clone());
                inst.global::<AnimationState>()
                    .set_graph_handles(ce.graph_handles.clone());
            }
        });
    }
    {
        // 摄影表搜索框：只过滤摄影表自己的通道视图
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_dope_search_changed(move |text| {
            s.borrow_mut().ui_models_mut().curve_editor.set_dope_search(text.as_str());
            if let Some(inst) = weak.upgrade() {
                let s = s.borrow();
                inst.global::<AnimationState>()
                    .set_dope_channels(s.ui_models().curve_editor.dope_filter.clone());
            }
        });
    }
    {
        // 摄影表 Header 中区：Action 选择下拉菜单
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_action_selected(move |index| {
            s.borrow_mut().ui_models_mut().curve_editor.select_action(index);
            if let Some(inst) = weak.upgrade() {
                let s = s.borrow();
                let ce = &s.ui_models().curve_editor;
                inst.global::<AnimationState>().set_action_name(ce.action_name.clone());
                inst.global::<AnimationState>().set_current_action(ce.current_action);
                inst.global::<AnimationState>().set_action_start(ce.action_start);
                inst.global::<AnimationState>().set_action_end(ce.action_end);
            }
            // 同步 NLA 全局 Action 轨（名称 = 实际 Action 名 + 是否有数据）
            sync_nla_global_action(&s);
        });
    }
    {
        // 摄影表压缩视图点击：显示行 → 真实通道下标
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_dope_channel_pressed(move |view_row, mode| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .dope_channel_pressed(view_row as usize, mode);
            sync_current_channel(&s, &weak);
        });
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_channel_toggle_visible(move |curve| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .toggle_channel_visible(curve as usize);
        });
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_channel_toggle_modifier(move |curve| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .toggle_channel_modifier(curve as usize);
        });
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_channel_toggle_active(move |curve| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .toggle_channel_active(curve as usize);
        });
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_channel_toggle_editable(move |curve| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .toggle_channel_editable(curve as usize);
        });
    }
    // ── 曲线编辑器：侧边栏 ──
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_curve_color_mode_changed(move |curve, mode| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .set_curve_color_mode(curve as usize, mode);
        });
    }
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_keyframe_interpolation_changed(move |interp| {
            s.borrow_mut()
                .ui_models_mut()
                .curve_editor
                .set_selected_keyframes_interpolation(interp);
            sync_keyframe_selection(&s, &weak);
        });
    }
    // ── NLA 编辑器：片断 ──
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_strip_pressed(
            move |track, strip, additive| {
                s.borrow_mut()
                    .ui_models_mut()
                    .nla_editor
                    .strip_pressed(track as usize, strip as usize, additive);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_strip_pick(
            move |x, row, mode| -> i32 {
                s.borrow_mut()
                    .ui_models_mut()
                    .nla_editor
                    .strip_pick(x, row, mode)
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_strip_dragged(
            move |track, strip, delta_x, delta_row| {
                s.borrow_mut()
                    .ui_models_mut()
                    .nla_editor
                    .strip_dragged(track as usize, strip as usize, delta_x, delta_row);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_strip_released(
            move |track, strip| {
                s.borrow_mut()
                    .ui_models_mut()
                    .nla_editor
                    .strip_released(track as usize, strip as usize);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_strip_deselect_all(move || {
            s.borrow_mut().ui_models_mut().nla_editor.deselect_all();
        });
    }
    // ── 动画编辑器通用 ──

    // ── NLA 编辑器：轨道 ──
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_track_toggle_enabled(move |track| {
            s.borrow_mut()
                .ui_models_mut()
                .nla_editor
                .toggle_track_enabled(track as usize);
        });
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_track_toggle_locked(move |track| {
            s.borrow_mut()
                .ui_models_mut()
                .nla_editor
                .toggle_track_locked(track as usize);
        });
    }
    {
        // 全局 Action 轨"向下插入"：把当前编辑 Action 作为片断加入 NLA，
        // 然后重置全局 Action（开始新的空 Action）。
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_track_push_down(move |_track| {
            let (name, start, end) = {
                let s_ref = s.borrow();
                let ce = &s_ref.ui_models().curve_editor;
                (ce.action_name.clone(), ce.action_start, ce.action_end)
            };
            s.borrow_mut()
                .ui_models_mut()
                .nla_editor
                .push_down(name, start, end);
            if let Some(inst) = weak.upgrade() {
                let mut s_mut = s.borrow_mut();
                let ce = &mut s_mut.ui_models_mut().curve_editor;
                ce.reset_action();
                inst.global::<AnimationState>().set_action_name(ce.action_name.clone());
                inst.global::<AnimationState>().set_current_action(ce.current_action);
                inst.global::<AnimationState>().set_action_start(ce.action_start);
                inst.global::<AnimationState>().set_action_end(ce.action_end);
            }
            // 下插后全局 Action 变空（名称保留、无数据 → 插入按钮隐藏）
            sync_nla_global_action(&s);
        });
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_nla_search_changed(move |text| {
            s.borrow_mut()
                .ui_models_mut()
                .nla_editor
                .filter_tracks(text);
        });
    }
    // ── 摄影表（Dope Sheet）：与曲线编辑器共用数据/选中，拖拽只调时间 ──
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_dope_keyframe_pressed(
            move |curve, key, additive| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .keyframe_pressed(curve as usize, key as usize, additive);
                sync_current_channel(&s, &weak);
                sync_keyframe_selection(&s, &weak);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_dope_keyframe_dragged(
            move |curve, key, delta_time| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .keyframe_dragged(curve as usize, key as usize, delta_time, 0.0, 0);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_dope_keyframe_released(
            move |curve, key, moved| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .keyframe_released(curve as usize, key as usize, moved);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_dope_keyframe_deselect_all(move || {
            s.borrow_mut().ui_models_mut().curve_editor.deselect_all();
            sync_keyframe_selection(&s, &weak);
        });
    }
    {
        // 摄影表空白处框选：按世界时间范围 + 行区间选中关键帧（mode 0=替换，非 0=追加）
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_dope_marquee_select(
            move |x_lo, x_hi, row_lo, row_hi, mode| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .dope_marquee_select(x_lo, x_hi, row_lo, row_hi, mode);
                sync_keyframe_selection(&s, &weak);
                sync_current_channel(&s, &weak);
            },
        );
    }
    // ── 摄影表：汇总条带（聚合所有通道同一时刻的关键帧；跨通道同步拖拽） ──
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_dope_summary_pressed(
            move |index, additive| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .summary_keyframe_pressed(index as usize, additive);
                sync_current_channel(&s, &weak);
                sync_keyframe_selection(&s, &weak);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        instance.global::<AnimationCallbacks>().on_dope_summary_dragged(
            move |index, delta_time| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .summary_keyframe_dragged(index as usize, delta_time);
            },
        );
    }
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_dope_summary_released(
            move |_index, moved| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .summary_keyframe_released(moved);
                sync_keyframe_selection(&s, &weak);
            },
        );
    }
    // ── 共享：当前时间 ──
    {
        let s = Rc::clone(&store);
        let weak = instance.as_weak();
        instance.global::<AnimationCallbacks>().on_set_current_time(move |time| {
            // 播放头按整数帧步进（Blender 风格，anim_ops.cc change_frame_apply）：
            // frame = round(time * fps) / fps，fps 为高层属性（后续随视频/项目设置变化）。
            // 这也保证拖动手柄时"一卡一卡"地对齐到帧。
            let fps = s.borrow().state().fps.max(0.001);
            let time = (time * fps).round() / fps;
            {
                let mut store_mut = s.borrow_mut();
                store_mut.ui_models_mut().curve_editor.current_time = time as f64;
                store_mut.ui_models_mut().nla_editor.current_time = time as f64;
            }
            if let Some(inst) = weak.upgrade() {
                inst.global::<AnimationState>().set_current_time(time);
            }
        });
    }
    {
        // 视图缩放变化（滚轮）：更新命中检测容差依据。
        // 刻度已完全移到 Slint 侧即时计算（ruler.slint），pan/画布尺寸不再需要 Rust 状态。
        let s = Rc::clone(&store);
        instance
            .global::<AnimationCallbacks>()
            .on_view_changed(move |zoom_x, zoom_y, pan_x, pan_y, width, height| {
                s.borrow_mut()
                    .ui_models_mut()
                    .curve_editor
                    .set_view(zoom_x, zoom_y, pan_x, pan_y, width, height);
            });
    }
}

/// 设置区域类型切换回调 - 使用 Action 模式
fn setup_area_type_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();

    instance.global::<AreaCallbacks>().on_switch_area_type(
        move |slint_area_key, slint_area_type| {
            let area_key = slint_area_key_to_key(&slint_area_key);
            let area_type = slint_area_type_to_rust(&slint_area_type);

            // 创建 Action
            let action = Action::Area(AreaAction::SwitchType {
                area_key,
                area_type,
            });

            // 分发 Action 并更新 UI
            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}

fn setup_rendering_notifier_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let window = instance.window();
    let weak_instance = instance.as_weak();

    if let Err(e) = window.set_rendering_notifier(move |rendering_state, graphics_api| {
        let instance_opt = weak_instance.upgrade();

        match rendering_state {
            RenderingState::RenderingSetup => {
                if let Some(instance) = instance_opt.as_ref() {
                    handle_rendering_setup(instance, &store, graphics_api);
                }
            }
            RenderingState::BeforeRendering => {
                if let Some(instance) = instance_opt.as_ref() {
                    handle_before_rendering(instance, &store);
                }
            }
            RenderingState::RenderingTeardown => {
                // Teardown 通常不需要实例
                handle_rendering_teardown(&store);
            }
            _ => (),
        }
    }) {
        eprintln!("设置渲染通知器失败: {}", e);
    }
}
fn handle_rendering_teardown(store: &Rc<RefCell<AppStore>>) {
    // 清理 WGPU sink (链式调用玩直接释放锁)
    if let Some(wgpu_sink) = &store.borrow().state().wgpu_sink {
        wgpu_sink.deactivate_and_pause();
    }
}

fn handle_before_rendering(instance: &MainWindow, store: &Rc<RefCell<AppStore>>) {
    // 获取新的视频帧并设置到 UI
    let mut image_opt = None;
    if let Some(wgpu_sink) = &store.borrow_mut().state().wgpu_sink {
        image_opt = wgpu_sink.fetch_next_frame();
    }
    if let Some(image) = image_opt {
        println!("收到新视频帧，设置到 UI");
        instance
            .global::<crate::bridge::types::VideoState>()
            .set_current_video_frame(image);
    }
}

fn handle_rendering_setup(
    instance: &MainWindow,
    store: &Rc<RefCell<AppStore>>,
    graphics_api: &slint::GraphicsAPI,
) {
    println!("窗口初始化完成",);
    let windows_size = instance.window().size();
    let action = Action::Screen(crate::actions::screen::ScreenAction::InitializeLayout {
        width: windows_size.width,
        height: windows_size.height,
    });
    dispatch_and_render(store, action, &instance.as_weak());

    // HACK: 后续需要修改为按需执行
    if let Err(e) = connect_video_to_wgpu(store, instance, graphics_api) {
        eprintln!("❌ 视频系统连接失败: {}", e);
    }
}

fn setup_winit_window_event_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let window = instance.window();
    let weak_instance = instance.as_weak();

    window.on_winit_window_event(move |_slint_window, event| {
        let instance_opt = weak_instance.upgrade();
        match event {
            WindowEvent::Resized(new_size) => {
                // TODO: 这部分重构没有处理
                // // HACK: 更新字幕
                // // FIXME: 需要使用动态参数的数据
                // let subtitle_image_opt = Self::render_hello_world_subtitle(
                //     &mut state,
                //     &instance,
                //     new_size.width as i32,
                //     new_size.height as i32,
                // );
                // if let Some((subtitle_image, x, y)) = subtitle_image_opt {
                //     instance
                //         .global::<SubtitleState>()
                //         .set_current_subtitle_frame(subtitle_image);
                //     instance.set_subtitle_x(x as f32);
                //     instance.set_subtitle_y(y as f32);
                // }
                if let Some(instance) = instance_opt.as_ref() {
                    handle_resized(instance, &store, &new_size);
                }
            }
            winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(instance) = instance_opt.as_ref() {
                    handle_scale_factor_changed(instance, &store, *scale_factor);
                }
            }
            _ => {}
        }
        // 确保事件继续执行
        EventResult::Propagate
    });
}

fn handle_resized(
    instance: &MainWindow,
    store: &Rc<RefCell<AppStore>>,
    new_size: &winit::dpi::PhysicalSize<u32>,
) {
    println!("窗口大小发生改变: {}x{}", new_size.width, new_size.height);

    let action = Action::Screen(crate::actions::screen::ScreenAction::WindowResized {
        width: new_size.width,
        height: new_size.height,
    });

    dispatch_and_render(store, action, &instance.as_weak());
}

fn handle_scale_factor_changed(
    instance: &MainWindow,
    store: &Rc<RefCell<AppStore>>,
    scale_factor: f64,
) {
    println!("检测到屏幕缩放因子变化: {:.2}", scale_factor);
    let action =
        Action::Screen(crate::actions::screen::ScreenAction::ScaleFactorChanged { scale_factor });

    dispatch_and_render(store, action, &instance.as_weak());
}

fn setup_actionzone_drag_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();

    instance.global::<DragCallbacks>().on_actionzone_drag(
        move |area_key, corner, offset_x, offset_y| {
            let action = Action::Drag(crate::actions::drag::DragAction::ActionZone(
                crate::actions::drag::ActionZoneDrag::Update {
                    area_key,
                    corner,
                    offset_x,
                    offset_y,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}

fn setup_actionzone_drag_end_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<DragCallbacks>()
        .on_actionzone_drag_end(move |area_key, is_cancel| {
            let action = Action::Drag(crate::actions::drag::DragAction::ActionZone(
                crate::actions::drag::ActionZoneDrag::End {
                    area_key,
                    is_cancel,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_edge_drag_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance.global::<DragCallbacks>().on_edge_drag(
        move |vert_keys,
              // 此坐标是相较于 UI Edge 矩形左上角的
              offset_x,
              offset_y| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::Update {
                    vert_keys,
                    offset_x,
                    offset_y,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}
fn setup_edge_drag_end_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<DragCallbacks>()
        .on_edge_drag_end(move |is_cancel| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::End { is_cancel },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
}

/// 设置分组折叠/展开回调
fn setup_toggle_group_collapsed_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_toggle_group_collapsed(move |group_id_str| {
            use aweft_model::ids::ItemId;
            let Ok(group_id) = uuid::Uuid::parse_str(group_id_str.as_str()).map(ItemId) else {
                warn!("无效的 group_id: {}", group_id_str);
                return;
            };

            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::ToggleGroupCollapsed { group_id },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_line_editor_timecode_editor_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance.global::<LineEditorCallbacks>().on_timecode_input(
        move |event_id_str, is_start_time, text, is_batch| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            println!("输入文本: {text}");
            if is_batch {
                // 遍历所有选中的 event 项，应用时间修改
                let selected_ids = store
                    .borrow()
                    .ui_models()
                    .line_editor_state
                    .directly_selected_items()
                    .clone();
                for &id in &selected_ids {
                    let action = Action::LineEditor(LineEditorAction::InputTimecode {
                        event_id: id,
                        is_start_time,
                        text: text.to_string(),
                    });
                    // dispatch each
                    dispatch_and_render(&store, action, &weak_instance);
                }
            } else {
                let action = Action::LineEditor(LineEditorAction::InputTimecode {
                    event_id,
                    is_start_time,
                    text: text.to_string(),
                });
                dispatch_and_render(&store, action, &weak_instance);
            }
        },
    );
}
fn setup_line_editor_timecode_step_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    let register = |setter: fn(&MainWindow, Box<dyn FnMut(slint::SharedString, bool, bool)>),
                    build: fn(
        aweft_model::ids::ItemId,
        bool,
        bool,
    ) -> crate::actions::line_editor::LineEditorAction| {
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
                let action = Action::LineEditor(build(event_id, is_start_time, is_batch));
                dispatch_and_render(&s, action, &w);
            }),
        );
    };

    register(
        |inst, cb| {
            inst.global::<LineEditorCallbacks>()
                .on_increment_timecode(cb)
        },
        |event_id, is_start_time, is_batch| {
            crate::actions::line_editor::LineEditorAction::IncrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            }
        },
    );

    register(
        |inst, cb| {
            inst.global::<LineEditorCallbacks>()
                .on_decrement_timecode(cb)
        },
        |event_id, is_start_time, is_batch| {
            crate::actions::line_editor::LineEditorAction::DecrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            }
        },
    );
}

fn setup_line_editor_drag_timecode_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    let register =
        |setter: fn(&MainWindow, Box<dyn FnMut(slint::SharedString, bool, f32, bool)>),
         build: fn(
            aweft_model::ids::ItemId,
            bool,
            f32,
            bool,
        ) -> crate::actions::line_editor::LineEditorAction| {
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
                        Action::LineEditor(build(event_id, is_start_time, delta_x, is_batch));
                    dispatch_and_render(&s, action, &w);
                }),
            );
        };

    register(
        |inst, cb| {
            inst.global::<LineEditorCallbacks>()
                .on_drag_timecode_preview(cb)
        },
        |event_id, is_start_time, delta_x, is_batch| {
            crate::actions::line_editor::LineEditorAction::DragTimecodePreview {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            }
        },
    );

    register(
        |inst, cb| {
            inst.global::<LineEditorCallbacks>()
                .on_drag_timecode_commit(cb)
        },
        |event_id, is_start_time, delta_x, is_batch| {
            crate::actions::line_editor::LineEditorAction::DragTimecodeCommit {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            }
        },
    );
}
fn setup_line_editor_drag_timecode_cancel_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_drag_timecode_cancel(move |event_id_str, is_start_time, is_batch| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::DragTimecodeCancel {
                    event_id,
                    is_start_time,
                    is_batch,
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_line_editor_selection_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use aweft_model::ids::ItemId;
    let weak_instance = instance.as_weak();

    let register =
        |f: fn(ItemId) -> LineEditorAction,
         setter: fn(&MainWindow, Box<dyn FnMut(slint::SharedString)>)| {
            let s = Rc::clone(&store);
            let w = weak_instance.clone();
            setter(
                instance,
                Box::new(move |item_id_str| {
                    use aweft_model::ids::ItemId;
                    let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId)
                    else {
                        warn!("无效的 item_id: {}", item_id_str);
                        return;
                    };
                    let action = Action::LineEditor(f(item_id));
                    dispatch_and_render(&s, action, &w);
                }),
            );
        };

    register(
        |item_id| LineEditorAction::SelectSingle { item_id },
        |inst, cb| inst.global::<LineEditorCallbacks>().on_select_single(cb),
    );
    register(
        |item_id| LineEditorAction::SelectRange { item_id },
        |inst, cb| inst.global::<LineEditorCallbacks>().on_select_range(cb),
    );
    register(
        |item_id| LineEditorAction::SelectToggle { item_id },
        |inst, cb| inst.global::<LineEditorCallbacks>().on_select_toggle(cb),
    );
}

fn setup_line_editor_add_line_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance.global::<LineEditorCallbacks>().on_add_line(
        move |item_id_str, start_time_str, end_time_str, text| {
            use aweft_model::ids::ItemId;
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                println!("[callback] 无效的 item_id: {}", item_id_str);
                return;
            };

            let start_time = if let Some(start_time) = parse_time(&start_time_str) {
                start_time
            } else {
                println!("[callback] 无效的 : start_time_str {}", start_time_str);
                return;
            };

            let end_time = if let Some(end_time) = parse_time(&end_time_str) {
                end_time
            } else {
                println!("[callback] 无效的 : end_time_str {}", end_time_str);
                return;
            };

            let action =
                Action::LineEditor(crate::actions::line_editor::LineEditorAction::AddLine {
                    item_id,
                    start_time,
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

fn setup_line_editor_add_group_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_add_group(move |item_id_str| {
            use aweft_model::ids::ItemId;
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                println!("[callback] 无效的 item_id: {}", item_id_str);
                return;
            };

            let action =
                Action::LineEditor(crate::actions::line_editor::LineEditorAction::AddGroup {
                    item_id,
                });

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_line_editor_delete_selected_items_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_delete_selected_items(move || {
            let action =
                Action::LineEditor(crate::actions::line_editor::LineEditorAction::DeleteItems);

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_line_editor_clipboard_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();

    {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        instance
            .global::<LineEditorCallbacks>()
            .on_copy_items(move || {
                let action = Action::LineEditor(LineEditorAction::CopyItems);
                dispatch_and_render(&s, action, &w);
            });
    }

    {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        instance
            .global::<LineEditorCallbacks>()
            .on_paste_items(move |anchor_item_id_str| {
                use aweft_model::ids::ItemId;
                let Ok(anchor) = uuid::Uuid::parse_str(anchor_item_id_str.as_str()).map(ItemId)
                else {
                    warn!("无效的 paste target item_id: {}", anchor_item_id_str);
                    return;
                };
                let action = Action::LineEditor(LineEditorAction::PasteItems { anchor });
                dispatch_and_render(&s, action, &w);
            });
    }

    {
        let s = Rc::clone(&store);
        let w = weak_instance;
        instance
            .global::<LineEditorCallbacks>()
            .on_cut_items(move || {
                let action = Action::LineEditor(LineEditorAction::CutItems);
                dispatch_and_render(&s, action, &w);
            });
    }
}

fn setup_line_editor_ungroup_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_ungroups(move |group_item_id_str| {
            let Ok(group_item_id) = uuid::Uuid::parse_str(group_item_id_str.as_str()).map(ItemId)
            else {
                println!("[callback] ungroup 无效的 item_id: {}", group_item_id_str);
                return;
            };

            let action =
                Action::LineEditor(crate::actions::line_editor::LineEditorAction::UnGroup {
                    group_item_id,
                });

            dispatch_and_render(&store, action, &weak_instance);
        });
}

/// 统一的 dispatch 和渲染逻辑 - 使用新的 StateChange 机制
fn dispatch_and_render(
    store: &Rc<RefCell<AppStore>>,
    action: Action,
    weak_instance: &slint::Weak<MainWindow>,
) {
    let mut store_mut = store.borrow_mut();

    // 1. 分发 Action，获取状态变更
    let state_change = match store_mut.dispatch(action) {
        Ok(change) => change,
        Err(e) => {
            log::error!("❌ Action 处理失败: {}", e);
            return;
        }
    };

    // 2. 转换为 UI 更新指令
    let ui_commands = UIUpdateCommand::from_state_change(state_change);

    // 3. 合并优化指令
    let optimized_commands = UIUpdateCommand::merge(ui_commands);

    // 4. 如果有需要更新的内容，执行 UI 更新
    if !optimized_commands.is_empty() {
        if let Some(instance) = weak_instance.upgrade() {
            UIUpdater::execute(&instance, &mut store_mut, optimized_commands);
        }
    }
}
