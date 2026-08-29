//! SlintWGPUSink：视频 sink 的核心数据结构与生命周期。
//! - `connect`：把 GPU 句柄、总线消息通道和"帧到达"通知接到 appsink 回调
//! - `fetch_next_frame`：渲染线程逐帧拉取 → 转 RGBA（细节见 `frame`）
//! - `after_rendering`：Slint 每帧提交后归还 RGBA 输出槽位
//! 帧字节到位图的具体处理（dmabuf/sysmem、上传、转色、槽位池）在 `frame`。

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::UnboundedSender;
use gstreamer::{Bus, Element, Fraction, Message, Sample};
use gstreamer::{BusSyncReply, prelude::*};
use gstreamer_app::{AppSink, AppSinkCallbacks};
use gstreamer_video::VideoMeta;
use slint::wgpu_30::wgpu;

use super::convert::YuvConvertPipeline;
use super::dmabuf::DmabufImporter;
use super::frame::{
    CacheEpoch, CachedPlane, OUTPUT_TEXTURE_POOL_SIZE, OutputTextureSlot, PlaneCacheKey,
};

/// 连接视频 Sink 所需的 GPU 句柄：slint::GraphicsAPI 仅在渲染通知回调期间
/// 有效，RenderingSetup 时克隆缓存，之后可随时按需连接（如打开视频时）。
pub type WgpuHandles = (
    std::sync::Arc<wgpu::Instance>,
    std::sync::Arc<wgpu::Device>,
    std::sync::Arc<wgpu::Queue>,
);

pub struct SlintWGPUSink {
    pub(crate) appsink: AppSink,
    // 最新帧：GStreamer 线程写、渲染线程取。dmabuf 帧不可 map，统一存 Sample
    // （自带 buffer + caps + 元数据），避免按格式分叉存储结构。
    pub(crate) next_frame: Arc<Mutex<Option<Sample>>>,
    pub(crate) wgpu_device: Option<Arc<wgpu::Device>>,
    pub(crate) wgpu_queue: Option<Arc<wgpu::Queue>>,
    // 系统内存/mmap 上传路径的平面纹理与视图缓存（按 平面+尺寸+格式 复用，渲染线程访问）
    pub(crate) plane_cache: HashMap<PlaneCacheKey, CachedPlane>,
    // YUV→RGB 离屏管线（connect 时构建）
    pub(crate) convert: Option<YuvConvertPipeline>,
    // 固定槽位的 RGBA 输出纹理池。槽位在 AfterRendering 后才归还，避免
    // Slint 仍在采样上一帧时被下一帧覆盖。
    pub(crate) output_pool: Vec<OutputTextureSlot>,
    pub(crate) active_output_slot: Option<usize>,
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
                // max-buffers=1 + drop：appsink 内部队列只保留最新帧，
                // 渲染线程跟不上时丢旧帧保延迟（不是缓冲上限）
                .max_buffers(1)
                .drop(true)
                .build(),
            next_frame: Default::default(),
            wgpu_device: None,
            wgpu_queue: None,
            plane_cache: HashMap::new(),
            convert: None,
            output_pool: Vec::new(),
            active_output_slot: None,
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
        on_frame_arrived: Box<dyn Fn() + Send>,
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
        self.active_output_slot = None;
        self.output_pool.reserve(OUTPUT_TEXTURE_POOL_SIZE);

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
                Some(on_frame_arrived),
            ));
        Ok(())
    }

    /// GStreamer appsink 回调集合。`on_frame_arrived` 为新帧就绪通知（None 供
    /// 测试/离线拉帧用，只保留 allocation 协商）。
    /// DMA-BUF 分配必须请求 VideoMeta，以获得每个 plane 的 stride/offset。
    /// sysmem 不请求它：部分 GStreamer 分配器会附加与实际 payload 不匹配的
    /// 对齐 stride，导致紧凑 Y410 buffer 被错误解释。
    pub(crate) fn build_gst_appsink_callbacks(
        next_frame: Arc<Mutex<Option<Sample>>>,
        frame_dur_secs: Arc<Mutex<Option<f64>>>,
        on_frame_arrived: Option<Box<dyn Fn() + Send>>,
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
            builder = builder.new_sample(move |appsink| {
                let sample = appsink
                    .pull_sample()
                    .map_err(|_| gstreamer::FlowError::Flushing)?;

                // 记录帧时长（编辑器播放头按帧跳动用）；fps 无效（可变帧率）为 None
                if let Some(fps) = sample
                    .caps()
                    .and_then(|c| c.structure(0))
                    .and_then(|s| s.get::<Fraction>("framerate").ok())
                {
                    if let Ok(mut frame_dur) = frame_dur_secs.lock() {
                        *frame_dur = if fps.numer() > 0 {
                            Some(fps.denom() as f64 / fps.numer() as f64)
                        } else {
                            None
                        };
                    }
                }

                // try_lock 避免阻塞 GStreamer 线程；锁被渲染线程占用则丢弃本帧
                if let Ok(mut next_frame) = next_frame.try_lock() {
                    *next_frame = Some(sample);
                    drop(next_frame); // 立即释放锁
                    frame_arrived();
                } else {
                    // 锁竞争时丢帧是正常的背压行为；不要按帧写 debug 日志。
                    log::trace!("丢弃一帧以保持性能");
                }

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

        // try_lock + 立即取出，避免长时间持有锁阻塞 GStreamer 线程
        let sample = match self.next_frame.try_lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => {
                // 渲染线程暂时持锁时跳过一帧是正常的背压行为。
                log::trace!("跳过一帧以保持渲染性能");
                return None;
            }
        }?;

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
        let Some(index) = self.active_output_slot.take() else {
            return;
        };
        let Some(slot) = self.output_pool.get(index) else {
            return;
        };
        let in_flight = slot.in_flight.clone();
        let Some(queue) = self.wgpu_queue.clone() else {
            in_flight.store(false, Ordering::Release);
            return;
        };
        queue.on_submitted_work_done(move || {
            in_flight.store(false, Ordering::Release);
        });
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
        self.active_output_slot = None;
        self.dmabuf = None;
        self.last_cache_epoch = None;
        self.last_video_size = None;
        self.video_size_changed = false;
    }
}
