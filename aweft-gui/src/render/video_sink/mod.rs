//! 视频播放：appsink 硬解管线 + dmabuf 零拷贝导入（或 sysmem 上传回退）+
//! wgpu 离屏 YUV→RGB + slint::Image。
//! ```text
//! urisourcebin → parsebin → 视频 decodebin(硬解) → vapostproc(GPU 转换) → appsink(dmabuf)
//!   → 每 plane fd → Vulkan 导入(VK_EXT_external_memory_dma_buf) → yuv_convert
//!   → slint::Image(零拷贝)
//! ```
//! 会话内的实时音频分支与视频同管线同源（见 `audio`），音轨经 input-selector
//! 选流后走独立的 decodebin → 尾段，与视频共享管线时钟。
//! dmabuf 不可用（无 VA / 非 Vulkan 后端 / stride 不匹配）时协商自动落
//! sysmem NV12 或 mmap 上传，路径选择见 `frame` 与 `dmabuf`。
//! （Wayland 子 surface 直通方案因跨 Rust/C 两套 libwayland 连接不可行而移除，
//! 详见 git 历史。）
//!
//! # 模块职责
//! - `pipeline`：GStreamer 管线搭建与格式协商
//! - `sink`：`SlintWGPUSink` 数据结构与生命周期（连接、逐帧回调、取帧、槽位归还）
//! - `frame`：一帧 Sample → RGBA 位图（dmabuf/sysmem 平面、上传、输出槽位池）
//! - `convert`：YUV→RGB 离屏转色管线
//! - `dmabuf`：DMA-BUF 零拷贝导入器
//! 本文件是入口：会话的初始化 / 停止 / 连接与总线回路。
//!
//! # RGBA 直出方案调研结论（2026-08，已否决）
//! 曾评估让 vapostproc 直出 AB24（ABGR8888 DMABuf）以省去 WGSL 转色：
//! caps 协商、VA 导出、Slint `Image::try_from`（要求 Rgba8Unorm +
//! TEXTURE_BINDING|RENDER_ATTACHMENT）均可行，但导入被硬件拒绝——Arc DG2
//! 渲染引擎不支持采样 legacy X-TILED，而 iHD 恰只把 RGBA 表面导出为
//! X-TILED；ANV 对 R8G8B8A8_UNORM 仅列 Linear/Tile4 家族 modifier。
//! YU12 裸 fourcc（= 线性）是双方唯一交集，故现有「线性 YU12 零拷贝 +
//! WGSL 转色」就是本机硬件上的最优形态。能力探针见
//! `tests::test_dmabuf_rgba_ab24_import_spike`，驱动升级后可复验。

pub(crate) mod audio;
pub(crate) mod convert;
pub(crate) mod dmabuf;
pub(crate) mod frame;
pub(crate) mod pipeline;
pub(crate) mod sink;
pub(crate) mod waveform;

#[cfg(test)]
#[path = "tests.rs"]
pub(crate) mod tests;

pub use sink::{SlintWGPUSink, WgpuHandles};

use crate::store::AppStore;
use crate::store::state::VideoSession;
use anyhow::Result;
use gstreamer::prelude::*;
use slint::ComponentHandle as _;

/// 构建并连接一个视频会话（管线 + sink + GPU 输出 + 启动播放）。
/// 失败时完整回滚（已连接的 sink/管线全部释放），且**不修改 store 状态**——
/// 由 reducer 决定是追加会话还是原地替换（替换在连接成功后才拆旧会话）。
pub(crate) fn build_session(store: &mut AppStore, uri: &str) -> Result<VideoSession> {
    // P010（10bit）平面需要 R16Unorm/Rg16Unorm 纹理，设备未启用
    // TEXTURE_FORMAT_16BIT_NORM 时必须从 caps 排除（否则纹理创建 panic）
    let ten_bit = store
        .state()
        .video_wgpu
        .as_ref()
        .is_some_and(|(_, device, _)| {
            use slint::wgpu_30::wgpu;
            device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
        });
    // dmabuf 零拷贝需要设备启用 VULKAN_EXTERNAL_MEMORY_DMA_BUF（与上面的
    // ten_bit 一样，`device.features()` 返回的是创建时请求的特性集）
    let dmabuf = store
        .state()
        .video_wgpu
        .as_ref()
        .is_some_and(|(_, device, _)| {
            use slint::wgpu_30::wgpu;
            device
                .features()
                .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
        });
    let (pipeline, sink_element, wgpu_sink, session_audio) =
        pipeline::build_decode_pipeline_with_dmabuf(uri, ten_bit, dmabuf)?;

    // 先完整连接候选会话；连接失败时回滚，避免留下半初始化状态。
    let mut wgpu_sink = wgpu_sink;
    let at_eos = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let playback_dirty = std::sync::Arc::clone(&store.state().video_playback_dirty);
    if let Err(error) = connect_pipeline_to_wgpu(
        store,
        &pipeline,
        &mut wgpu_sink,
        std::sync::Arc::clone(&at_eos),
        playback_dirty,
    ) {
        stop_session_parts(&pipeline, wgpu_sink);
        return Err(error);
    }
    let duration = gst_query_pipeline_duration_secs(&pipeline);

    Ok(VideoSession {
        uri: uri.to_string(),
        pipeline,
        sink_element,
        wgpu_sink,
        audio: Some(session_audio),
        position_x: 0.0,
        position_y: 0.0,
        current_time: 0.0,
        duration,
        muted: false,
        playing: false,
        at_eos,
        last_frame_seeked: false,
    })
}

/// 查询媒体管线已发布的总时长（秒）。暂停态预滚动完成前可能暂时不可用，
/// 调用方应在首个 preroll/sample 到达后重试，而不是把缺失值固化为 0。
pub(crate) fn gst_query_pipeline_duration_secs(pipeline: &gstreamer::Pipeline) -> Option<f64> {
    let duration = pipeline.query_duration::<gstreamer::ClockTime>()?;
    let secs = duration.nseconds() as f64 / 1_000_000_000.0;
    (secs.is_finite() && secs > 0.0).then_some(secs)
}

/// 连接 WGPU 输出并启动播放（帧到达 → 请求 Slint 重绘）。
/// 把管线、GPU 句柄、总线消息与"帧到达通知"接线，随后置 Playing。
fn connect_pipeline_to_wgpu(
    store: &AppStore,
    pipeline: &gstreamer::Pipeline,
    wgpu_sink: &mut sink::SlintWGPUSink,
    at_eos: std::sync::Arc<std::sync::atomic::AtomicBool>,
    playback_dirty: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    use anyhow::Context as _;
    use futures::StreamExt;
    use futures::channel::mpsc;

    let Some(handles) = store.state().video_wgpu.clone() else {
        anyhow::bail!("WGPU 设备未缓存（渲染器尚未初始化）");
    };

    let bus = pipeline.bus().context("无法获取 GStreamer Bus")?;
    let (bus_sender, mut bus_receiver) = mpsc::unbounded::<gstreamer::Message>();

    // 帧到达 → 投递到 Slint 主线程请求重绘（winit 窗口不可跨线程直接操作）
    let Some(weak_instance) = store.main_window.clone() else {
        anyhow::bail!("主窗口未初始化");
    };
    let bus_window = weak_instance.clone();
    let next_frame_notifier: Box<dyn Fn() + Send + Sync> = Box::new(move || {
        let _ = weak_instance.upgrade_in_event_loop(|instance| {
            // slint::Window 原生支持请求重绘（跨线程安全入口已在 upgrade 内）
            instance.window().request_redraw();
        });
    });

    wgpu_sink.connect(handles, &bus, bus_sender, next_frame_notifier)?;
    pipeline
        .set_state(gstreamer::State::Paused)
        .context("启动（暂停态预滚动）失败")?;

    if let Err(error) = slint::spawn_local(async move {
        while let Some(msg) = bus_receiver.next().await {
            use gstreamer::MessageView;
            match msg.view() {
                MessageView::Eos(..) => {
                    log::info!("视频播放结束");
                    at_eos.store(true, std::sync::atomic::Ordering::Release);
                    playback_dirty.store(true, std::sync::atomic::Ordering::Release);
                    let _ = bus_window.upgrade_in_event_loop(|instance| {
                        instance.window().request_redraw();
                    });
                }
                MessageView::Error(err) => {
                    log::error!(
                        "GStreamer 错误: {} (来自 {:?})",
                        err.error(),
                        err.src().map(|s| s.path_string())
                    );
                    break;
                }
                MessageView::Warning(warning) => log::warn!(
                    "GStreamer 警告: {} (来自 {:?})",
                    warning.error(),
                    warning.src().map(|s| s.path_string())
                ),
                _ => {}
            }
        }
    }) {
        log::warn!("启动视频总线监听失败: {error}");
    }

    log::info!("✅ 视频系统连接成功，开始播放");
    Ok(())
}

/// 停止单个会话的管线与 sink（替换/回滚时复用）。
pub(crate) fn stop_session_parts(
    pipeline: &gstreamer::Pipeline,
    mut wgpu_sink: sink::SlintWGPUSink,
) {
    if let Err(error) = pipeline.set_state(gstreamer::State::Null) {
        log::warn!("停止视频管线失败: {error}");
    }
    wgpu_sink.teardown();
}

/// 释放全部视频会话（退出前统一调用）。
pub(crate) fn teardown(store: &mut AppStore) {
    for cancellation in std::mem::take(&mut store.state_mut().waveform_cancellations) {
        cancellation.store(true, std::sync::atomic::Ordering::Release);
    }
    store.state_mut().waveform_ui_sender.take();
    let sessions = std::mem::take(&mut store.state_mut().video_sessions);
    store.state_mut().audio_track_indices.clear();
    store.state_mut().audio_tracks.clear();
    store.ui_models.audio.tracks.set_vec(Vec::new());
    store.ui_models.audio.track_models.clear();
    store.ui_models.audio.playback_tracks.set_vec(Vec::new());
    store.ui_models.audio.buckets.clear();
    store.ui_models.audio.views.clear();
    store.ui_models.audio.columns.clear();
    // 音频分支位于会话管线内（同源同钟），置 Null 时随会话一并释放，无需单独处理。
    for session in sessions {
        stop_session_parts(&session.pipeline, session.wgpu_sink);
    }
    store.state_mut().main_video = None;
    store.state_mut().selected_videos.clear();
    store.state_mut().video_playing = false;
    store.state_mut().video_seek_session = None;
}
