//! SlintWGPUSink：视频 sink 的核心数据结构与生命周期。
//! - `connect`：把 GPU 句柄、总线消息通道和"帧到达"通知接到 appsink 回调
//! - `fetch_next_frame`：渲染线程逐帧拉取 → 转 RGBA（细节见 `frame`）
//! - `after_rendering`：Slint 每帧提交后归还 RGBA 输出槽位
//! 帧字节到位图的具体处理（dmabuf/sysmem、上传、转色、槽位池）在 `frame`。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::UnboundedSender;
use gstreamer::{Bus, Element, Fraction, Message, Sample};
use gstreamer::{BusSyncReply, prelude::*};
use gstreamer_app::{AppSink, AppSinkCallbacks};
use gstreamer_video::VideoMeta;
use slint::wgpu_30::wgpu;

use super::convert::YuvConvertPipeline;
use super::dmabuf::DmabufImporter;
use super::frame::{CacheEpoch, CachedPlane, PlaneCacheKey};
use crate::render::output_pool::OutputSlotPool;

/// 连接视频 Sink 所需的 GPU 句柄：slint::GraphicsAPI 仅在渲染通知回调期间
/// 有效，RenderingSetup 时克隆缓存，之后可随时按需连接（如打开视频时）。
pub type WgpuHandles = (
    std::sync::Arc<wgpu::Instance>,
    std::sync::Arc<wgpu::Device>,
    std::sync::Arc<wgpu::Queue>,
);

pub struct SlintWGPUSink {
    pub(crate) appsink: AppSink,
    // GStreamer 回调始终只保留最新一帧；渲染线程慢于解码线程时覆盖旧帧，
    // 避免积压和追帧。Sample 同时覆盖 sysmem 与 dmabuf。
    pub(crate) next_frame: Arc<Mutex<Option<Sample>>>,
    pub(crate) wgpu_device: Option<Arc<wgpu::Device>>,
    pub(crate) wgpu_queue: Option<Arc<wgpu::Queue>>,
    // 系统内存/mmap 上传路径的平面纹理与视图缓存（按 平面+尺寸+格式 复用，渲染线程访问）
    pub(crate) plane_cache: HashMap<PlaneCacheKey, CachedPlane>,
    // YUV→RGB 离屏管线（connect 时构建）
    pub(crate) convert: Option<YuvConvertPipeline>,
    // 固定槽位的 RGBA 输出纹理池。槽位在 AfterRendering 后才归还，避免
    // Slint 仍在采样上一帧时被下一帧覆盖。池与字幕输出共用同一份实现。
    pub(crate) output_pool: OutputSlotPool,
    // dmabuf 零拷贝导入器（connect 时构建；非 Vulkan 渲染器/扩展缺失为 None）
    pub(crate) dmabuf: Option<DmabufImporter>,
    // 上一帧的 dmabuf 缓存代际（caps 宽高/格式），池重建时据此清空导入缓存
    // （见 `CacheEpoch`）。渲染线程访问。
    pub(crate) last_cache_epoch: Option<CacheEpoch>,
    // 视频帧时长（秒，由 caps 的 fps 计算；未知/可变帧率为 None）。
    // GStreamer 线程写、渲染线程读（Arc 跨 'static 回调共享）。
    pub(crate) frame_dur_secs: Arc<Mutex<Option<f64>>>,
    // 最近一帧的实际视频分辨率（宽×高），供 UI 调整显示尺寸。
    pub(crate) last_video_size: Option<(u32, u32)>,
    // 分辨率发生变化（含首次从 None→Some），供 fetch_next_frame 检测。
    pub(crate) video_size_changed: bool,
}

impl SlintWGPUSink {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            appsink: AppSink::builder()
                .enable_last_sample(false)
                // 与 main 保持一致：按媒体时钟交付，队列只保留最新帧。
                // UI 跟不上时丢弃过期帧，不能让解码器等待 UI 逐帧追赶。
                .max_buffers(1)
                .drop(true)
                .build(),
            next_frame: Default::default(),
            wgpu_device: None,
            wgpu_queue: None,
            plane_cache: HashMap::new(),
            convert: None,
            output_pool: OutputSlotPool::new("video-rgba-out"),
            dmabuf: None,
            last_cache_epoch: None,
            frame_dur_secs: Default::default(),
            last_video_size: None,
            video_size_changed: false,
        }
    }

    pub fn element(&self) -> Element {
        self.appsink.clone().upcast()
    }

    pub fn connect(
        &mut self,
        handles: WgpuHandles,
        bus: &Bus,
        bus_sender: UnboundedSender<Message>,
        on_frame_arrived: Box<dyn Fn() + Send + Sync>,
    ) -> anyhow::Result<()> {
        let (_, device, queue) = handles;
        let convert = YuvConvertPipeline::new(&device);
        let dmabuf = DmabufImporter::try_new(&device);

        self.wgpu_device = Some(device.clone());
        self.wgpu_queue = Some(queue.clone());
        self.convert = Some(convert);
        self.dmabuf = dmabuf;
        self.last_cache_epoch = None;
        self.output_pool.clear();

        // 设置总线消息处理器
        bus.set_sync_handler({
            move |_, msg| {
                let _ = bus_sender.unbounded_send(msg.to_owned());
                BusSyncReply::Drop
            }
        });

        self.appsink
            .set_callbacks(Self::build_gst_appsink_callbacks(
                self.next_frame.clone(),
                self.frame_dur_secs.clone(),
                Some(Arc::from(on_frame_arrived)),
            ));
        Ok(())
    }

    /// GStreamer appsink 回调集合。播放帧和暂停态 preroll 都在回调线程中取出，
    /// 覆盖 `next_frame` 的旧值并通知 Slint；渲染线程永远只消费最新帧。
    /// DMA-BUF 分配必须请求 VideoMeta，以获得每个 plane 的 stride/offset。
    /// sysmem 不请求它：部分 GStreamer 分配器会附加与实际 payload 不匹配的
    /// 对齐 stride，导致紧凑 Y410 buffer 被错误解释。
    pub(crate) fn build_gst_appsink_callbacks(
        next_frame: Arc<Mutex<Option<Sample>>>,
        frame_dur_secs: Arc<Mutex<Option<f64>>>,
        on_frame_arrived: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> AppSinkCallbacks {
        let mut builder = AppSinkCallbacks::builder().propose_allocation(|_sink, query| {
            // VideoInfo 描述 caps 的紧凑逻辑布局，VideoMeta 描述具体 buffer 的实际
            // plane 布局。DMA-BUF 的 VA surface 需要后者才能导入真实 stride/offset；
            // 但当前 GStreamer VA sysmem pool 可能写入对齐后的 VideoMeta stride，
            // 同时仍只分配紧凑 payload，导致 metadata 与 buffer 大小不一致。因此
            // sysmem 不请求 VideoMeta，使用 VideoInfo；DMA-BUF 才请求 VideoMeta。
            let wants_dmabuf = query.caps().is_some_and(|caps| {
                caps.iter_with_features()
                    .any(|(_, features)| features.contains("memory:DMABuf"))
            });
            if wants_dmabuf {
                query.add_allocation_meta::<VideoMeta>(None);
            }
            true
        });
        if let Some(frame_arrived) = on_frame_arrived {
            let sample_slot = Arc::clone(&next_frame);
            let frame_duration = Arc::clone(&frame_dur_secs);
            let preroll_notify = Arc::clone(&frame_arrived);
            let preroll_slot = Arc::clone(&next_frame);
            builder = builder
                .new_sample(move |appsink| {
                    let sample = appsink
                        .pull_sample()
                        .map_err(|_| gstreamer::FlowError::Flushing)?;
                    update_frame_duration(&sample, &frame_duration);
                    store_latest_sample(&sample_slot, sample, frame_arrived.as_ref());
                    Ok(gstreamer::FlowSuccess::Ok)
                })
                .new_preroll(move |appsink| {
                    let sample = appsink
                        .pull_preroll()
                        .map_err(|_| gstreamer::FlowError::Flushing)?;
                    update_frame_duration(&sample, &frame_dur_secs);
                    store_latest_sample(&preroll_slot, sample, preroll_notify.as_ref());
                    Ok(gstreamer::FlowSuccess::Ok)
                });
        }
        builder.build()
    }

    /// 视频帧时长（秒）：编辑器播放头按帧量化跳动用；未知为 None。
    pub fn frame_duration_secs(&self) -> Option<f64> {
        *self.frame_dur_secs.lock().ok()?
    }

    /// 拉取最新帧并转为 RGBA slint::Image。
    /// 返回 `(image, width, height)`——尺寸仅在分辨率变化时返回（Some），
    /// 否则为 None，避免 UI 侧每帧重复设置 Slint 属性。
    pub fn fetch_next_frame(&mut self) -> Option<(slint::Image, Option<u32>, Option<u32>)> {
        let device = self.wgpu_device.clone()?;
        let queue = self.wgpu_queue.clone()?;
        let sample = self.next_frame.try_lock().ok()?.take()?;

        // 帧到位图转换（dmabuf 导入 / sysmem 上传 + 离屏转色）在 frame 模块
        let image = self.sample_to_image(&device, &queue, sample)?;
        if self.video_size_changed {
            self.video_size_changed = false;
            let (w, h) = self.last_video_size?;
            Some((image, Some(w), Some(h)))
        } else {
            Some((image, None, None))
        }
    }

    /// 在 Slint 完成本轮场景提交后调用。此时 queue 上已经包含 Slint 对
    /// 当前输出纹理的采样命令，槽位只能在这之后等待 GPU 完成再复用。
    pub fn after_rendering(&mut self) {
        self.output_pool.after_rendering(self.wgpu_queue.as_deref());
    }

    /// 清理渲染器销毁时不能继续使用的 GPU 资源。
    pub fn teardown(&mut self) {
        if let Ok(mut next_frame) = self.next_frame.try_lock() {
            next_frame.take();
        }
        self.wgpu_device = None;
        self.wgpu_queue = None;
        self.plane_cache.clear();
        self.convert = None;
        self.output_pool.clear();
        self.dmabuf = None;
        self.last_cache_epoch = None;
        self.last_video_size = None;
        self.video_size_changed = false;
    }
}

fn update_frame_duration(sample: &Sample, frame_dur_secs: &Mutex<Option<f64>>) {
    let Some(fps) = sample
        .caps()
        .and_then(|caps| caps.structure(0))
        .and_then(|structure| structure.get::<Fraction>("framerate").ok())
    else {
        return;
    };
    if let Ok(mut frame_duration) = frame_dur_secs.lock() {
        *frame_duration = (fps.numer() > 0).then(|| fps.denom() as f64 / fps.numer() as f64);
    }
}

pub(super) fn store_latest_sample(slot: &Mutex<Option<Sample>>, sample: Sample, notify: &dyn Fn()) {
    let Ok(mut latest) = slot.try_lock() else {
        return;
    };
    *latest = Some(sample);
    drop(latest);
    notify();
}
