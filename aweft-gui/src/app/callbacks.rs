use crate::actions::{Action, area::AreaAction};
use crate::bridge::types::{CallbackManager, MainWindow};
use crate::bridge::{SlintSubtitleRowType, parse_group_id};
use crate::store::AppStore;
use crate::store::reducers::media::connect_video_to_wgpu;
use crate::ui_update::{UIUpdateCommand, UIUpdater};
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
}

/// 设置区域类型切换回调 - 使用 Action 模式
fn setup_area_type_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();

    instance
        .global::<CallbackManager>()
        .on_switch_area_type(move |area_key, area_type| {
            println!(
                "收到切换区域类型请求: area_key={:?}, area_type={:?}",
                area_key, area_type
            );

            // 创建 Action
            let action = Action::Area(AreaAction::SwitchType {
                area_key,
                area_type,
            });

            // 分发 Action 并更新 UI
            dispatch_and_render(&store, action, &weak_instance);
        });
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

    instance.global::<CallbackManager>().on_actionzone_drag(
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
        .global::<CallbackManager>()
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
        .global::<CallbackManager>()
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
        .global::<CallbackManager>()
        .on_edge_drag_end(move |is_cancel| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::End { is_cancel },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
}

/// 设置分组折叠/展开回调
///
/// 当用户点击分组头的折叠按钮时：
/// 1. 切换 group.collapsed 状态
/// 2. 调用 refresh_filter() 触发 FilterModel 重新过滤
fn setup_toggle_group_collapsed_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    instance
        .global::<CallbackManager>()
        .on_toggle_group_collapsed(move |group_id_str| {
            let Ok(group_id) = parse_group_id(&group_id_str) else {
                warn!("无效的 group_id: {}", group_id_str);
                return;
            };

            let mut store_mut = store.borrow_mut();
            let line_editor_state = &mut store_mut.state_mut().line_editor_state;

            // Rust 侧修改
            if let Some(group) = line_editor_state.group_manager.groups.get_mut(&group_id) {
                group.collapsed = !group.collapsed;
            }

            // 触发 FilterModel 重新过滤
            // Sline 侧修改
            line_editor_state.toggle_group(group_id);
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
            eprintln!("❌ Action 处理失败: {}", e);
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
