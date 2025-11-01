use std::sync::{Arc, Mutex};

use futures::channel::mpsc::UnboundedSender;
use gstreamer::{Buffer, Bus, Element, Fraction, Message, Pipeline};
use gstreamer::{BusSyncReply, prelude::*};
use gstreamer_app::{AppSink, AppSinkCallbacks};
use gstreamer_video::{VideoCapsBuilder, VideoFormat, VideoInfo};
use slint::wgpu_27::wgpu;
use slint::wgpu_27::wgpu::util::DeviceExt;

pub fn create_sink(pipeline: &Pipeline) -> (Element, SlintWGPUSink) {
    let slint_sink = SlintWGPUSink::new();
    let sink_element = slint_sink.element();
    pipeline.set_property("video-sink", &sink_element);
    (sink_element, slint_sink)
}

pub struct SlintWGPUSink {
    appsink: AppSink,
    // 🚀 性能优化：只缓冲必要的数据，减少内存拷贝
    next_frame: Arc<Mutex<Option<(VideoInfo, Buffer)>>>,
    wgpu_device: Option<Arc<wgpu::Device>>,
    wgpu_queue: Option<Arc<wgpu::Queue>>,
}

impl SlintWGPUSink {
    pub fn new() -> Self {
        let appsink = AppSink::builder()
            .caps(
                &VideoCapsBuilder::new()
                    .format(VideoFormat::Rgba)
                    .field("pixel-aspect-ratio", Fraction::new(1, 1))
                    .build(),
            )
            .enable_last_sample(false)
            // 🚀 性能优化：设置为 0 缓冲，强制丢帧以获得最低延迟
            .max_buffers(0u32)
            .build();

        Self {
            appsink,
            next_frame: Default::default(),
            wgpu_device: None,
            wgpu_queue: None,
        }
    }

    pub fn element(&self) -> Element {
        self.appsink.clone().upcast()
    }

    pub fn connect(
        &mut self,
        graphics_api: &slint::GraphicsAPI<'_>,
        bus: &Bus,
        bus_sender: UnboundedSender<Message>,
        next_frame_available_notifier: Box<dyn Fn() + Send>,
    ) {
        // 获取 WGPU 设备和队列
        match graphics_api {
            slint::GraphicsAPI::WGPU27 { device, queue, .. } => {
                self.wgpu_device = Some(Arc::new(device.clone()));
                self.wgpu_queue = Some(Arc::new(queue.clone()));
            }
            _ => panic!("不支持的图形 API，需要 WGPU"),
        }

        // 设置总线消息处理器
        bus.set_sync_handler({
            move |_, msg| {
                let _ = bus_sender.unbounded_send(msg.to_owned());
                BusSyncReply::Drop
            }
        });

        let next_frame_ref = self.next_frame.clone();

        self.appsink.set_callbacks(
            AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink
                        .pull_sample()
                        .map_err(|_| gstreamer::FlowError::Flushing)?;
                    let buffer = sample.buffer_owned().unwrap();

                    let Some(info) = sample
                        .caps()
                        .and_then(|caps| VideoInfo::from_caps(caps).ok())
                    else {
                        eprintln!("获取到无效的 caps");
                        return Err(gstreamer::FlowError::NotNegotiated);
                    };

                    // 🚀 性能优化：使用 try_lock 避免阻塞 GStreamer 线程
                    if let Ok(mut next_frame) = next_frame_ref.try_lock() {
                        *next_frame = Some((info, buffer));
                        drop(next_frame); // 立即释放锁
                        next_frame_available_notifier();
                    } else {
                        // 如果无法获取锁，直接丢弃这一帧（优先性能）
                        println!("丢弃一帧以保持性能");
                    }

                    Ok(gstreamer::FlowSuccess::Ok)
                })
                .build(),
        );
    }

    pub fn fetch_next_frame(&self) -> Option<slint::Image> {
        let device = self.wgpu_device.as_ref()?;
        let queue = self.wgpu_queue.as_ref()?;

        // 🚀 性能优化：使用 try_lock 避免阻塞渲染线程
        if let Ok(mut next_frame_guard) = self.next_frame.try_lock() {
            if let Some((info, buffer)) = next_frame_guard.take() {
                drop(next_frame_guard); // 立即释放锁，减少锁定时间

                // println!(
                //     "处理新的视频帧: {}x{}, 格式: {:?}",
                //     info.width(),
                //     info.height(),
                //     info.format()
                // );

                if let Some(texture) =
                    self.create_wgpu_texture_from_buffer(device, queue, &info, &buffer)
                {
                    // 将 WGPU 纹理转换为 Slint Image
                    match slint::Image::try_from(texture) {
                        Ok(image) => {
                            return Some(image);
                        }
                        Err(e) => {
                            eprintln!("转换为 Slint Image 失败: {:?}", e);
                            return None;
                        }
                    }
                } else {
                    eprintln!("创建 WGPU 纹理失败");
                }
            }
        } else {
            // 如果无法获取锁，跳过这一帧（优先性能）
            println!("跳过一帧以保持渲染性能");
        }

        None
    }

    fn create_wgpu_texture_from_buffer(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        info: &VideoInfo,
        buffer: &Buffer,
    ) -> Option<wgpu::Texture> {
        if let Ok(buffer_map) = buffer.map_readable() {
            let data = buffer_map.as_slice();

            if data.is_empty() {
                eprintln!("Buffer 数据为空");
                return None;
            }

            // 🚀 最优性能：使用 create_texture_with_data 直接上传
            let texture = device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("GStreamer video frame"),
                    size: wgpu::Extent3d {
                        width: info.width(),
                        height: info.height(),
                        // 视频帧为单一层
                        depth_or_array_layers: 1,
                    },
                    // 不存在缩略图版本
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::MipMajor,
                data,
            );

            Some(texture)
        } else {
            eprintln!("无法映射 GStreamer buffer");
            None
        }
    }

    pub fn deactivate_and_pause(&self) {
        if let Ok(mut next_frame) = self.next_frame.try_lock() {
            next_frame.take();
        }
    }
}
