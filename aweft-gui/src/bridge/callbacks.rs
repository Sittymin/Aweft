use crate::actions::{Action, area::AreaAction};
use crate::bridge::types::{AreaCallbacks, DragCallbacks, LineEditorCallbacks, MainWindow};
use crate::bridge::{slint_area_key_to_key, slint_area_type_to_rust};
use crate::store::AppStore;
use crate::store::reducers::media::connect_video_to_wgpu;
use crate::ui_update::{UIUpdateCommand, UIUpdater};
use log::warn;
use slint::winit_030::winit::{self, event::WindowEvent};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, RenderingState};
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
    setup_line_editor_increment_timecode_callback(instance, Rc::clone(&store));
    setup_line_editor_decrement_timecode_callback(instance, Rc::clone(&store));
    setup_line_editor_drag_timecode_preview_callback(instance, Rc::clone(&store));
    setup_line_editor_drag_timecode_commit_callback(instance, Rc::clone(&store));
    setup_line_editor_drag_timecode_cancel_callback(instance, Rc::clone(&store));
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
    instance
        .global::<DragCallbacks>()
        .on_edge_drag(move |vert_keys, offset_x, offset_y| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::Update {
                    vert_keys,
                    offset_x,
                    offset_y,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
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
        move |event_id_str, is_start_time, text| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            println!("输入文本: {text}");
            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::InputTimecode {
                    event_id,
                    is_start_time,
                    text: text.to_string(),
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}
fn setup_line_editor_increment_timecode_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_increment_timecode(move |event_id_str, is_start_time| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::IncrementTimecode {
                    event_id,
                    is_start_time,
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}
fn setup_line_editor_decrement_timecode_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_decrement_timecode(move |event_id_str, is_start_time| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::DecrementTimecode {
                    event_id,
                    is_start_time,
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}
fn setup_line_editor_drag_timecode_preview_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_drag_timecode_preview(move |event_id_str, is_start_time, delta_x| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::DragTimecodePreview {
                    event_id,
                    is_start_time,
                    delta_x,
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}
fn setup_line_editor_drag_timecode_commit_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_drag_timecode_commit(move |event_id_str, is_start_time, delta_x| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::DragTimecodeCommit {
                    event_id,
                    is_start_time,
                    delta_x,
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}
fn setup_line_editor_drag_timecode_cancel_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<LineEditorCallbacks>()
        .on_drag_timecode_cancel(move |event_id_str, is_start_time| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::LineEditor(
                crate::actions::line_editor::LineEditorAction::DragTimecodeCancel {
                    event_id,
                    is_start_time,
                },
            );

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
    let ui_commands = UIUpdateCommand::from_state_change(&state_change);

    // 3. 合并优化指令
    let optimized_commands = UIUpdateCommand::merge(ui_commands);

    // 4. 如果有需要更新的内容，执行 UI 更新
    if !optimized_commands.is_empty() {
        if let Some(instance) = weak_instance.upgrade() {
            UIUpdater::execute(&instance, &store_mut, &optimized_commands);
        }
    }
}
