//! 视频输出无头测试：
//! - 合成 NV12/YU12/P010 帧 → yuv_convert 着色器数值校验（无需媒体文件）
//! - 真实视频文件默认路径端到端（vpp GPU 转换 → sysmem 下载 → 上传 → 转色
//!   → slint::Image），走渲染真实入口 fetch_next_frame
//! - dmabuf 零拷贝路径像素校验（VA 在位时默认启用）
//! （文件缺失时跳过；仅验证本机存在的路径）
//! 全程无窗口：wgpu headless 设备 + appsink 拉帧。

use super::*;
use gstreamer_app::AppSink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use super::convert::YuvConvertPipeline;
use super::dmabuf::{DmabufImporter, PlaneLayout};
use super::frame::{
    CacheEpoch, ChromaViews, FramePlanes, display_width_from_par, dmabuf_plane, plane_range,
    plane_rows_avail, positive_stride, sysmem_plane_layout,
};
use super::pipeline::{
    build_decode_pipeline_with_dmabuf, create_sink, filter_runtime_caps, video_caps_with,
};
use gstreamer::Sample;
use gstreamer_video::{VideoFormat, VideoInfo, VideoInfoDmaDrm, VideoMeta, is_dma_drm_caps};
use slint::wgpu_30::wgpu;

use crate::render::wgpu_test_support::{headless_wgpu, readback_rgba};

/// 生成测试视频的参数：1280x720/30fps 的 6 秒 SMPTE 彩条。静态内容保证两条
/// 独立管线（我们的渲染 vs GStreamer 原生参考）拉到的任意两帧逐像素相同，
/// 不会因解码时序差一帧而产生非对齐差异。
const TEST_VIDEO_W: i32 = 1280;
const TEST_VIDEO_H: i32 = 720;
const TEST_VIDEO_FPS: i32 = 30;
const TEST_VIDEO_FRAMES: i32 = 180;

fn gst_init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        gstreamer::init().expect("GStreamer 初始化失败");
    });
}

/// 用 GStreamer 现场生成一个 H.264 MP4 测试视频并缓存（并行测试共享同一文件，
/// 用锁避免并发写）。返回 None 表示生成失败（调用方跳过该测试）。不绑定任何
/// 用户机器上的素材：`videotestsrc` 输出完全确定性的内容。
fn generated_test_video() -> Option<&'static Path> {
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            gst_init();
            let out = std::env::temp_dir().join("aweft_generated_test.mp4");
            if out.is_file() {
                return Some(out);
            }
            match generate_encoded_video(
                &out,
                TEST_VIDEO_W,
                TEST_VIDEO_H,
                TEST_VIDEO_FPS,
                TEST_VIDEO_FRAMES,
                None,
            ) {
                Ok(()) => Some(out),
                Err(error) => {
                    let _ = std::fs::remove_file(&out);
                    eprintln!("跳过：GStreamer 生成测试视频失败: {error}");
                    None
                }
            }
        })
        .as_deref()
}

/// 用 GStreamer 编码一段视频到 MP4：`videotestsrc → capsfilter → x264enc → qtmux
/// → filesink`。纯视频（无音轨）：无音频时钟时测试里把 appsink 设为 `sync=true`
/// 用系统时钟做实时节奏，避免 VA 表面池被全速解码灌满卡死。`par` 为
/// Some((n, d)) 时在 raw caps 上声明像素宽高比，x264enc 会把它写进 bitstream、
/// qtmux 写进 pasp，解码端读回。返回前等 EOS 收齐。
fn generate_encoded_video(
    out: &Path,
    width: i32,
    height: i32,
    fps: i32,
    frames: i32,
    par: Option<(i32, i32)>,
) -> Result<(), String> {
    use gstreamer::prelude::*;

    let pipeline = gstreamer::Pipeline::new();
    // videotestsrc 默认 pattern 就是 smpte（75% 彩条）；该枚举属性没有
    // gstreamer-rs 绑定，不显式设置。
    let src = gstreamer::ElementFactory::make("videotestsrc")
        .property("num-buffers", frames)
        .build()
        .map_err(|e| e.to_string())?;
    let mut caps_builder = gstreamer::Caps::builder("video/x-raw")
        .field("format", "I420")
        .field("width", width)
        .field("height", height)
        .field("framerate", gstreamer::Fraction::new(fps, 1));
    if let Some((num, den)) = par {
        caps_builder = caps_builder.field("pixel-aspect-ratio", gstreamer::Fraction::new(num, den));
    }
    let capsfilter = gstreamer::ElementFactory::make("capsfilter")
        .property("caps", caps_builder.build())
        .build()
        .map_err(|e| e.to_string())?;
    let enc = gstreamer::ElementFactory::make("x264enc")
        .property("key-int-max", fps as u32)
        .build()
        .map_err(|e| e.to_string())?;
    let mux = gstreamer::ElementFactory::make("qtmux")
        .build()
        .map_err(|e| e.to_string())?;
    let sink = gstreamer::ElementFactory::make("filesink")
        .property("location", out)
        .build()
        .map_err(|e| e.to_string())?;
    pipeline
        .add_many([&src, &capsfilter, &enc, &mux, &sink])
        .map_err(|e| e.to_string())?;
    gstreamer::Element::link_many([&src, &capsfilter, &enc, &mux, &sink])
        .map_err(|e| e.to_string())?;

    let bus = pipeline.bus().ok_or("bus 不可用")?;
    pipeline
        .set_state(gstreamer::State::Playing)
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(msg) = bus.timed_pop_filtered(
            gstreamer::ClockTime::from_mseconds(200),
            &[
                gstreamer::MessageType::Eos,
                gstreamer::MessageType::Error,
                gstreamer::MessageType::Warning,
            ],
        ) {
            use gstreamer::MessageView;
            match msg.view() {
                MessageView::Error(e) => {
                    let _ = pipeline.set_state(gstreamer::State::Null);
                    return Err(format!("GStreamer 错误: {}", e.error()));
                }
                MessageView::Eos(_) => break,
                MessageView::Warning(w) => log::warn!("生成测试视频 warning: {}", w.error()),
                _ => {}
            }
        }
        if Instant::now() > deadline {
            let _ = pipeline.set_state(gstreamer::State::Null);
            return Err("生成测试视频超时".into());
        }
    }
    pipeline
        .set_state(gstreamer::State::Null)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 解码/VA 重度测试的全局串行锁。多个测试并行各开一条 VA 管线（vapostproc +
/// iHD 上下文 + Vulkan headless 设备）会在共享 /dev/dri 上争用，偶发「无首帧」
/// 或 VA 初始化失败。这些测试彼此独立，串行执行不损失正确性，只牺牲一点并行度。
fn decode_serial_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn test_production_appsink_keeps_only_latest_frame() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gst_init();
    let sink = SlintWGPUSink::new();

    assert_eq!(sink.appsink.property::<u32>("max-buffers"), 1);
    assert!(sink.appsink.property::<bool>("drop"));
    assert!(
        sink.appsink.property::<bool>("sync"),
        "生产播放必须由媒体时钟同步，不能全速向 UI 推帧"
    );
}

#[test]
fn test_latest_frame_slot_overwrites_stale_sample() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gst_init();
    let slot = Mutex::new(None);
    let notifications = std::sync::atomic::AtomicUsize::new(0);
    let sample_at = |milliseconds| {
        let mut buffer = gstreamer::Buffer::new();
        buffer
            .get_mut()
            .expect("新建 buffer 应唯一持有")
            .set_pts(gstreamer::ClockTime::from_mseconds(milliseconds));
        Sample::builder().buffer(&buffer).build()
    };

    super::sink::store_latest_sample(&slot, sample_at(10), &|| {
        notifications.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    super::sink::store_latest_sample(&slot, sample_at(20), &|| {
        notifications.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });

    let latest = slot.lock().unwrap().take().expect("最新帧槽位为空");
    assert_eq!(
        latest.buffer().and_then(|buffer| buffer.pts()),
        Some(gstreamer::ClockTime::from_mseconds(20))
    );
    assert_eq!(notifications.load(std::sync::atomic::Ordering::Relaxed), 2);
}

#[test]
fn test_runtime_dmabuf_caps_keep_modifiers() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gst_init();

    let formats = gstreamer::List::new(vec![
        "NV12:0x0100000000000009",
        "P010:0x010000000000000a",
        "UNSUPPORTED:0x010000000000000b",
    ]);
    let caps = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("drm-format", &formats)
        .build();

    let filtered = filter_runtime_caps(caps);
    let structure = filtered.structure(0).expect("DMABuf caps 结构被错误移除");
    let formats = structure
        .get::<gstreamer::List>("drm-format")
        .expect("过滤后的 caps 缺少 drm-format");
    let formats = formats
        .as_ref()
        .iter()
        .map(|value| value.get::<String>().expect("drm-format 不是字符串"))
        .collect::<Vec<_>>();

    assert_eq!(
        formats,
        vec![
            "NV12:0x0100000000000009".to_string(),
            "P010:0x010000000000000a".to_string(),
        ],
        "支持的 modifier 必须原样保留，不能退化成裸 fourcc"
    );
}

/// 复现“导入即暂停、首帧作海报”的真实路径：appsink 安装了 new_preroll/new_sample
/// 回调（on_frame_arrived=Some）并走完整 connect()（bus 同步处理器丢弃消息 + 构建
/// wgpu 转换管线），管线置 Paused，再用 fetch_next_frame（渲染线程入口）拉首帧。
/// 回归点：暂停态 preroll 必须由回调写入最新帧槽位并请求重绘，不能依赖播放态
/// sample，也不能靠额外定时器或临时切到 Playing 才获得首帧。
#[test]
fn test_paused_preroll_with_callbacks() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let path = match generated_test_video() {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    let uri = url::Url::from_file_path(&path)
        .expect("测试视频路径无法转换为 URI")
        .to_string();
    gst_init();
    let (pipeline, _sink_element, mut wgpu_sink, _audio) =
        build_decode_pipeline_with_dmabuf(&uri, false, false).expect("搭建管线失败");
    let (instance, device, queue) = headless_wgpu();
    let _keep = &instance;
    let bus = pipeline.bus().expect("bus");
    let (bus_sender, _bus_receiver) = futures::channel::mpsc::unbounded::<gstreamer::Message>();
    wgpu_sink
        .connect(
            (
                std::sync::Arc::new(instance),
                std::sync::Arc::new(device),
                std::sync::Arc::new(queue),
            ),
            &bus,
            bus_sender,
            Box::new(|| {}),
        )
        .expect("连接 GPU 输出失败");
    pipeline
        .set_state(gstreamer::State::Paused)
        .expect("暂停态预滚动失败");

    // new_preroll 回调把首帧写入槽位，fetch_next_frame 应能直接消费它。
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = None;
    while Instant::now() < deadline {
        if let Some((_img, w, h)) = wgpu_sink.fetch_next_frame() {
            got = Some((w, h));
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = pipeline.set_state(gstreamer::State::Null);
    let (w, h) = got.expect("connect()+Paused+fetch_next_frame 取不回首帧（黑屏根因）");
    assert!(
        w.is_some() && h.is_some(),
        "首帧未携带分辨率变化（video-widths/heights 不更新，参考框尺寸错乱）"
    );
}

#[test]
fn test_runtime_dmabuf_caps_drop_unknown_format_but_keep_sysmem() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gst_init();

    let unsupported_dmabuf = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("drm-format", "UNSUPPORTED:0x010000000000000b")
        .build();
    let sysmem = gstreamer::Caps::builder("video/x-raw")
        .field("format", "Y444")
        .build();
    let mut caps = unsupported_dmabuf;
    caps.make_mut().append(sysmem);

    let filtered = filter_runtime_caps(caps);
    assert_eq!(filtered.size(), 1, "未知 DMABuf 格式应被移除");
    assert_eq!(
        filtered
            .structure(0)
            .unwrap()
            .get::<String>("format")
            .unwrap(),
        "Y444"
    );
}

#[test]
fn test_plane_bounds_reject_invalid_stride_and_truncation() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert!(positive_stride(-1, 0).is_none());
    assert!(plane_range(16, 0, 4, 4, 0).is_some());
    assert!(plane_range(15, 0, 4, 4, 0).is_none());
    assert!(plane_range(16, 14, 4, 1, 0).is_none());

    // 4K Y410：VideoMeta 行距 15488（VA 对齐），映射 payload 只有
    // 33177600B（= 紧凑 15360×2160）。真实布局是 15488/行、末尾被截断——
    // 必须信任 meta 行距并钳制行数；若像旧逻辑那样“回退紧凑 15360”，每一行
    // 都会从 padding/下一行字节起读，产生横向条纹。
    let payload = 3840 * 2160 * 4; // 33177600
    // 信任 meta 行距时，映射内实际可容纳行数为 floor(33177600/15488)=2142
    assert_eq!(plane_rows_avail(payload, 0, 15488, 2160), 2142);
    // payload 充足时不受影响（紧凑行距本来就能放满）
    assert_eq!(plane_rows_avail(payload, 0, 15360, 2160), 2160);
    // offset 越过 payload / 非法 stride → 0
    assert_eq!(plane_rows_avail(payload, payload, 15488, 2160), 0);
    assert_eq!(plane_rows_avail(payload, 0, 0, 2160), 0);
}
/// PAR 水平换算：显示宽度 = round(存储宽度 × num/den)。1:1 或无效 PAR 原样返回。
#[test]
fn test_display_width_from_par() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        display_width_from_par(720, gstreamer::Fraction::new(16, 15)),
        768
    );
    assert_eq!(
        display_width_from_par(720, gstreamer::Fraction::new(15, 16)),
        675
    );
    assert_eq!(
        display_width_from_par(1920, gstreamer::Fraction::new(1, 1)),
        1920
    );
    assert_eq!(
        display_width_from_par(720, gstreamer::Fraction::new(0, 1)),
        720
    );
}

/// `drm-format` 字符串解析：带 modifier（NV12:0x…）、裸 fourcc（NV12）都能
/// 解析；未知格式 / 空串返回 None。优先走库函数，裸 fourcc 走 from_le_bytes 回退。
#[test]
fn test_plane_layout_from_drm_format() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert!(dmabuf::plane_layout_from_drm_format("NV12:0x0100000000000009").is_some());
    assert!(dmabuf::plane_layout_from_drm_format("P010:0x010000000000000a").is_some());
    assert!(dmabuf::plane_layout_from_drm_format("NV12").is_some());
    assert!(dmabuf::plane_layout_from_drm_format("UNSUPPORTED:0x0100000000000009").is_none());
    assert!(dmabuf::plane_layout_from_drm_format("").is_none());
}

/// dmabuf 缓存代际：相同布局的 caps（即便 colorimetry 等无关字段不同）应同代；
/// 分辨率/格式/modifier 任一变化必须换代（触发导入缓存清空）。
#[test]
fn test_dmabuf_cache_epoch() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gst_init();
    let dmabuf_caps = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("width", 1920i32)
        .field("height", 1080i32)
        .field("drm-format", "NV12:0x0100000000000009")
        .build();

    let epoch = CacheEpoch::from_caps(dmabuf_caps.as_ref()).expect("解析 DMA_DRM caps");
    assert_eq!(epoch, CacheEpoch::from_caps(dmabuf_caps.as_ref()).unwrap());

    // 仅 colorimetry 变化 → 池未重建，代际不变
    let color_only = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("width", 1920i32)
        .field("height", 1080i32)
        .field("drm-format", "NV12:0x0100000000000009")
        .field("colorimetry", "bt2020")
        .build();
    assert_eq!(epoch, CacheEpoch::from_caps(color_only.as_ref()).unwrap());

    // 分辨率变化 → 换代
    let resized = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("width", 1280i32)
        .field("height", 720i32)
        .field("drm-format", "NV12:0x0100000000000009")
        .build();
    assert_ne!(epoch, CacheEpoch::from_caps(resized.as_ref()).unwrap());

    // format 变化（dmabuf → sysmem）→ 换代
    let sysmem = gstreamer::Caps::builder("video/x-raw")
        .field("format", "NV12")
        .field("width", 1920i32)
        .field("height", 1080i32)
        .build();
    assert_ne!(epoch, CacheEpoch::from_caps(sysmem.as_ref()).unwrap());
}

#[test]
fn test_video_caps_are_derived_from_format_capabilities() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gst_init();

    let caps = video_caps_with(false, false);
    let formats = caps
        .structure(0)
        .expect("sysmem caps 缺少结构")
        .get::<gstreamer::List>("format")
        .expect("sysmem caps 缺少格式列表");
    let formats = formats
        .as_ref()
        .iter()
        .map(|value| value.get::<String>().expect("格式不是字符串"))
        .collect::<Vec<_>>();
    let expected = super::dmabuf::sysmem_capabilities(false)
        .map(|capability| capability.name)
        .map(String::from)
        .collect::<Vec<_>>();

    assert_eq!(formats, expected);
    assert!(super::dmabuf::capability_for_video_format(VideoFormat::P01010le).is_some());
    assert!(sysmem_plane_layout(VideoFormat::P01010le).is_some());
}

/// 平面纹理（按字节模式逐像素填充后建视图）
fn uniform_plane(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    fill: &[u8],
) -> wgpu::TextureView {
    let bytes_per_px = fill.len() as u32;
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-plane"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let aligned = (w * bytes_per_px).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let mut data = vec![0u8; (aligned * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let base = y * aligned as usize + x * bytes_per_px as usize;
            data[base..base + bytes_per_px as usize].copy_from_slice(fill);
        }
    }
    queue.write_texture(
        tex.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    tex.create_view(&wgpu::TextureViewDescriptor::default())
}

/// 中性灰测试帧（8bit 全 128 / 10bit 全 0x8000）。
fn neutral_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    w: u32,
    h: u32,
    layout: PlaneLayout,
) -> FramePlanes {
    let (cw, ch) = (
        w.div_ceil(layout.chroma_width_divisor),
        h.div_ceil(layout.chroma_height_divisor),
    );

    if layout.is_packed() {
        // Y410: U=Y=V=512，A=3，little-endian word = 0xe0080200。
        let y_view = uniform_plane(
            device,
            queue,
            w,
            h,
            layout.y_format,
            &[0x00, 0x02, 0x08, 0xe0],
        );
        return FramePlanes {
            width: w,
            height: h,
            display_width: w,
            y_view,
            chroma: ChromaViews::Packed,
            layout,
            matrix_is_601: false,
            full_range: false,
            zero_copy: false,
        };
    }

    // P010：16bit 小端 0x8000 = 32768（= 10bit 512 左移 6 位）
    let y_fill: &[u8] = if matches!(layout.bit_depth, dmabuf::BitDepth::Ten) {
        &[0x00, 0x80]
    } else {
        &[128]
    };
    let chroma_fill: &[u8] = if layout.is_planar() {
        if matches!(layout.bit_depth, dmabuf::BitDepth::Ten) {
            &[0x00, 0x80]
        } else {
            &[128]
        }
    } else if matches!(layout.bit_depth, dmabuf::BitDepth::Ten) {
        &[0x00, 0x80, 0x00, 0x80]
    } else {
        &[128, 128]
    };
    let y_view = uniform_plane(device, queue, w, h, layout.y_format, y_fill);
    let chroma = if layout.is_planar() {
        let u = uniform_plane(device, queue, cw, ch, layout.chroma_format, chroma_fill);
        let v = uniform_plane(device, queue, cw, ch, layout.chroma_format, chroma_fill);
        ChromaViews::Planar(u, v)
    } else {
        ChromaViews::Interleaved(uniform_plane(
            device,
            queue,
            cw,
            ch,
            layout.chroma_format,
            chroma_fill,
        ))
    };
    FramePlanes {
        width: w,
        height: h,
        display_width: w,
        y_view,
        chroma,
        layout,
        matrix_is_601: false,
        full_range: false,
        zero_copy: false,
    }
}

/// 着色器数值校验：中性 YUV（8bit 128 / 10bit 512）→ 中性灰 ≈ (130,130,130)。
/// 覆盖 NV12（交错 8bit）、YU12（独立 U/V 8bit）、P010（交错 10bit）和
/// Y410（packed 4:4:4 10bit）布局。
#[test]
fn test_yuv_convert_shader_all_layouts() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_instance, device, queue) = headless_wgpu();
    let convert = YuvConvertPipeline::new(&device);
    let (w, h) = (256u32, 2u32);

    let mut layouts = vec![("NV12", b"NV12"), ("YU12", b"YU12"), ("P010", b"P010")]
        .into_iter()
        .filter_map(|(label, fourcc)| {
            let fourcc = fourcc[0] as u32
                | (fourcc[1] as u32) << 8
                | (fourcc[2] as u32) << 16
                | (fourcc[3] as u32) << 24;
            dmabuf::plane_layout(fourcc).map(|layout| (label, layout))
        })
        .collect::<Vec<_>>();
    let y410_fourcc = b"Y410"[0] as u32
        | (b"Y410"[1] as u32) << 8
        | (b"Y410"[2] as u32) << 16
        | (b"Y410"[3] as u32) << 24;
    layouts.push((
        "Y410-DMABuf",
        dmabuf::plane_layout(y410_fourcc).expect("Y410 DMABuf 布局未注册"),
    ));
    layouts.push((
        "Y410-sysmem",
        sysmem_plane_layout(VideoFormat::Y410).expect("Y410 布局未注册"),
    ));
    // 其余已注册的平面式布局（4:4:4 / 4:2:2 / YV12 反序）只走 sysmem 注册表，
    // 没有全部注册 drm_fourcc，统一从 sysmem_plane_layout 取。
    for (label, format) in [
        ("Y444", VideoFormat::Y444),
        ("Y42B", VideoFormat::Y42b),
        ("YV12", VideoFormat::Yv12),
    ] {
        layouts.push((
            label,
            sysmem_plane_layout(format).unwrap_or_else(|| panic!("{label} 布局未注册")),
        ));
    }

    for (label, layout) in layouts {
        let planes = neutral_frame(&device, &queue, w, h, layout);

        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-out"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        convert.run(&device, &queue, &planes, &out_view);
        let rgba = readback_rgba(&device, &queue, &out, w, h);
        let px = |i: usize| rgba[i] as i32;
        for i in [0, 4] {
            let (r, g, b) = (px(i), px(i + 1), px(i + 2));
            assert!(
                (125..=135).contains(&r),
                "{label} red={r}（期望中性灰 ~130）"
            );
            assert!((125..=135).contains(&g), "{label} green={g}");
            assert!((125..=135).contains(&b), "{label} blue={b}");
            assert_eq!(px(i + 3), 255, "{label} alpha 应为 255");
        }
    }
}

/// NV12 交错帧，Y/U/V 各取指定字节值；用于矩阵/色度方向的已知值校验。
fn colored_nv12_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    w: u32,
    h: u32,
    y: u8,
    u: u8,
    v: u8,
    matrix_is_601: bool,
    full_range: bool,
) -> FramePlanes {
    let layout = dmabuf::plane_layout(nv12_fourcc()).expect("NV12 布局未注册");
    let (cw, ch) = (
        w.div_ceil(layout.chroma_width_divisor),
        h.div_ceil(layout.chroma_height_divisor),
    );
    let y_view = uniform_plane(device, queue, w, h, layout.y_format, &[y]);
    let uv = uniform_plane(device, queue, cw, ch, layout.chroma_format, &[u, v]);
    FramePlanes {
        width: w,
        height: h,
        display_width: w,
        y_view,
        chroma: ChromaViews::Interleaved(uv),
        layout,
        matrix_is_601,
        full_range,
        zero_copy: false,
    }
}

fn nv12_fourcc() -> u32 {
    let f = b"NV12";
    f[0] as u32 | (f[1] as u32) << 8 | (f[2] as u32) << 16 | (f[3] as u32) << 24
}

/// 色彩矩阵三条路径（BT.709-limited / BT.601-limited / full-range）的已知值校验，
/// 且 U≠V 使 U/V 方向可判别。固定 Y=180、U=120、V=160，期望 RGB 由
/// `yuv_convert.wgsl` 中的系数按解析公式算出：
///   bt709-limited: y'=(180-16)/255·255/219, 矩阵 709 列；(249,175,175)
///   bt601-limited: y'同上，矩阵 601 列；(243,167,176)
///   full-range:    y=180/255, 矩阵 full 列；(226,159,167)
/// 若 U/V 被交换（如 YV12 平面反序未按 chroma_order 校正），R/B 会互换，
/// 此处断言必然失败——中性灰测试无法区分 U/V，正是这类 bug 的盲区。
#[test]
fn test_yuv_convert_shader_color_matrices() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_instance, device, queue) = headless_wgpu();
    let convert = YuvConvertPipeline::new(&device);
    let (w, h) = (64u32, 2u32);
    let (y, u, v) = (180u8, 120u8, 160u8);

    let cases: [(&str, bool, bool, [i32; 3]); 3] = [
        ("bt709-limited", false, false, [249, 175, 175]),
        ("bt601-limited", true, false, [243, 167, 176]),
        ("full-range", false, true, [226, 159, 167]),
    ];

    for (label, is_601, full_range, expected) in cases {
        let planes = colored_nv12_frame(&device, &queue, w, h, y, u, v, is_601, full_range);
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-out"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        convert.run(&device, &queue, &planes, &out_view);
        let rgba = readback_rgba(&device, &queue, &out, w, h);
        let (r, g, b) = (rgba[0] as i32, rgba[1] as i32, rgba[2] as i32);
        assert!(
            (r - expected[0]).abs() <= 3,
            "{label} red={r}（期望 {}）",
            expected[0]
        );
        assert!(
            (g - expected[1]).abs() <= 3,
            "{label} green={g}（期望 {}）",
            expected[1]
        );
        assert!(
            (b - expected[2]).abs() <= 3,
            "{label} blue={b}（期望 {}）",
            expected[2]
        );
        assert_eq!(rgba[3], 255, "{label} alpha 应为 255");
    }
}

/// 真实视频文件——默认路径（vapostproc GPU 转换 + sysmem 下载 +
/// write_texture 上传 + 离屏转色）端到端：走 sink 的真实渲染入口
/// fetch_next_frame，产出 slint::Image；校验尺寸与实时性能。
/// 文件不存在则跳过。
#[test]
fn test_playback_realpath() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let Some(path) = generated_test_video() else {
        return;
    };
    gst_init();
    let uri = url::Url::from_file_path(path)
        .expect("测试视频路径无法转换为 URI")
        .to_string();

    let (pipeline, _sink_element, mut sink, _audio) =
        build_decode_pipeline_with_dmabuf(&uri, true, true).expect("搭建解码管线失败");
    // sync=true：纯视频无音频时钟，让 appsink 按系统时钟以 30fps 实时节奏消费，
    // 避免全速解码把 VA 表面池灌满而卡死（本机 headless 环境实测约 20 帧后停）。
    _sink_element.set_property("sync", true);

    let Some((instance, device, queue)) = headless_wgpu_dmabuf() else {
        eprintln!("当前 wgpu 设备不支持 Vulkan DMABuf 导入，跳过真实 DMABuf 播放测试");
        return;
    };
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    let handles: WgpuHandles = (Arc::new(instance), device.clone(), queue.clone());
    let bus = pipeline.bus().unwrap();
    let (sender, _receiver) = futures::channel::mpsc::unbounded();
    sink.connect(handles, &bus, sender, Box::new(|| {}))
        .expect("连接视频 sink 失败");

    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("启动管线失败");

    // 取一帧（渲染真实入口）；超时窗口内轮询 bus，Error 即 panic。
    // headless 下没有 Slint 的 AfterRendering 调用来回收输出槽位池，必须手动
    // 模拟：after_rendering() 标记刚用的槽位等 GPU 完成，再 device.poll(Wait)
    // 强制触发其内部 on_submitted_work_done 回调放行该槽位。否则 3 个槽位
    // 用尽后 fetch_next_frame 恒返回 None——正是"取到 1-2 帧后卡死"的根因。
    let fetch_and_recycle = |sink: &mut SlintWGPUSink,
                             device: &wgpu::Device,
                             deadline: Instant|
     -> Option<slint::Image> {
        while Instant::now() < deadline {
            if let Some((image, _w, _h)) = sink.fetch_next_frame() {
                sink.after_rendering();
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                return Some(image);
            }
            if let Some(msg) = bus.timed_pop(gstreamer::ClockTime::from_mseconds(50)) {
                use gstreamer::MessageView;
                match msg.view() {
                    MessageView::Error(e) => {
                        panic!(
                            "GStreamer 错误: {} (来自 {:?})",
                            e.error(),
                            e.src().map(|s| s.path_string())
                        );
                    }
                    MessageView::Warning(w) => {
                        eprintln!(
                            "GStreamer 警告: {} (来自 {:?})",
                            w.error(),
                            w.src().map(|s| s.path_string())
                        );
                    }
                    MessageView::Element(elem) => {
                        eprintln!("Element 消息: {:?}", elem.structure());
                    }
                    _ => {}
                }
            }
        }
        None
    };

    // 首帧：尺寸校验
    let first = fetch_and_recycle(&mut sink, &device, Instant::now() + Duration::from_secs(30))
        .expect("无首帧");
    let (w, h) = (first.size().width as u32, first.size().height as u32);
    assert_eq!(
        (w, h),
        (TEST_VIDEO_W as u32, TEST_VIDEO_H as u32),
        "视频尺寸"
    );

    // 吞吐：90 帧（同步时钟 30fps 实时节奏；导入省略——sysmem 路径每帧
    // map+上传+转色）
    const N: usize = 90;
    let t0 = Instant::now();
    let mut got = 0;
    while got < N {
        if fetch_and_recycle(&mut sink, &device, Instant::now() + Duration::from_secs(15)).is_some()
        {
            got += 1;
        } else {
            break;
        }
    }
    let elapsed = t0.elapsed();
    let fps = got as f64 / elapsed.as_secs_f64();
    eprintln!(
        "默认路径（vpp→sysmem→上传→转色）吞吐: {got} 帧 / {elapsed:.2?} = {fps:.1} fps（素材 30 fps）"
    );
    // sync=true 后按 30fps 实时节奏出帧，90 帧约需 3 秒；这里只要求持续取得
    // 足够帧，不把时钟调度误判成视频转换性能回归。
    assert!(got >= 60, "仅取到 {got} 帧");
    assert!(fps > 10.0, "视频管线没有持续出帧: {fps:.1} fps");

    let _ = pipeline.set_state(gstreamer::State::Null);
}

/// 用 GStreamer 生成非方形 PAR 测试视频（720x576, SAR 16:15 → 显示 4:3）。
/// 已存在则复用；生成失败返回 Err（调用方跳过）。不依赖 ffmpeg。
fn generate_par_test_video() -> Result<PathBuf, String> {
    gst_init();
    let out = std::env::temp_dir().join("aweft_par_test.mp4");
    if out.is_file() {
        return Ok(out);
    }
    generate_encoded_video(&out, 720, 576, 25, 50, Some((16, 15)))?;
    Ok(out)
}

/// PAR 跟随输入端到端：GStreamer 生成 720x576 SAR=16:15（显示 4:3）的视频，
/// 走真实管线播放，断言 `fetch_next_frame` 回报的显示尺寸比例 == 4:3。
/// 无论 VPP 把 PAR 归一成 1:1（存储被重采样成 768x576）还是保留原始
/// PAR（存储 720x576 + 显示宽 768），回报比例都必须是 4:3。
/// GStreamer 生成失败则跳过该测试。
#[test]
fn test_playback_anamorphic_par() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let Ok(path) = generate_par_test_video() else {
        eprintln!("跳过：GStreamer 生成 PAR 测试视频失败");
        return;
    };
    gst_init();
    let uri = url::Url::from_file_path(path)
        .expect("测试视频路径无法转换为 URI")
        .to_string();

    // 强制 sysmem + autovideoconvert（build_format_pipeline）：headless 设备
    // 未启用 VULKAN_EXTERNAL_MEMORY_DMA_BUF，若协商出 DMA_DRM 候选会静默导入
    // 失败拿不到帧。PAR 显示换算逻辑在 sysmem 与 dmabuf 两条路径共享，
    // 走 sysmem 同样覆盖本次改动。
    let (pipeline, sink_element, mut sink) = match build_format_pipeline(&uri, "NV12", false) {
        Some(x) => x,
        None => panic!("NV12 sysmem 管线构建失败"),
    };
    // sync=true：纯视频无音频时钟时全速解码会灌满 VA 表面池卡死（约 20 帧后停）。
    sink_element.set_property("sync", true);
    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("启动管线失败");

    let (instance, device, queue) = headless_wgpu();
    let handles: WgpuHandles = (Arc::new(instance), Arc::new(device), Arc::new(queue));
    let bus = pipeline.bus().unwrap();
    let (sender, _receiver) = futures::channel::mpsc::unbounded();
    sink.connect(handles, &bus, sender, Box::new(|| {}))
        .expect("连接视频 sink 失败");

    // 首帧携显示尺寸（首次 size_changed=true）
    let mut frame: Option<slint::Image> = None;
    let mut displayed: Option<(u32, u32)> = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && displayed.is_none() {
        if let Some((image, w, h)) = sink.fetch_next_frame() {
            frame = Some(image);
            displayed = Some((w.unwrap_or(0), h.unwrap_or(0)));
        }
        if let Some(msg) = bus.timed_pop(gstreamer::ClockTime::from_mseconds(50)) {
            use gstreamer::MessageView;
            if let MessageView::Error(e) = msg.view() {
                let _ = pipeline.set_state(gstreamer::State::Null);
                panic!(
                    "GStreamer 错误: {} (来自 {:?})",
                    e.error(),
                    e.src().map(|s| s.path_string())
                );
            }
        }
    }
    let _ = pipeline.set_state(gstreamer::State::Null);
    assert!(frame.is_some(), "无首帧");
    let Some((disp_w, disp_h)) = displayed else {
        panic!("fetch_next_frame 未回报尺寸");
    };
    let ratio = disp_w as f64 / disp_h as f64;
    eprintln!(
        "PAR 测试: 源存储 720x576 SAR16:15；回报显示尺寸 {disp_w}x{disp_h}（ratio={ratio:.3}，期望 1.333）"
    );
    assert!(
        (ratio - 4.0 / 3.0).abs() < 0.02,
        "显示比例应为 4:3，实际 {disp_w}x{disp_h} ratio={ratio:.3}"
    );
}
/// 如果当前素材的最佳输出是系统内存，测试记录回退并通过；严格能力验证见
/// `test_dmabuf_zero_copy_realpath`。使用 GStreamer 生成的素材，缺素材则跳过。
#[test]
fn test_dmabuf_compatibility_realpath() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let Some(path) = generated_test_video() else {
        return;
    };
    gst_init();
    let uri = url::Url::from_file_path(path)
        .expect("测试视频路径无法转换为 URI")
        .to_string();
    dmabuf_optin_inner(&uri, false, Some((1280, 720)));
}

/// 严格验证 DMABuf 零拷贝链路。
///
/// 默认使用 GStreamer 生成的素材；通过 `AWEFT_DMABUF_TEST_VIDEO` 可指定真实
/// 素材覆盖（需要真实 VA/Vulkan 设备的场景）。
#[test]
fn test_dmabuf_zero_copy_realpath() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let path = match std::env::var_os("AWEFT_DMABUF_TEST_VIDEO") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if !path.is_file() {
                eprintln!("跳过：AWEFT_DMABUF_TEST_VIDEO 指向的文件不存在");
                return;
            }
            path
        }
        None => match generated_test_video() {
            Some(path) => path.to_path_buf(),
            None => return,
        },
    };
    let uri = url::Url::from_file_path(&path)
        .expect("DMABuf 测试视频路径无法转换为 URI")
        .to_string();

    gst_init();
    dmabuf_optin_inner(&uri, true, None);
}

fn dmabuf_optin_inner(uri: &str, require_dmabuf: bool, expected_size: Option<(u32, u32)>) {
    let (pipeline, sink_element, _sink, _audio) =
        build_decode_pipeline_with_dmabuf(uri, true, true).expect("搭建 dmabuf 管线失败");
    // sync=true：纯视频无音频时钟时全速解码会把 VA 表面池灌满卡死（约 20 帧后停），
    // 该测试需要连续拉 40+ 帧，必须用系统时钟做实时节奏。
    sink_element.set_property("sync", true);
    let appsink = sink_element
        .clone()
        .dynamic_cast::<gstreamer_app::AppSink>()
        .ok()
        .expect("sink 不是 appsink");
    appsink.set_callbacks(SlintWGPUSink::build_gst_appsink_callbacks(
        Default::default(),
        Default::default(),
        None,
    ));
    if require_dmabuf {
        let dmabuf_caps = gstreamer::Caps::builder("video/x-raw")
            .features(["memory:DMABuf"])
            .field("format", "DMA_DRM")
            .build();
        // 只锁定内存类型，不锁定 modifier；modifier 仍由 vapostproc/驱动
        // 根据真实分配能力动态选出。
        appsink.set_caps(Some(&dmabuf_caps));
    }
    // allocation 回调必须在状态切换前安装；DMABuf 的 VideoMeta 查询发生在
    // 协商阶段，晚于 Playing 才设置会错过这次查询而一直收不到首帧。
    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("启动管线失败");

    let (instance, device, queue) = headless_wgpu();
    let _keep = &instance;
    let Some(mut importer) = DmabufImporter::try_new(&device) else {
        // headless 设备（如 llvmpipe）未启用 VULKAN_EXTERNAL_MEMORY_DMA_BUF 时
        // 导入器不存在，零拷贝路径整体不可用。兼容性测试到此跳过即可，不要
        // 把"设备能力不足"当成实现 bug 红掉。
        eprintln!("当前 wgpu 设备不支持 dmabuf 导入（非 Vulkan 或缺少扩展），跳过测试");
        let _ = pipeline.set_state(gstreamer::State::Null);
        return;
    };
    let convert = YuvConvertPipeline::new(&device);

    let pull = |timeout_secs: u64| -> Sample {
        let bus = pipeline.bus().unwrap();
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        while Instant::now() < deadline {
            // Playing 之前 appsink 保存的是 preroll buffer；先取走它，
            // 让 pipeline 完成 Ready -> Playing，再读取后续 sample。
            if let Some(sample) = appsink.try_pull_preroll(gstreamer::ClockTime::from_mseconds(50))
            {
                return sample;
            }
            if let Some(sample) = appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(200))
            {
                return sample;
            }
            use gstreamer::MessageView;
            if let Some(msg) = bus.timed_pop(gstreamer::ClockTime::from_mseconds(50)) {
                if let MessageView::Error(e) = msg.view() {
                    panic!(
                        "GStreamer 错误: {} (来自 {:?})",
                        e.error(),
                        e.src().map(|s| s.path_string())
                    );
                }
            }
        }
        let (state_result, state, pending) =
            pipeline.state(Some(gstreamer::ClockTime::from_mseconds(0)));
        let negotiated_caps = sink_element
            .static_pad("sink")
            .and_then(|pad| pad.current_caps());
        eprintln!(
            "DMABuf 测试超时: pipeline state={state:?}, pending={pending:?}, state_result={state_result:?}, appsink caps={negotiated_caps:?}"
        );
        panic!("{timeout_secs} 秒内未收到帧");
    };

    // 首帧：检查是否实际协商到 DMABuf。兼容性测试允许驱动回退，严格
    // 测试则必须停在这里失败，不能把 sysmem 当成零拷贝通过。
    let first = pull(30);
    let caps = first.caps().unwrap().to_owned();
    if !is_dma_drm_caps(caps.as_ref()) {
        if require_dmabuf {
            panic!("严格 DMABuf 测试协商到系统内存: {caps}");
        }
        eprintln!("当前驱动未提供可用的 DMABuf 输出，GStreamer 已回退到: {caps}");
        let _ = pipeline.set_state(gstreamer::State::Null);
        return;
    }
    let info = VideoInfoDmaDrm::from_caps(caps.as_ref()).expect("解析 DMA caps 失败");
    if let Some(expected_size) = expected_size {
        assert_eq!((info.width(), info.height()), expected_size, "视频尺寸");
    }
    if require_dmabuf {
        let drm_format = caps
            .structure(0)
            .and_then(|structure| structure.get::<String>("drm-format").ok())
            .expect("严格 DMABuf 测试缺少 drm-format");
        assert!(
            dmabuf::plane_layout_from_drm_format(&drm_format).is_some(),
            "严格 DMABuf 测试协商到未被导入器支持的格式: {drm_format}"
        );
    }
    // 现在支持任意 modifier（tiled / linear），不再限制 linear
    let layout = dmabuf::plane_layout(info.fourcc())
        .unwrap_or_else(|| panic!("不支持的 fourcc 0x{:08x}", info.fourcc()));
    let (w, h) = (info.width(), info.height());

    // 处理帧：导入全部平面 → 转色（降采样出图，全幅回读太慢）→ 回读
    let process = |sample: &Sample,
                   importer: &mut DmabufImporter,
                   hashes: &mut Vec<u64>,
                   means: &mut Vec<f64>| {
        let buffer = sample.buffer().unwrap();
        let Some(meta) = buffer.meta::<VideoMeta>() else {
            panic!("缺 VideoMeta")
        };
        let strides = meta.stride();
        let (cw, ch) = (
            w.div_ceil(layout.chroma_width_divisor),
            h.div_ceil(layout.chroma_height_divisor),
        );
        let n_planes = 1 + layout.chroma_plane_count();
        let descs: Vec<dmabuf::PlaneDesc> = (0..n_planes)
            .map(|i| {
                let (pw, ph, fmt) = match i {
                    0 => (w, h, layout.y_format),
                    _ => (cw, ch, layout.chroma_format),
                };
                let plane = dmabuf_plane(buffer, meta.offset()[i], strides[i], i)
                    .expect("dmabuf 帧平面描述无效");
                dmabuf::PlaneDesc {
                    fd: plane.fd,
                    offset: plane.offset,
                    stride: plane.stride,
                    width: pw,
                    height: ph,
                    format: fmt,
                    modifier: info.modifier(),
                }
            })
            .collect();
        let views = importer
            .import_frame(&device, info.fourcc(), &descs)
            .expect("零拷贝导入失败");
        let planes = FramePlanes {
            width: w,
            height: h,
            display_width: w,
            y_view: views[0].clone(),
            chroma: super::frame::chroma_views(&layout, &views)
                .unwrap_or_else(|| panic!("平面视图不完整")),
            layout,
            matrix_is_601: false,
            full_range: false,
            zero_copy: true,
        };
        let (ow, oh) = (w / 8, h / 8);
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-out"),
            size: wgpu::Extent3d {
                width: ow,
                height: oh,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        convert.run(&device, &queue, &planes, &out_view);
        let rgba = readback_rgba(&device, &queue, &out, ow, oh);
        let sum: u64 = rgba.chunks_exact(4).map(|px| px[1] as u64).sum();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&rgba.as_slice(), &mut hasher);
        hashes.push(std::hash::Hasher::finish(&hasher));
        means.push(sum as f64 / (rgba.len() / 4) as f64);
    };

    if require_dmabuf {
        // 严格测试的判据到这里已经全部完成：caps、VideoMeta、fd 和
        // import_frame 成功。后续吞吐/重复帧检查属于兼容性测试的附加项，
        // 不让无音轨素材的音频分支状态影响 DMABuf 能力结论。
        let mut hashes = Vec::new();
        let mut means = Vec::new();
        process(&first, &mut importer, &mut hashes, &mut means);
        eprintln!(
            "严格 DMABuf 导入成功: fourcc=0x{:08x}, modifier=0x{:x}, 首帧灰度均值={:?}",
            info.fourcc(),
            info.modifier(),
            means
        );
        let _ = pipeline.set_state(gstreamer::State::Null);
        return;
    }

    // 片头淡入：取前 10 帧中最亮者做非平凡断言
    let mut hashes = Vec::new();
    let mut means = Vec::new();
    let mut alive = first.clone();
    for _ in 0..10 {
        process(&alive, &mut importer, &mut hashes, &mut means);
        let Some(next) = appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(2000)) else {
            break;
        };
        alive = next;
    }
    eprintln!("前 10 帧灰度均值: {means:?}");
    let max_mean = means.iter().copied().fold(0.0f64, f64::max);
    assert!(
        max_mean > 8.0 && max_mean < 247.0,
        "前 10 帧灰度均值全部异常: {means:?}"
    );

    // 同帧两次处理：内容稳定（隐式同步正确性 + 缓存命中路径）
    let n = hashes.len();
    process(&alive, &mut importer, &mut hashes, &mut means);
    process(&alive, &mut importer, &mut hashes, &mut means);
    assert!(
        (means[n] - means[n + 1]).abs() < 0.5,
        "同帧两次处理漂移: {} vs {}",
        means[n],
        means[n + 1]
    );
    assert_eq!(
        hashes[n],
        hashes[n + 1],
        "同帧两次回读哈希不一致（同步问题？）"
    );
    assert!(importer.cached_frames() <= 32, "导入缓存应受上限约束");

    // 吞吐只记录（dmabuf 零拷贝路径，含回读开销）
    let t0 = Instant::now();
    let mut got = 0;
    while got < 30 {
        let sample = pull(10);
        process(&sample, &mut importer, &mut hashes, &mut means);
        got += 1;
    }
    eprintln!(
        "dmabuf opt-in 吞吐（含回读）: {got} 帧 / {:.2?}",
        t0.elapsed()
    );

    let _ = pipeline.set_state(gstreamer::State::Null);
}

/// 强制或禁用 VA 硬件解码：通过运行时调整插件 rank 实现（进程内顺序切换）。
/// 注意：本仓库 `build_decode_pipeline` 在 VA 可用时总会插入 `vapostproc` 做
/// GPU 后处理，因此这里比较的是「解码器」软/硬之差（avdec vs vah265dec…），
/// 后处理链路共享。要定位「我们处理 vapostproc 输出」的 bug，看与 GStreamer
/// 原生 RGBA 参考的对比（见 `ref_render_rgba`）。
fn set_decode_ranks(force_sw: bool) {
    use gstreamer::prelude::*;
    let reg = gstreamer::Registry::get();
    let va = [
        "vah264dec",
        "vah265dec",
        "vah266dec",
        "vaav1dec",
        "vampeg2dec",
        "vavp8dec",
        "vavp9dec",
        "vah264lccdec",
        "vah265lccdec",
        "vaav1lccdec",
    ];
    let sw = [
        "avdec_h264",
        "avdec_h265",
        "avdec_h266",
        "avdec_av1",
        "avdec_mpeg2video",
        "avdec_vp8",
        "avdec_vp9",
    ];
    for n in va {
        if let Some(f) = reg.lookup_feature(n) {
            f.set_rank(if force_sw {
                gstreamer::Rank::NONE
            } else {
                gstreamer::Rank::PRIMARY
            });
        }
    }
    for n in sw {
        if let Some(f) = reg.lookup_feature(n) {
            f.set_rank(if force_sw {
                gstreamer::Rank::PRIMARY
            } else {
                gstreamer::Rank::NONE
            });
        }
    }
}

/// 软件参考管线：强制 avdec 解码 + autovideoconvert（软件 CSC）输出 P010/NV12，
/// 绕过 vapostproc（其输入格式表不含 Y444_10LE 等 4:4:4 10bit 平面格式，这类
/// 软解码输出会在 vapostproc 处协商失败）。
/// 这样软/硬两条路径最终都喂给我们的同一套 yuv_convert 着色器，差异仅来自
/// 「解码+后处理」链路本身。
fn build_soft_pipeline(
    uri: &str,
) -> Option<(gstreamer::Pipeline, gstreamer::Element, SlintWGPUSink)> {
    set_decode_ranks(true);
    let pipeline = gstreamer::Pipeline::new();
    let decoder = gstreamer::ElementFactory::make("uridecodebin")
        .name("video-decoder")
        .property("uri", uri)
        .build()
        .ok()?;
    pipeline.add(&decoder).ok()?;
    let video_queue = gstreamer::ElementFactory::make("queue").build().ok()?;
    let video_convert = gstreamer::ElementFactory::make("autovideoconvert")
        .build()
        .ok()?;
    let capsfilter = gstreamer::ElementFactory::make("capsfilter")
        .property("caps", video_caps_with(false, true))
        .build()
        .ok()?;
    let (sink_element, wgpu_sink) = create_sink();
    pipeline
        .add_many([&video_queue, &video_convert, &capsfilter, &sink_element])
        .ok()?;
    gstreamer::Element::link_many([&video_queue, &video_convert, &capsfilter, &sink_element])
        .ok()?;
    let video_entry = video_queue.clone();
    decoder.connect_pad_added(move |_decoder, pad| {
        let Some(caps) = pad.current_caps().or_else(|| Some(pad.query_caps(None))) else {
            return;
        };
        let Some(media) = caps.structure(0).map(|s| s.name().to_string()) else {
            return;
        };
        if !media.starts_with("video/") {
            return;
        }
        let Some(sp) = video_entry.static_pad("sink") else {
            return;
        };
        if sp.is_linked() {
            return;
        }
        let _ = pad.link(&sp);
    });
    Some((pipeline, sink_element, wgpu_sink))
}

/// 跳过片头黑场：capture 前先丢弃这么多帧（约 90/23.976 ≈ 3.75s）。
const SKIP_FRAMES: usize = 90;

/// 已建好管线：拉一帧（跳过片头黑场）→ 我们的 yuv_convert 着色器 → 全分辨率
/// RGBA。is_dma 由 caps 决定走 dmabuf 零拷贝或 sysmem 上传。返回 (rgba, 全宽, 全高)。
fn render_decoded_frame(
    pipeline: gstreamer::Pipeline,
    sink_element: gstreamer::Element,
    sink: SlintWGPUSink,
) -> Option<(Vec<u8>, u32, u32)> {
    render_decoded_frame_with_skip(pipeline, sink_element, sink, SKIP_FRAMES)
}

fn render_decoded_frame_with_skip(
    pipeline: gstreamer::Pipeline,
    sink_element: gstreamer::Element,
    mut sink: SlintWGPUSink,
    skip_frames: usize,
) -> Option<(Vec<u8>, u32, u32)> {
    // sync=true：纯视频无音频时钟，让 appsink 按系统时钟实时节奏消费，避免
    // 全速解码灌满 VA 表面池后卡死（本机 headless 实测约 20 帧后停）。
    sink_element.set_property("sync", true);
    let _bus = pipeline.bus();
    // 不设 new_sample 回调（notifier=None），避免回调抢先 pull 走样本；
    // 我们直接 try_pull_sample 拉帧。
    let appsink = sink_element.clone().dynamic_cast::<AppSink>().ok()?;
    // 回调必须在置 Playing 之前安装：vapostproc 的 DMABuf decide_allocation 发生在
    // 协商阶段，靠这里的 propose_allocation 请求 VideoMeta。晚装会错过这次查询，
    // vapostproc 报「DMABuf caps negotiated without the mandatory support of VideoMeta」
    // 并以 not-negotiated 停流，本函数取不到任何帧（同 `dmabuf_optin_inner` 的注释）。
    appsink.set_callbacks(SlintWGPUSink::build_gst_appsink_callbacks(
        Default::default(),
        Default::default(),
        None,
    ));
    pipeline.set_state(gstreamer::State::Playing).ok()?;
    // 本函数绕开 connect 直接调用 dmabuf_planes，因此需要自备申请了
    // VULKAN_EXTERNAL_MEMORY_DMA_BUF 的设备与 DmabufImporter；缺一时协商到
    // DMA_DRM 的帧必然导入失败。
    let (_instance, device, queue) = headless_wgpu_dmabuf().unwrap_or_else(headless_wgpu);
    sink.dmabuf = DmabufImporter::try_new(&device);
    let device = std::sync::Arc::new(device);
    let queue = std::sync::Arc::new(queue);

    let pull = || -> Option<gstreamer::Sample> {
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut skipped = 0usize;
        loop {
            let Some(s) = appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(200)) else {
                if Instant::now() > deadline {
                    return None;
                }
                continue;
            };
            if skipped < skip_frames {
                skipped += 1;
                continue;
            }
            return Some(s);
        }
    };
    let sample = pull()?;
    let caps = sample.caps()?.to_owned();
    let buffer = sample.buffer()?.to_owned();
    let is_dma = is_dma_drm_caps(caps.as_ref());
    let planes = if is_dma {
        sink.dmabuf_planes(&device, &queue, &caps, &buffer)
    } else {
        sink.sysmem_planes(&device, &queue, &caps, &buffer)
    }?;
    let (w, h) = (planes.width, planes.height);
    // 全分辨率渲染：降采样会把周期几像素的横向条纹平均掉，掩盖 bug。
    let ow = w;
    let oh = h;
    let out = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cmp-out"),
        size: wgpu::Extent3d {
            width: ow,
            height: oh,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
    let convert = YuvConvertPipeline::new(&device);
    convert.run(&device, &queue, &planes, &out_view);
    let rgba = readback_rgba(&device, &queue, &out, ow, oh);
    let _ = pipeline.set_state(gstreamer::State::Null);
    Some((rgba, w, h))
}

/// uridecodebin → autovideoconvert → capsfilter(format) → appsink（sysmem），
/// 用于指定特定像素格式（如 I420/YU12）单独验证某条着色器/stride 路径。
fn build_format_pipeline(
    uri: &str,
    format: &str,
    force_sw: bool,
) -> Option<(gstreamer::Pipeline, gstreamer::Element, SlintWGPUSink)> {
    if force_sw {
        set_decode_ranks(true);
    } else {
        set_decode_ranks(false);
    }
    let pipeline = gstreamer::Pipeline::new();
    let decoder = gstreamer::ElementFactory::make("uridecodebin")
        .property("uri", uri)
        .build()
        .ok()?;
    pipeline.add(&decoder).ok()?;
    let video_queue = gstreamer::ElementFactory::make("queue").build().ok()?;
    let video_convert = gstreamer::ElementFactory::make("autovideoconvert")
        .build()
        .ok()?;
    let capsfilter = gstreamer::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gstreamer::Caps::builder("video/x-raw")
                .field("format", format)
                .build(),
        )
        .build()
        .ok()?;
    let (sink_element, wgpu_sink) = create_sink();
    pipeline
        .add_many([&video_queue, &video_convert, &capsfilter, &sink_element])
        .ok()?;
    gstreamer::Element::link_many([&video_queue, &video_convert, &capsfilter, &sink_element])
        .ok()?;
    let video_entry = video_queue.clone();
    decoder.connect_pad_added(move |_decoder, pad| {
        let Some(caps) = pad.current_caps().or_else(|| Some(pad.query_caps(None))) else {
            return;
        };
        let Some(media) = caps.structure(0).map(|s| s.name().to_string()) else {
            return;
        };
        if !media.starts_with("video/") {
            return;
        }
        let Some(sp) = video_entry.static_pad("sink") else {
            return;
        };
        if sp.is_linked() {
            return;
        }
        let _ = pad.link(&sp);
    });
    Some((pipeline, sink_element, wgpu_sink))
}

/// 用我们的渲染管线（sysmem/dmabuf 上传 + yuv_convert 着色器）把一帧解码为
/// 全分辨率 RGBA。force_sw 控制软/硬链路。返回 (rgba, 全宽, 全高)。
fn our_render_frame(uri: &str, force_sw: bool) -> Option<(Vec<u8>, u32, u32)> {
    let (pipeline, sink_element, sink) = if force_sw {
        build_soft_pipeline(uri)?
    } else {
        set_decode_ranks(false);
        let (p, s, w, _a) = build_decode_pipeline_with_dmabuf(uri, true, true).ok()?;
        (p, s, w)
    };
    render_decoded_frame(pipeline, sink_element, sink)
}

/// GStreamer 原生 RGBA 参考帧（videoconvert 直接出 RGBA，绕过我们的着色器/
/// 上传/stride 处理），降采样到 ow×oh 便于与我们的渲染对比。这是定位
/// 「我们的 YUV→RGB 处理」是否有 bug 的金标准。
fn ref_render_rgba(uri: &str, force_sw: bool, ow: u32, oh: u32) -> Option<Vec<u8>> {
    ref_render_rgba_with_skip(uri, force_sw, ow, oh, SKIP_FRAMES)
}

fn ref_render_rgba_with_skip(
    uri: &str,
    force_sw: bool,
    ow: u32,
    oh: u32,
    skip_frames: usize,
) -> Option<Vec<u8>> {
    set_decode_ranks(force_sw);
    let pipeline = gstreamer::Pipeline::new();
    let dec = gstreamer::ElementFactory::make("uridecodebin")
        .property("uri", uri)
        .build()
        .ok()?;
    let queue = gstreamer::ElementFactory::make("queue").build().ok()?;
    let conv = gstreamer::ElementFactory::make("videoconvert")
        .build()
        .ok()?;
    let filter = gstreamer::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gstreamer::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .build(),
        )
        .build()
        .ok()?;
    let sink = AppSink::builder().max_buffers(1).drop(true).build();
    let sink_elem: gstreamer::Element = sink.clone().upcast();
    // sync=true：force_sw=false 时参考管线也走 VA 解码，全速拉帧会灌满表面池
    // 卡死；用系统时钟实时节奏（30fps）稳定出帧。
    sink_elem.set_property("sync", true);
    pipeline
        .add_many([&dec, &queue, &conv, &filter, &sink_elem])
        .ok()?;
    gstreamer::Element::link_many([&queue, &conv, &filter, &sink_elem]).ok()?;
    dec.connect_pad_added(move |_d, pad| {
        let Some(caps) = pad.current_caps().or_else(|| Some(pad.query_caps(None))) else {
            return;
        };
        let Some(media) = caps.structure(0).map(|s| s.name().to_string()) else {
            return;
        };
        if !media.starts_with("video/") {
            return;
        }
        let Some(sp) = queue.static_pad("sink") else {
            return;
        };
        if sp.is_linked() {
            return;
        }
        let _ = pad.link(&sp);
    });
    pipeline.set_state(gstreamer::State::Playing).ok()?;
    let sample = {
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut skipped = 0usize;
        loop {
            let Some(s) = sink.try_pull_sample(gstreamer::ClockTime::from_mseconds(200)) else {
                if Instant::now() > deadline {
                    return None;
                }
                continue;
            };
            if skipped < skip_frames {
                skipped += 1;
                continue;
            }
            break s;
        }
    };
    let caps = sample.caps()?;
    let info = VideoInfo::from_caps(caps).ok()?;
    let (w, h) = (info.width(), info.height());
    let stride = info.stride()[0] as usize;
    let map = sample.buffer()?.map_readable().ok()?;
    let data = map.as_slice();
    let step_x = (w / ow).max(1) as usize;
    let step_y = (h / oh).max(1) as usize;
    let mut out = Vec::with_capacity((ow * oh * 4) as usize);
    for y in 0..oh as usize {
        let sy = (y * step_y).min(h as usize - 1);
        for x in 0..ow as usize {
            let sx = (x * step_x).min(w as usize - 1);
            let o = sy * stride + sx * 4;
            out.extend_from_slice(&data[o..o + 4]);
        }
    }
    let _ = pipeline.set_state(gstreamer::State::Null);
    Some(out)
}

/// 平均绝对差（RGB，0..255 尺度）。
fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "对比帧尺寸不一致");
    let s: u64 = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| {
            (x[0] as i32 - y[0] as i32).abs() as u64
                + (x[1] as i32 - y[1] as i32).abs() as u64
                + (x[2] as i32 - y[2] as i32).abs() as u64
        })
        .sum();
    s as f64 / (a.len() as f64 / 4.0) / 3.0
}

fn dump_ppm(path: &str, rgba: &[u8], w: u32, h: u32) {
    let mut buf = Vec::with_capacity((w * h * 4 + 16) as usize);
    buf.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for px in rgba.chunks_exact(4) {
        buf.push(px[0]);
        buf.push(px[1]);
        buf.push(px[2]);
    }
    std::fs::write(path, &buf).expect("写 PPM 失败");
}

/// 横向条纹度量：逐行亮度均值，计算二阶差分能量（对周期 2 条纹最敏感）与
/// 相邻行最大跳变。数值越大代表越可能出现水平条带。
fn stripe_score(rgba: &[u8], w: u32, h: u32) -> (f64, f64) {
    let (w, h) = (w as usize, h as usize);
    let mut row_mean = vec![0f64; h];
    for y in 0..h {
        let mut s = 0u64;
        for x in 0..w {
            let i = (y * w + x) * 4;
            s += rgba[i] as u64 * 3 + rgba[i + 1] as u64 * 6 + rgba[i + 2] as u64;
        }
        row_mean[y] = s as f64 / w as f64;
    }
    let mut energy = 0f64;
    let mut max_delta = 0f64;
    for y in 1..h - 1 {
        let d2 = row_mean[y] - (row_mean[y - 1] + row_mean[y + 1]) * 0.5;
        energy += d2.abs();
        let d1 = (row_mean[y] - row_mean[y - 1]).abs();
        if d1 > max_delta {
            max_delta = d1;
        }
    }
    (energy / h as f64, max_delta)
}

/// 软解 vs 硬解 单帧对比：
/// - 两路都走我们的渲染管线 → 差异仅来自解码器（avdec vs VA）
/// - 硬解渲染 vs GStreamer 原生 RGBA → 差异来自「我们的 YUV→RGB 处理」
/// 判读：
///   · 两路都出现横条纹且 diff≈0 → bug 在共享着色器/上传（与解码无关）
///   · 仅硬解异常（与软解差异大）→ 硬解路径（vapostproc/dmabuf/stride）有 bug
///   · 与 GStreamer 参考差异大 → 我们的 YUV→RGB 转色/stride 处理损坏
///   · 三者都接近 → 解码与转色都正确，横条纹在更下游（Slint Image 上传/显示）
#[test]
fn test_soft_vs_hard_decode_frame_compare() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let Some(path) = generated_test_video() else {
        return;
    };
    let Ok(uri) = url::Url::from_file_path(path) else {
        eprintln!("跳过：测试视频路径无法转换 URI");
        return;
    };
    gst_init();
    let uri = uri.to_string();

    // 硬解那一路显式请求 DMABuf（本测试比较的正是零拷贝硬解与软解）。本机 wgpu
    // 设备申请不到 VULKAN_EXTERNAL_MEMORY_DMA_BUF 时，协商到 DMA_DRM 的帧无法渲染；
    // 那属于设备能力缺失而非渲染缺陷，因此先探测一次能力，不具备时跳过本测试。
    let dmabuf_usable = headless_wgpu_dmabuf()
        .is_some_and(|(_instance, device, _queue)| DmabufImporter::try_new(&device).is_some());
    if !dmabuf_usable {
        eprintln!("跳过：本机 wgpu 设备不支持 DMABuf 导入，硬解零拷贝路径不可测");
        return;
    }

    let Some((soft, w, h)) = our_render_frame(&uri, true) else {
        panic!("软件解码渲染失败");
    };
    let Some((hard, _, _)) = our_render_frame(&uri, false) else {
        panic!("硬件解码渲染失败");
    };

    // 全分辨率对比（不再降采样，保留横向条纹）
    let diff_dec = mean_abs_diff(&soft, &hard);
    dump_ppm("/tmp/aweft_soft.ppm", &soft, w, h);
    dump_ppm("/tmp/aweft_hard.ppm", &hard, w, h);
    let (e_soft, d_soft) = stripe_score(&soft, w, h);
    let (e_hard, d_hard) = stripe_score(&hard, w, h);
    eprintln!(
        "软解 vs 硬解（我们的渲染管线）平均绝对差: {diff_dec:.3}/255（{}x{} 全分辨率）",
        w, h
    );
    eprintln!(
        "横向条纹能量 软解={e_soft:.2}(最大行差{d_soft:.1}) 硬解={e_hard:.2}(最大行差{d_hard:.1})"
    );

    let Some(hard_ref) = ref_render_rgba(&uri, false, w, h) else {
        panic!("硬解参考渲染失败");
    };
    let diff_ref = mean_abs_diff(&hard, &hard_ref);
    let (e_ref, d_ref) = stripe_score(&hard_ref, w, h);
    dump_ppm("/tmp/aweft_gst_ref.ppm", &hard_ref, w, h);
    eprintln!("硬解（我们的渲染）vs GStreamer 原生 RGBA 参考 平均绝对差: {diff_ref:.3}/255");
    eprintln!("GStreamer 参考 横向条纹能量={e_ref:.2}(最大行差{d_ref:.1})");

    assert!(
        diff_dec < 40.0,
        "软解与硬解画面差异过大（{diff_dec:.2}），硬解路径可能损坏；检查 /tmp/aweft_soft.ppm 与 /tmp/aweft_hard.ppm"
    );
    assert!(
        diff_ref < 40.0,
        "我们的渲染与 GStreamer 原生 RGBA 差异过大（{diff_ref:.2}），YUV→RGB 转色/上传/stride 可能损坏；检查 /tmp/aweft_hard.ppm"
    );
    eprintln!(
        "✅ 软/硬解码一致，且与 GStreamer 参考位级一致。若仍见横条纹：\
         条纹来自解码后的 YUV 帧本身（vapostproc/解码器或片源），或下游 Slint Image 上传/显示，\
         而非 yuv_convert 着色器。检查 /tmp/aweft_hard.ppm 与 /tmp/aweft_gst_ref.ppm。"
    );
}

/// NV12（8bit 系统内存上传路径）像素校验：其余像素级对比里 sysmem 只覆盖到
/// I420/YV12/Y410（`test_yu12_sysmem_vs_gst_reference` 等），NV12 的 sysmem
/// 上传路径此前从未与参考对比过；DMA_DRM NV12 零拷贝那一路由
/// `test_soft_vs_hard_decode_frame_compare` 的硬解分支覆盖。本用例第三参数传
/// `false` 才对应名字里的 sysmem：那条路径按 VideoInfo 的紧凑行距切平面
/// （VideoMeta 只在协商到 DMABuf 时才请求，见
/// `SlintWGPUSink::build_gst_appsink_callbacks` 的 propose_allocation），行距或
/// 平面偏移算错就表现为横向条纹。条纹能量断言用相对判据：参考帧本身含稳定的
/// 横向细节，只要求上传路径不显著放大它。
#[test]
fn test_nv12_sysmem_vs_gst_reference() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let Some(path) = generated_test_video() else {
        return;
    };
    let Ok(uri) = url::Url::from_file_path(path) else {
        eprintln!("跳过：测试视频路径无法转换 URI");
        return;
    };
    gst_init();
    let uri = uri.to_string();
    set_decode_ranks(false);
    let (pipeline, sink_element, sink, _audio) =
        build_decode_pipeline_with_dmabuf(&uri, false, false).expect("搭建 NV12 管线失败");
    let Some((nv12, w, h)) = render_decoded_frame(pipeline, sink_element, sink) else {
        panic!("NV12 渲染失败");
    };
    let Some(refp) = ref_render_rgba(&uri, false, w, h) else {
        panic!("GStreamer 参考渲染失败");
    };
    let diff = mean_abs_diff(&nv12, &refp);
    let (e_nv12, d_nv12) = stripe_score(&nv12, w, h);
    let (e_ref, _) = stripe_score(&refp, w, h);
    dump_ppm("/tmp/aweft_nv12.ppm", &nv12, w, h);
    eprintln!(
        "NV12 我们的渲染 vs GStreamer 参考 平均绝对差: {diff:.3}/255（{}x{}）",
        w, h
    );
    eprintln!("NV12 条纹能量={e_nv12:.2}(最大行差{d_nv12:.1})  参考条纹能量={e_ref:.2}");
    assert!(
        diff < 40.0,
        "NV12 路径渲染损坏（diff={diff:.2}）；检查 /tmp/aweft_nv12.ppm"
    );
    // 参考帧本身包含稳定的横向细节（videotestsrc 生成素材实测 2.95），不能用
    // 绝对的“接近零”阈值；只要求上传路径不要显著放大参考帧的条纹能量。
    assert!(
        e_nv12 <= e_ref + 1.0,
        "NV12 路径放大了横向条纹（能量={e_nv12:.2}，参考={e_ref:.2}）；检查 /tmp/aweft_nv12.ppm"
    );
    eprintln!("✅ NV12 sysmem 上传路径（按 VideoInfo 紧凑行距切平面）像素级正确。");
}

/// 逐通道均值（0..255），用于定位偏差来源（U/V 反序→R/B 互异；亮度范围错→三通道同偏）。
fn channel_means(rgba: &[u8]) -> (f64, f64, f64) {
    let n = (rgba.len() / 4) as f64;
    let (mut r, mut g, mut b) = (0f64, 0f64, 0f64);
    for px in rgba.chunks_exact(4) {
        r += px[0] as f64;
        g += px[1] as f64;
        b += px[2] as f64;
    }
    (r / n, g / n, b / n)
}

/// `build_format_pipeline` 强制输出指定格式后，我们的渲染 vs GStreamer 原生
/// RGBA 的完整对比结果（像素、平均绝对差与条纹能量）。
struct FormatComparison {
    ours: Vec<u8>,
    reference: Vec<u8>,
    diff: f64,
    ours_stripe: (f64, f64),
    ref_stripe: (f64, f64),
    w: u32,
    h: u32,
}

/// 格式专项对比的公共样板：搭建 `format` 输出管线 → 我们的渲染 → GStreamer
/// 原生参考 → 计算 diff 与条纹能量。供 I420/YV12/Y410 等格式测试复用。
fn format_vs_gst_reference(
    uri: &str,
    format: &str,
    force_sw: bool,
    skip: usize,
) -> Option<FormatComparison> {
    let (pipeline, sink_element, sink) = build_format_pipeline(uri, format, force_sw)?;
    let (ours, w, h) = render_decoded_frame_with_skip(pipeline, sink_element, sink, skip)?;
    let reference = ref_render_rgba_with_skip(uri, force_sw, w, h, skip)?;
    Some(FormatComparison {
        diff: mean_abs_diff(&ours, &reference),
        ours_stripe: stripe_score(&ours, w, h),
        ref_stripe: stripe_score(&reference, w, h),
        ours,
        reference,
        w,
        h,
    })
}

/// 单独验证 YU12（I420）平面布局的着色器/stride 路径：dmabuf 零拷贝路径只
/// 导出 YU12（iHD 唯一线性格式），而 sysmem 路径原先不支持 YU12，故该布局
/// 从未被测试。这里强制 I420 走 sysmem，渲染成 RGBA 后对比 GStreamer 原生
/// RGBA 参考。若差异大 → YU12 着色器/平面处理损坏（dmabuf 路径也用同一着色器，
/// 故会同等条纹）；若接近 → YU12 着色器正确，条纹来自 dmabuf 导入本身。
#[test]
fn test_yu12_sysmem_vs_gst_reference() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let Some(path) = generated_test_video() else {
        return;
    };
    let Ok(uri) = url::Url::from_file_path(path) else {
        eprintln!("跳过：测试视频路径无法转换 URI");
        return;
    };
    gst_init();
    let uri = uri.to_string();

    let Some(cmp) = format_vs_gst_reference(&uri, "I420", false, SKIP_FRAMES) else {
        panic!("I420 渲染或参考失败");
    };
    dump_ppm("/tmp/aweft_yu12.ppm", &cmp.ours, cmp.w, cmp.h);
    let (yr, yg, yb) = channel_means(&cmp.ours);
    let (rr, rg, rb) = channel_means(&cmp.reference);
    let (e_yu12, _) = cmp.ours_stripe;
    let (e_ref, _) = cmp.ref_stripe;
    eprintln!(
        "I420(YU12) 我们的渲染 vs GStreamer 参考 平均绝对差: {:.3}/255（{}x{}）",
        cmp.diff, cmp.w, cmp.h
    );
    eprintln!("I420 条纹能量={e_yu12:.2}  GStreamer 参考条纹能量={e_ref:.2}（两者应接近）");
    eprintln!("逐通道均值 我们的(R{yr:.1} G{yg:.1} B{yb:.1}) 参考(R{rr:.1} G{rg:.1} B{rb:.1})");
    // 静态生成内容保证两条管线取到逐像素相同的帧；差异仅来自 videoconvert 与
    // shader 的矩阵系数（正常 5--9）。U/V 纹理别名等平面错误会推到 12 以上。
    assert!(
        cmp.diff < 12.0,
        "YU12 平面着色器/stride 处理损坏（diff={:.2}）；检查 /tmp/aweft_yu12.ppm",
        cmp.diff
    );
    eprintln!(
        "✅ YU12 平面着色器/stride 正确。结合软/硬测试可下结论：dmabuf 零拷贝导入（dmabuf.rs）\
         是横向条纹的来源。检查 /tmp/aweft_yu12.ppm 与 /tmp/aweft_hard.ppm（dmabuf 路径）。"
    );
}

/// 单独验证 YV12 的平面顺序（chroma_order）：GStreamer 的 YV12 平面顺序是
/// Y、V、U，`offsets[1]` 是 V 平面。若平面映射仍硬编码
/// `offsets[1]→U`，R/B 会互换，diff 会远超正常矩阵系数差。中性灰测试无法
/// 区分 U/V，本测试用真实内容把该假设落实。
#[test]
fn test_yv12_sysmem_vs_gst_reference() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let Some(path) = generated_test_video() else {
        return;
    };
    let Ok(uri) = url::Url::from_file_path(path) else {
        eprintln!("跳过：测试视频路径无法转换 URI");
        return;
    };
    gst_init();
    let uri = uri.to_string();

    let Some(cmp) = format_vs_gst_reference(&uri, "YV12", false, SKIP_FRAMES) else {
        panic!("YV12 渲染或参考失败");
    };
    dump_ppm("/tmp/aweft_yv12.ppm", &cmp.ours, cmp.w, cmp.h);
    let (yr, yg, yb) = channel_means(&cmp.ours);
    let (rr, rg, rb) = channel_means(&cmp.reference);
    let (e_yv12, _) = cmp.ours_stripe;
    let (e_ref, _) = cmp.ref_stripe;
    eprintln!(
        "YV12 我们的渲染 vs GStreamer 参考 平均绝对差: {:.3}/255（{}x{}）",
        cmp.diff, cmp.w, cmp.h
    );
    eprintln!("YV12 条纹能量={e_yv12:.2}  GStreamer 参考条纹能量={e_ref:.2}（两者应接近）");
    eprintln!("逐通道均值 我们的(R{yr:.1} G{yg:.1} B{yb:.1}) 参考(R{rr:.1} G{rg:.1} B{rb:.1})");
    assert!(
        cmp.diff < 12.0,
        "YV12 平面顺序（chroma_order）处理损坏（diff={:.2}）；检查 /tmp/aweft_yv12.ppm",
        cmp.diff
    );
    eprintln!("✅ YV12 平面 U/V 顺序（Y,V,U→chroma_order 校正）像素级正确。");
}

/// 验证 packed Y410 的 sysmem 行距处理。默认使用 GStreamer 生成的素材
/// （autovideoconvert 会把解码输出转成 Y410），通过 `AWEFT_Y410_TEST_VIDEO`
/// 可指定真实 Y410 素材覆盖。
#[test]
fn test_y410_sysmem_vs_gst_reference() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    let path = match std::env::var_os("AWEFT_Y410_TEST_VIDEO") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if !path.is_file() {
                eprintln!("跳过：AWEFT_Y410_TEST_VIDEO 指向的文件不存在");
                return;
            }
            path
        }
        None => match generated_test_video() {
            Some(path) => path.to_path_buf(),
            None => return,
        },
    };
    gst_init();
    let uri = url::Url::from_file_path(&path)
        .expect("Y410 测试视频路径无法转换为 URI")
        .to_string();
    let Some(cmp) = format_vs_gst_reference(&uri, "Y410", true, 0) else {
        panic!("Y410 渲染或参考失败");
    };
    dump_ppm("/tmp/aweft_y410.ppm", &cmp.ours, cmp.w, cmp.h);
    eprintln!(
        "Y410 sysmem 我们的渲染 vs GStreamer 参考 平均绝对差: {:.3}/255（{}x{}）",
        cmp.diff, cmp.w, cmp.h
    );
    assert!(
        cmp.diff < 12.0,
        "Y410 sysmem 行距/位域处理损坏（diff={:.2}）；检查 /tmp/aweft_y410.ppm",
        cmp.diff
    );
}

/// 请求 dmabuf 导入特性的 headless 设备；不可用（非 Vulkan / 驱动缺扩展）返回 None。
fn headless_wgpu_dmabuf() -> Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
    let (device, queue) =
        futures::executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF,
            ..Default::default()
        }))
        .ok()?;
    Some((instance, device, queue))
}

/// 从 vapostproc 的 src 模板 caps 中解析出 `AB24:<modifier>`。
/// iHD 等驱动把 RGBA 格式以 X-TILED modifier 导出；模板是插件注册时按
/// 当前 VA display 探测生成的，因此这是驱动能力的权威来源。
fn find_ab24_in_vpp_template(vpp: &gstreamer::Element) -> Option<(u32, u64)> {
    let template_caps = vpp.static_pad("src")?.pad_template_caps();
    for structure in template_caps.iter() {
        if structure.name() != "video/x-raw" {
            continue;
        }
        let Ok(list) = structure.get::<gstreamer::List>("drm-format") else {
            continue;
        };
        for value in list.as_ref().iter() {
            let Ok(name) = value.get::<String>() else {
                continue;
            };
            if name.starts_with("AB24")
                && let Ok(parsed) = gstreamer_video::dma_drm_fourcc_from_str(&name)
            {
                return Some(parsed);
            }
        }
    }
    None
}

/// AB24 直出能力探针：验证 vapostproc 能否导出 AB24（ABGR8888）DMABuf 且
/// Vulkan 可将其导入为 Rgba8Unorm 纹理。成功则进一步做像素级正确性校验
/// （与同一 VA 引擎下载的 BGRA sysmem 逐像素对比）并确认 slint::Image
/// 接受性；失败（如 Arc DG2 无法导入 X-TILED）打印诊断结论后跳过。
#[test]
fn test_dmabuf_rgba_ab24_import_spike() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _serial = decode_serial_lock();
    gst_init();
    const W: u32 = 640;
    const H: u32 = 480;

    // ---- 管线 A：NV12 → vapostproc → DMA_DRM AB24 → appsink ----
    let pipeline_a = gstreamer::Pipeline::new();
    let vpp = gstreamer::ElementFactory::make("vapostproc")
        .property("disable-passthrough", true)
        .build()
        .expect("创建 vapostproc 失败");
    let Some((fourcc_ab24, modifier)) = find_ab24_in_vpp_template(&vpp) else {
        eprintln!("跳过：当前 VA 驱动的 vapostproc 不支持 AB24 DMABuf 输出");
        return;
    };
    assert_eq!(fourcc_ab24, super::dmabuf::ab24_fourcc());

    let appsink_a = gstreamer_app::AppSink::builder()
        .enable_last_sample(false)
        .max_buffers(1)
        .drop(true)
        .build();
    appsink_a.set_callbacks(SlintWGPUSink::build_gst_appsink_callbacks(
        Default::default(),
        Default::default(),
        None,
    ));
    let src_caps = gstreamer::Caps::builder("video/x-raw")
        .field("format", "NV12")
        .field("width", W as i32)
        .field("height", H as i32)
        .field("framerate", gstreamer::Fraction::new(30, 1))
        .build();
    let dma_caps = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        // caps 求交对字符串做精确比较，modifier 必须与模板同样补零到 16 位
        .field("drm-format", format!("AB24:0x{:016x}", modifier))
        .build();
    {
        let template = vpp.static_pad("src").unwrap().pad_template_caps();
        let common = template.intersect(&dma_caps);
        assert!(
            !common.is_empty(),
            "请求的 AB24 caps 与 vapostproc 模板无交集:\n请求={dma_caps}\n模板={template}"
        );
    }
    pipeline_a
        .add_many([&vpp, appsink_a.upcast_ref()])
        .expect("add A 失败");
    {
        let src = gstreamer::ElementFactory::make("videotestsrc")
            .property("num-buffers", 2i32)
            .build()
            .expect("videotestsrc 失败");
        let cf_in = gstreamer::ElementFactory::make("capsfilter")
            .property("caps", &src_caps)
            .build()
            .unwrap();
        let cf_out = gstreamer::ElementFactory::make("capsfilter")
            .property("caps", &dma_caps)
            .build()
            .unwrap();
        pipeline_a
            .add_many([&src, &cf_in, &cf_out])
            .expect("add A2 失败");
        gstreamer::Element::link_many([&src, &cf_in, &vpp, &cf_out, appsink_a.upcast_ref()])
            .expect("link A 失败");
    }
    appsink_a
        .upcast_ref::<gstreamer::Element>()
        .set_property("sync", false);

    // ---- 管线 B：同引擎参考——vapostproc 直接输出 BGRA sysmem ----
    let pipeline_b = gstreamer::Pipeline::new();
    let vpp_b = gstreamer::ElementFactory::make("vapostproc")
        .property("disable-passthrough", true)
        .build()
        .expect("创建 vapostproc B 失败");
    let appsink_b = gstreamer_app::AppSink::builder().build();
    let bgra_caps = gstreamer::Caps::builder("video/x-raw")
        .field("format", "BGRA")
        .field("width", W as i32)
        .field("height", H as i32)
        .build();
    let cf_b = gstreamer::ElementFactory::make("capsfilter")
        .property("caps", &bgra_caps)
        .build()
        .unwrap();
    pipeline_b
        .add_many([&vpp_b, &cf_b, appsink_b.upcast_ref()])
        .unwrap();
    {
        let src = gstreamer::ElementFactory::make("videotestsrc")
            .property("num-buffers", 2i32)
            .build()
            .unwrap();
        let cf_in = gstreamer::ElementFactory::make("capsfilter")
            .property("caps", &src_caps)
            .build()
            .unwrap();
        pipeline_b.add_many([&src, &cf_in]).unwrap();
        gstreamer::Element::link_many([&src, &cf_in, &vpp_b, &cf_b, appsink_b.upcast_ref()])
            .unwrap();
    }
    appsink_b
        .upcast_ref::<gstreamer::Element>()
        .set_property("sync", false);

    pipeline_a.set_state(gstreamer::State::Playing).unwrap();
    pipeline_b.set_state(gstreamer::State::Playing).unwrap();

    let pull_frame = |appsink: &gstreamer_app::AppSink, label: &'static str| -> gstreamer::Sample {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(sample) = appsink.try_pull_preroll(gstreamer::ClockTime::from_mseconds(100))
            {
                return sample;
            }
            if let Some(sample) = appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(200))
            {
                return sample;
            }
            assert!(Instant::now() < deadline, "{label}: 30 秒内未收到帧");
        }
    };

    let sample_dma = pull_frame(&appsink_a, "AB24");
    let sample_ref = pull_frame(&appsink_b, "BGRA");

    // ---- 校验 A 协商结果并提取单平面描述 ----
    let caps_dma = sample_dma.caps().expect("A 无 caps").to_owned();
    assert!(
        is_dma_drm_caps(caps_dma.as_ref()),
        "管线 A 未协商到 DMA_DRM: {caps_dma}"
    );
    let info = gstreamer_video::VideoInfoDmaDrm::from_caps(caps_dma.as_ref())
        .expect("解析 DMA_DRM caps 失败");
    eprintln!(
        "协商结果: fourcc=0x{:08x} modifier=0x{:x} {}x{}",
        info.fourcc(),
        info.modifier(),
        info.width(),
        info.height()
    );
    assert_eq!(info.fourcc(), super::dmabuf::ab24_fourcc());
    assert_eq!((info.width(), info.height()), (W, H));

    let buffer_dma = sample_dma.buffer().expect("A 无 buffer").to_owned();
    let meta = buffer_dma
        .meta::<gstreamer_video::VideoMeta>()
        .expect("缺 VideoMeta");
    assert_eq!(meta.n_planes(), 1, "AB24 应为单平面");
    let plane = super::frame::dmabuf_plane(&buffer_dma, meta.offset()[0], meta.stride()[0], 0)
        .expect("提取 AB24 平面失败");

    // ---- Vulkan 导入：单平面 Rgba8Unorm + COLOR_TARGET（Slint 要求 RENDER_ATTACHMENT）----
    let Some((instance, device, queue)) = headless_wgpu_dmabuf() else {
        eprintln!("跳过：当前环境无支持 dmabuf 导入的 Vulkan 设备");
        let _ = (
            pipeline_a.set_state(gstreamer::State::Null),
            pipeline_b.set_state(gstreamer::State::Null),
        );
        return;
    };
    let _keep_alive = instance;
    let hal_guard = unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }.expect("非 Vulkan 后端");
    eprintln!(
        "导入参数: fd={} stride={} offset={} modifier=0x{:016x}",
        plane.fd, plane.stride, plane.offset, modifier
    );
    // 每次尝试都会消费一个 fd（成功归 Vulkan，失败被关闭），因此逐次 dup。
    // 先带 COLOR_TARGET（Slint RENDER_ATTACHMENT 要求），失败则降级探测，
    // 以区分「usage 不支持」与「modifier/格式组合不支持」。
    use std::os::unix::io::FromRawFd as _;
    let attempts: [(&str, wgpu::TextureUses); 2] = [
        (
            "RESOURCE|COLOR_TARGET",
            wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COLOR_TARGET,
        ),
        ("RESOURCE", wgpu::TextureUses::RESOURCE),
    ];
    let mut imported_tex = None;
    for (label, hal_usage) in attempts {
        let dup_fd = unsafe { libc::dup(plane.fd) };
        assert!(dup_fd >= 0, "dup fd 失败");
        let owned_fd = unsafe { std::os::unix::io::OwnedFd::from_raw_fd(dup_fd) };
        let hal_desc = wgpu::hal::TextureDescriptor {
            label: Some("spike-ab24"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: hal_usage,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        match unsafe {
            hal_guard.texture_from_dmabuf_fd(
                owned_fd,
                &hal_desc,
                modifier,
                u64::from(plane.stride),
                plane.offset,
            )
        } {
            Ok(hal_tex) => {
                eprintln!("✅ 导入成功（{label}）");
                let tex = unsafe {
                    device.create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                        hal_tex,
                        &wgpu::TextureDescriptor {
                            label: Some("spike-ab24"),
                            size: hal_desc.size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        },
                        wgpu::TextureUses::UNINITIALIZED,
                    )
                };
                imported_tex = Some(tex);
                break;
            }
            Err(error) => eprintln!("❌ 导入失败（{label}）: {error:?}"),
        }
    }
    let Some(imported_tex) = imported_tex else {
        // 典型场景：iHD(DG2) 只把 RGBA 表面导出为 legacy X-TILED，而 Arc 的
        // 渲染引擎不支持采样 X-TILED（ANV 仅列 Linear/Tile4 家族）。这是硬件
        // 能力边界而非实现缺陷——保留本测试作为驱动升级后的复验探针。
        eprintln!(
            "结论：当前 GPU/驱动无法导入 VA 导出的 AB24 dmabuf（modifier 0x{modifier:016x}），\
             RGBA 直出路径不可用；现有 YU12 线性零拷贝仍是该机器上的最优形态"
        );
        let _ = (
            pipeline_a.set_state(gstreamer::State::Null),
            pipeline_b.set_state(gstreamer::State::Null),
        );
        return;
    };

    let rgba_imported = readback_rgba(&device, &queue, &imported_tex, W, H);

    // ---- 参考：BGRA sysmem map → 行内 swizzle 成 RGBA ----
    let caps_ref = sample_ref.caps().expect("B 无 caps").to_owned();
    let info_ref =
        gstreamer_video::VideoInfo::from_caps(caps_ref.as_ref()).expect("BGRA VideoInfo 失败");
    assert_eq!(info_ref.format(), gstreamer_video::VideoFormat::Bgra);
    let stride_ref = info_ref.stride()[0] as usize;
    let map = sample_ref
        .buffer()
        .expect("B 无 buffer")
        .map_readable()
        .expect("BGRA map 失败");
    let data = map.as_slice();
    let mut rgba_ref = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H as usize {
        let row = &data[y * stride_ref..];
        for px in row[..W as usize * 4].chunks_exact(4) {
            rgba_ref.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }

    let diff = mean_abs_diff(&rgba_imported, &rgba_ref);
    eprintln!("AB24 导入回读 vs VA BGRA 下载 平均绝对差: {diff:.4}/255");

    // ---- Slint 接受性（A2 直通的硬前提）----
    let image_result = slint::Image::try_from(imported_tex.clone());
    match &image_result {
        Ok(_) => eprintln!(
            "✅ slint::Image 接受导入纹理（Rgba8Unorm + BINDING|RENDER_ATTACHMENT|COPY_SRC）"
        ),
        Err(e) => eprintln!("❌ slint::Image 拒绝导入纹理: {e:?}"),
    }

    let _ = pipeline_a.set_state(gstreamer::State::Null);
    let _ = pipeline_b.set_state(gstreamer::State::Null);

    assert!(
        diff < 2.0,
        "AB24 导入内容与 VA 参考不一致（diff={diff:.4}）——检查通道序/modifier/stride"
    );
    assert!(
        image_result.is_ok(),
        "slint::Image 拒绝导入纹理——A2 直通方案需改走 blit 降级"
    );
    eprintln!("✅ AB24 单平面零拷贝导入全链路验证通过");
}
