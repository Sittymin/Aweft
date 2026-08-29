//! 视频播放：appsink 硬解管线 + dmabuf 零拷贝导入（或 sysmem 上传回退）+
//! wgpu 离屏 YUV→RGB + slint::Image。
//! ```text
//! uridecodebin(硬解) → vapostproc(GPU 444p10→P010/NV12 转换) → appsink(dmabuf)
//!   → 每 plane fd → Vulkan 导入(VK_EXT_external_memory_dma_buf) → yuv_convert
//!   → slint::Image(零拷贝)
//! ```
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

pub(crate) mod convert;
pub(crate) mod dmabuf;
pub(crate) mod frame;
pub(crate) mod pipeline;
pub(crate) mod sink;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use sink::{SlintWGPUSink, WgpuHandles};

use crate::store::AppStore;
use anyhow::Result;
use gstreamer::prelude::*;
use slint::ComponentHandle as _;

type VideoSessionParts = (
    Option<gstreamer::Pipeline>,
    Option<gstreamer::Element>,
    Option<sink::SlintWGPUSink>,
);

/// 初始化视频输出：搭建管线（硬解按 rank 择优，VA 在位走 dmabuf 零拷贝）→
/// 连接 GPU 输出 → 启动播放。
pub(crate) fn initialize(store: &mut AppStore, uri: &str) -> Result<()> {
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
    let (pipeline, sink_element, wgpu_sink) =
        pipeline::build_decode_pipeline_with_dmabuf(uri, ten_bit, dmabuf)?;

    // 先提交候选会话，连接失败时完整回滚，避免留下半初始化状态。
    let old_session = {
        let state = store.state_mut();
        let old = (
            state.video_pipeline.take(),
            state.video_sink.take(),
            state.wgpu_sink.take(),
        );
        state.video_pipeline = Some(pipeline);
        state.video_sink = Some(sink_element);
        state.wgpu_sink = Some(wgpu_sink);
        old
    };

    if let Err(error) = connect_video_to_wgpu_from_store(store) {
        stop_video_session(store);
        let state = store.state_mut();
        (state.video_pipeline, state.video_sink, state.wgpu_sink) = old_session;
        return Err(error);
    }

    // 新会话已成功启动，旧管线现在才安全地释放。
    stop_video_parts(old_session);
    Ok(())
}

/// 连接 WGPU 输出并启动播放（帧到达 → 请求 Slint 重绘）。
/// 把管线、GPU 句柄、总线消息与"帧到达通知"接线，随后置 Playing。
pub(crate) fn connect_video_to_wgpu_from_store(store: &mut AppStore) -> Result<()> {
    use anyhow::Context as _;
    use futures::StreamExt;
    use futures::channel::mpsc;

    let Some(pipeline) = store.state().video_pipeline.clone() else {
        anyhow::bail!("Pipeline 未初始化");
    };
    let Some(handles) = store.state().video_wgpu.clone() else {
        anyhow::bail!("WGPU 设备未缓存（渲染器尚未初始化）");
    };
    if store.state().wgpu_sink.is_none() {
        anyhow::bail!("视频 Sink 未初始化");
    }

    let bus = pipeline.bus().context("无法获取 GStreamer Bus")?;
    let (bus_sender, mut bus_receiver) = mpsc::unbounded::<gstreamer::Message>();

    // 帧到达 → 投递到 Slint 主线程请求重绘（winit 窗口不可跨线程直接操作）
    let Some(weak_instance) = store.main_window.clone() else {
        anyhow::bail!("主窗口未初始化");
    };
    let next_frame_notifier = Box::new(move || {
        let _ = weak_instance.upgrade_in_event_loop(|instance| {
            // slint::Window 原生支持请求重绘（跨线程安全入口已在 upgrade 内）
            instance.window().request_redraw();
        });
    });

    store
        .state_mut()
        .wgpu_sink
        .as_mut()
        .expect("视频 Sink 已在前置检查后被移除")
        .connect(handles, &bus, bus_sender, next_frame_notifier)?;

    pipeline
        .set_state(gstreamer::State::Playing)
        .context("启动播放失败")?;

    if let Err(error) = slint::spawn_local(async move {
        while let Some(msg) = bus_receiver.next().await {
            use gstreamer::MessageView;
            match msg.view() {
                MessageView::Eos(..) => {
                    log::info!("视频播放结束");
                    break;
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

fn stop_video_parts((pipeline, _sink_element, mut wgpu_sink): VideoSessionParts) {
    if let Some(pipeline) = pipeline {
        if let Err(error) = pipeline.set_state(gstreamer::State::Null) {
            log::warn!("停止旧视频管线失败: {error}");
        }
    }
    if let Some(sink) = wgpu_sink.as_mut() {
        sink.teardown();
    }
}

fn stop_video_session(store: &mut AppStore) {
    let state = store.state_mut();
    let session = (
        state.video_pipeline.take(),
        state.video_sink.take(),
        state.wgpu_sink.take(),
    );
    stop_video_parts(session);
}

pub(crate) fn teardown(store: &mut AppStore) {
    stop_video_session(store);
}
