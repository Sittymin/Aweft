use crate::actions::Action;
use crate::bridge::types::MainWindow;

use crate::store::AppStore;
use slint::ComponentHandle;
#[cfg(not(target_os = "android"))]
use slint::winit_030::winit::{self, event::WindowEvent};
#[cfg(not(target_os = "android"))]
use slint::winit_030::{EventResult, WinitWindowAccessor};
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

#[cfg(not(target_os = "android"))]
pub(super) fn setup_winit_window_event_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
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

#[cfg(not(target_os = "android"))]
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

#[cfg(not(target_os = "android"))]
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
