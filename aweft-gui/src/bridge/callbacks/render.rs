use crate::actions::Action;
use crate::bridge::types::MainWindow;

use crate::store::AppStore;
use slint::{ComponentHandle, Model, RenderingState};
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

pub(super) fn setup_rendering_notifier_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
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
            RenderingState::AfterRendering => {
                let mut store_mut = store.borrow_mut();
                for session in store_mut.state_mut().video_sessions.iter_mut() {
                    session.wgpu_sink.after_rendering();
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
    crate::render::video_sink::teardown(&mut store.borrow_mut());
}

fn handle_before_rendering(_instance: &MainWindow, store: &Rc<RefCell<AppStore>>) {
    let mut store_mut = store.borrow_mut();

    // 未打开视频：高频路径零开销返回（不查位置、不碰 Sink）
    if store_mut.state().video_sessions.is_empty() {
        return;
    }

    // 逐个会话拉取最新帧；帧消费与对应 UI 行立即配对，不创建临时帧数组。
    let session_count = store_mut.state().video_sessions.len();
    let mut received_frame = false;
    for index in 0..session_count {
        let frame = store_mut.state_mut().video_sessions[index]
            .wgpu_sink
            .fetch_next_frame();
        if let Some((image, width, height)) = frame {
            received_frame = true;
            store_mut.ui_models.video.frames.set_row_data(index, image);
            if let (Some(w), Some(h)) = (width, height) {
                store_mut
                    .ui_models
                    .video
                    .widths
                    .set_row_data(index, w as i32);
                store_mut
                    .ui_models
                    .video
                    .heights
                    .set_row_data(index, h as i32);
            }
            render_sync_video_playback_time(&mut store_mut, index);
        }
    }

    // 新 sample/preroll 回调是播放渲染的唯一驱动；其它窗口重绘不查询媒体管线。
    if received_frame {
        crate::projections::ui_sync_active_video_time_realtime(&mut store_mut);
    }
    // Bus 在 EOS 时置一次脏标记并请求重绘。普通窗口重绘不遍历 transport 目标。
    let playback_dirty = store_mut
        .state()
        .video_playback_dirty
        .swap(false, std::sync::atomic::Ordering::AcqRel);
    if playback_dirty && crate::reducers::media::media_refresh_video_playing_state(&mut store_mut) {
        crate::projections::ui_sync_video_playback_scalar(&store_mut);
    }
}

/// 播放中仅同步本次确有新帧的会话位置，并局部刷新其时间条模型。活跃视频
/// （选中无则主视频）随后驱动动画域当前时间。高频路径与视频帧一样直接写：
/// 不经 dispatch/depsgraph，但复用只更新 current-time 的实时投影出口。
/// 时间按视频帧时长向下取整——播放头逐帧跳动而非逐查询抖动。
fn render_sync_video_playback_time(store: &mut AppStore, index: usize) {
    if index >= store.state().video_sessions.len() {
        return;
    }
    // 拖拽时间条时 current_time 由媒体 reducer 的 seek 写入（鼠标目标位置），这里不要以
    // 实际解码（关键帧）位置覆盖，否则游标在目标/关键帧间反复跳动。
    let dragged = store
        .state()
        .video_seek_session
        .as_ref()
        .is_some_and(|drag| drag.targets.iter().any(|target| target.index == index));
    let bars = store.ui_models.video.time_bars.clone();
    let session = &mut store.state.video_sessions[index];
    if session.duration.is_none() {
        session.duration =
            crate::render::video_sink::gst_query_pipeline_duration_secs(&session.pipeline);
        if let Some(duration) = session.duration {
            crate::projections::ui_update_video_time_bar_duration(&bars, index, duration);
        }
    }
    let Some((secs, frame_dur)) = crate::reducers::media::gst_session_playing_position(session)
    else {
        return;
    };
    let snapped = match frame_dur {
        Some(d) if d > f64::EPSILON => (secs / d).floor() * d,
        _ => secs,
    };
    if !dragged && (session.current_time - snapped).abs() >= 0.0005 {
        session.current_time = snapped;
        crate::projections::ui_update_video_time_bar_value(&bars, index, snapped);
    }
}

fn handle_rendering_setup(
    instance: &MainWindow,
    store: &Rc<RefCell<AppStore>>,
    graphics_api: &slint::GraphicsAPI,
) {
    log::info!("窗口初始化完成",);
    let windows_size = instance.window().size();
    // 现读一份 theme：store 里那份是建窗时读的，Android 上那一刻 scale_factor
    // 还是 1.0，真值要到本回调才有。
    let theme = crate::bridge::types::ThemeSettings::from_main_window(instance);
    let scale_factor = theme.get_scale_factor();
    // Slint 的 safe-area-insets 是逻辑像素，顶点坐标系是物理像素，走 theme 换算。
    let inset = {
        let px = |logical: f32| theme.to_physical_px(f64::from(logical)).round().max(0.0) as u32;
        crate::screen::primitives::Insets {
            left: px(instance.get_safe_inset_left()),
            top: px(instance.get_safe_inset_top()),
            right: px(instance.get_safe_inset_right()),
            bottom: px(instance.get_safe_inset_bottom()),
        }
    };
    log::info!(
        "窗口 {}x{}@{scale_factor:.2}，安全区（物理像素）左{} 上{} 右{} 下{}",
        windows_size.width,
        windows_size.height,
        inset.left,
        inset.top,
        inset.right,
        inset.bottom
    );
    let action = Action::Screen(crate::actions::screen::ScreenAction::InitializeLayout {
        width: windows_size.width,
        height: windows_size.height,
        scale_factor,
        inset,
    });
    dispatch_and_render(store, action, &instance.as_weak());

    // 缓存渲染器 GPU 句柄：GraphicsAPI 仅在本回调期间有效，克隆出的
    // Instance/Device/Queue 供之后按需连接视频 Sink（打开视频时，reducers/media）。
    match graphics_api {
        slint::GraphicsAPI::WGPU30 {
            instance,
            device,
            queue,
            ..
        } => {
            store.borrow_mut().state_mut().video_wgpu = Some((
                std::sync::Arc::new(instance.clone()),
                std::sync::Arc::new(device.clone()),
                std::sync::Arc::new(queue.clone()),
            ));
        }
        _ => log::warn!("非 WGPU 渲染器，视频输出不可用"),
    }
}
