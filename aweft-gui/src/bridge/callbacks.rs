use crate::actions::sequence::SequenceAction;
use crate::actions::{Action, area::AreaAction};
use crate::bridge::types::{
    AnimationCallbacks, AreaCallbacks, DragCallbacks, MainWindow, NodeGraphCallbacks,
    SequenceCallbacks,
};
use crate::bridge::{parse_time, slint_area_key_to_key, slint_area_type_to_rust};
use crate::reducers::media::connect_video_to_wgpu;
use crate::store::AppStore;
use aweft_model::ids::ItemId;
use log::warn;
use slint::winit_030::winit::{self, event::WindowEvent};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, RenderingState};
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
    setup_sequence_timecode_editor_callback(instance, Rc::clone(&store));
    setup_sequence_timecode_step_callbacks(instance, Rc::clone(&store));
    setup_sequence_drag_timecode_callbacks(instance, Rc::clone(&store));
    setup_sequence_drag_timecode_cancel_callback(instance, Rc::clone(&store));
    setup_sequence_selection_callbacks(instance, Rc::clone(&store));
    setup_sequence_add_line_callback(instance, Rc::clone(&store));
    setup_sequence_add_group_callback(instance, Rc::clone(&store));
    setup_sequence_delete_selected_items_callback(instance, Rc::clone(&store));
    setup_sequence_clipboard_callbacks(instance, Rc::clone(&store));
    setup_sequence_ungroup_callbacks(instance, Rc::clone(&store));
    setup_node_graph_callbacks(instance, Rc::clone(&store));
    setup_animation_callbacks(instance, Rc::clone(&store));
    setup_nla_callbacks(instance, Rc::clone(&store));
}

/// 设置节点图编辑器回调：图操作走 `Action::NodeGraph` → dispatch → tag → 投影。
/// 只读查询（命中检测/找连线）保持同步调用（拖拽需要立即结果）。
fn setup_node_graph_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::actions::node_graph::NodeGraphAction;
    use crate::bridge::types::GraphType;
    use aweft_model::node_graph::NodeGraphKind;

    let graph_type_of = |slint_type: GraphType| -> NodeGraphKind {
        match slint_type {
            GraphType::StyleGraph => NodeGraphKind::Style,
            GraphType::AppGraph => NodeGraphKind::App,
        }
    };

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_at(move |graph_type, node_index, x, y| {
            s.borrow()
                .find_socket_at(graph_type_of(graph_type), node_index as usize, x, y)
        });

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_node_dragged(move |graph_type, node_index, x, y| {
            s.borrow_mut()
                .set_node_position(graph_type_of(graph_type), node_index as usize, x, y);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_add_node(
        move |graph_type, template_index, x, y| -> i32 {
            let action = Action::NodeGraph(NodeGraphAction::AddNode {
                graph_type: graph_type_of(graph_type),
                template_index: template_index as usize,
                x,
                y,
            });
            dispatch_and_render(&s, action, &weak);
            // 投影已 sync：新节点在 nodes 模型末尾
            let store_ref = s.borrow();
            let count = match graph_type {
                GraphType::StyleGraph => store_ref.ui_models().style_graph.nodes.row_count(),
                GraphType::AppGraph => store_ref.ui_models().app_graph.nodes.row_count(),
            };
            (count as i32) - 1
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_delete_node(move |graph_type, node_index| {
            let action = Action::NodeGraph(NodeGraphAction::DeleteNode {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_find_link_for_input(move |graph_type, node_index, socket_index| -> i32 {
            s.borrow().find_link_for_input(
                graph_type_of(graph_type),
                node_index as usize,
                socket_index as usize,
            )
        });

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_find_drop_socket(
        move |graph_type, x, y, data_type| -> crate::bridge::types::SocketHit {
            s.borrow()
                .find_drop_socket(graph_type_of(graph_type), x, y, data_type)
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_drag_ended(
            move |graph_type, node_index, socket_index, is_input, x, y| {
                let action = Action::NodeGraph(NodeGraphAction::SocketDragEnded {
                    graph_type: graph_type_of(graph_type),
                    node_index: node_index as usize,
                    socket_index: socket_index as usize,
                    is_input,
                    x,
                    y,
                });
                dispatch_and_render(&s, action, &weak);
            },
        );

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_load_chain(move |graph_type, index| {
            s.borrow_mut()
                .load_chain(graph_type_of(graph_type), index as usize);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_new_chain(move |graph_type| -> i32 {
            let action = Action::NodeGraph(NodeGraphAction::NewChain {
                graph_type: graph_type_of(graph_type),
            });
            dispatch_and_render(&s, action, &weak);
            // 投影已刷新链菜单：新链在菜单末尾
            let store_ref = s.borrow();
            let count = match graph_type {
                GraphType::StyleGraph => store_ref.ui_models().style_graph.chain_items.row_count(),
                GraphType::AppGraph => store_ref.ui_models().app_graph.chain_items.row_count(),
            };
            (count as i32) - 1
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_menu_selected(
        move |graph_type, node_index, menu_index| {
            let action = Action::NodeGraph(NodeGraphAction::MenuSelected {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                menu_index: menu_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_option_selected(move |graph_type, node_index, socket_row, option_index| {
            let action = Action::NodeGraph(NodeGraphAction::SocketOptionSelected {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                option_index: option_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_create_actor(
        move |graph_type, node_index, socket_row, name| {
            let action = Action::NodeGraph(NodeGraphAction::CreateActor {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                name: name.to_string(),
            });
            dispatch_and_render(&s, action, &weak);
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_input_changed(move |graph_type, node_index, socket_row, text| {
            let action = Action::NodeGraph(NodeGraphAction::SocketInputChanged {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                text: text.to_string(),
            });
            dispatch_and_render(&s, action, &weak);
        });
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
        log::error!("设置渲染通知器失败: {}", e);
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
        log::debug!("收到新视频帧，设置到 UI");
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
    log::info!("窗口初始化完成",);
    let windows_size = instance.window().size();
    let action = Action::Screen(crate::actions::screen::ScreenAction::InitializeLayout {
        width: windows_size.width,
        height: windows_size.height,
    });
    dispatch_and_render(store, action, &instance.as_weak());

    // HACK: 后续需要修改为按需执行
    if let Err(e) = connect_video_to_wgpu(store, instance, graphics_api) {
        log::error!("❌ 视频系统连接失败: {}", e);
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
    log::debug!("窗口大小发生改变: {}x{}", new_size.width, new_size.height);

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
    log::info!("检测到屏幕缩放因子变化: {:.2}", scale_factor);
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

fn setup_sequence_timecode_editor_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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
fn setup_sequence_timecode_step_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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

fn setup_sequence_drag_timecode_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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
fn setup_sequence_drag_timecode_cancel_callback(
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

fn setup_sequence_selection_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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
}

fn setup_sequence_add_line_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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

fn setup_sequence_add_group_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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

fn setup_sequence_delete_selected_items_callback(
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

fn setup_sequence_clipboard_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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

fn setup_sequence_ungroup_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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

/// 统一的 dispatch 和渲染逻辑。
///
/// reducer 修改数据并向依赖图打 tag；`evaluate_depsgraph` 只重算受影响的投影。
fn dispatch_and_render(
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

/// 设置动画编辑器（曲线编辑器）回调：全部交互走 `Action::Animation` → dispatch → tag → 投影。
fn setup_animation_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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

/// 设置 NLA 编辑器回调：片断/轨道交互走 `Action::Nla` → dispatch → tag → 投影。
/// 命中检测（`nla-strip-pick`）为同步查询（拖拽需要立即结果）+ 选中修改：
/// 数据层就地更新后手动打 tag 并 evaluate（与 dispatch 同管线）。
fn setup_nla_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
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
}
