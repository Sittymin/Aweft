use crate::actions::Action;
use crate::bridge::types::MainWindow;

use crate::render::subtitle::Composition;
use crate::store::AppStore;
// winit 事件只在桌面后端存在：Android 特性集里 winit 后端被彻底排除
//（见 aweft-gui/Cargo.toml），slint::winit_030 模块在该目标上不存在。
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
                if let Some(subtitle) = store_mut.state_mut().subtitle.as_mut() {
                    subtitle.after_rendering();
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
    if let Some(subtitle) = store.borrow_mut().state_mut().subtitle.as_mut() {
        subtitle.teardown();
    }
}

/// 逐帧字幕出图：与视频帧、播放时钟同属实时例外，直接写 Slint 图像属性，
/// 不经 reducer/depsgraph（脚本本身由 `projections::subtitle` 在用户操作时编好）。
///
/// 时间取动画域当前时间（播放头显示值）；排版基准取预览帧尺寸（主视频显示像素尺寸，
/// 无主视频时兜底 1080P），脚本里的事件时间、PlayRes 与覆盖层贴的矩形同一基准。
/// 光栅尺寸另取覆盖层在屏幕上的物理像素数（视口报来的逻辑尺寸 × 窗口缩放），
/// 字幕纹理因此与屏幕 1:1，放大画面时不会让 Slint 替我们做插值放大。
fn render_subtitle_frame(instance: &MainWindow, store: &mut AppStore) {
    let layout_size = store.state().preview_frame_size();
    let Some(pipeline) = store.state_mut().subtitle.as_mut() else {
        return;
    };

    let subtitle_state = instance.global::<crate::bridge::types::SubtitleState>();
    let scale_factor = instance.window().scale_factor() as f32;
    let on_screen_physical = (
        subtitle_state.get_overlay_width() * scale_factor,
        subtitle_state.get_overlay_height() * scale_factor,
    );

    let now_ms = (instance
        .global::<crate::bridge::types::AnimationState>()
        .get_current_time() as f64
        * 1000.0) as i64;
    let composition = match pipeline.render_at(now_ms, layout_size, on_screen_physical) {
        Ok(composition) => composition,
        Err(error) => {
            log::warn!("字幕出帧失败: {error}");
            return;
        }
    };

    match composition {
        Composition::Frame(image) => subtitle_state.set_current_subtitle_frame(image),
        // 当前时刻没有字幕：交回默认空图，覆盖层因此不可见
        Composition::Empty => subtitle_state.set_current_subtitle_frame(slint::Image::default()),
        // 输出槽位全忙或画面未变：保持上一帧，一个属性都不写
        Composition::Skipped => {}
    }

    // 本帧还原出的行框交给视口：选中行的包围框照它画，点击与按住的命中也查它。
    // 出口只写变了的行，所以逐帧调用不会自己把重绘循环转起来。
    crate::projections::ui_sync_subtitle_boxes(store);
}

/// 逐帧视频通路：拉取各会话最新帧、同步播放位置、校正 transport 标量。
///
/// 新 sample/preroll 回调是播放渲染的唯一驱动；其它窗口重绘不查询媒体管线。
fn render_video_frames(store: &mut AppStore) {
    // 逐个会话拉取最新帧；帧消费与对应 UI 行立即配对，不创建临时帧数组。
    let session_count = store.state().video_sessions.len();
    let mut received_frame = false;
    for index in 0..session_count {
        let frame = store.state_mut().video_sessions[index]
            .wgpu_sink
            .fetch_next_frame();
        if let Some((image, width, height)) = frame {
            received_frame = true;
            store.ui_models.video.frames.set_row_data(index, image);
            if let (Some(w), Some(h)) = (width, height) {
                store.ui_models.video.widths.set_row_data(index, w as i32);
                store.ui_models.video.heights.set_row_data(index, h as i32);
            }
            render_sync_video_playback_time(store, index);
        }
    }

    if received_frame {
        crate::projections::ui_sync_active_video_time_realtime(store);
    }
    // Bus 在 EOS 时置一次脏标记并请求重绘。普通窗口重绘不遍历 transport 目标。
    let playback_dirty = store
        .state()
        .video_playback_dirty
        .swap(false, std::sync::atomic::Ordering::AcqRel);
    if playback_dirty && crate::reducers::media::media_refresh_video_playing_state(store) {
        crate::projections::ui_sync_video_playback_scalar(store);
    }
}

fn handle_before_rendering(instance: &MainWindow, store: &Rc<RefCell<AppStore>>) {
    let mut store_mut = store.borrow_mut();

    // 没有视频会话就不碰媒体管线；字幕照常出图
    if !store_mut.state().video_sessions.is_empty() {
        render_video_frames(&mut store_mut);
    }

    // 动画域当前时间已随上面的帧同步写完，字幕按同一时刻出图
    render_subtitle_frame(instance, &mut store_mut);
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
    let action = Action::Screen(crate::actions::screen::ScreenAction::InitializeLayout {
        width: windows_size.width,
        height: windows_size.height,
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
            let handles = (
                std::sync::Arc::new(instance.clone()),
                std::sync::Arc::new(device.clone()),
                std::sync::Arc::new(queue.clone()),
            );
            store.borrow_mut().state_mut().video_wgpu = Some(handles.clone());
            // 字幕合成器与视频输出共用同一对 GPU 句柄
            store.borrow_mut().state_mut().subtitle =
                Some(crate::render::subtitle::SubtitlePipeline::new(&handles));
            // 通路建好之前打过的字幕 tag 不会再来了：按当前真相补编一次脚本
            crate::projections::subtitle_rebuild_script(&mut store.borrow_mut());
        }
        _ => log::warn!("非 WGPU 渲染器，视频输出不可用"),
    }
}
