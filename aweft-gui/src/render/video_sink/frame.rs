//! 帧处理：把 GStreamer 的一帧 Sample（dmabuf 或 sysmem）转换为 Y/U/V 平面
//! 纹理视图，经离屏 `YuvConvertPipeline` 转成 RGBA 输出纹理。包含：
//! - 平面布局推导与上传（sysmem 路径，含 256 字节行距对齐）
//! - dmabuf 零拷贝导入组装（委托给 `dmabuf::DmabufImporter`）
//! - 固定槽位的 RGBA 输出纹理池（Slint Image 生命周期安全复用）
//! - 几何 / 色彩辅助函数

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gstreamer_allocators::DmaBufMemory;
use gstreamer_video::{VideoInfo, VideoInfoDmaDrm, VideoMeta, is_dma_drm_caps};
use slint::wgpu_30::wgpu;

use super::dmabuf::{self, PlaneLayout};
use super::sink::SlintWGPUSink;

/// dmabuf 导入缓存的代际标识：由 caps 中决定 VA surface 池布局的字段
/// （宽高 + 格式/drm-format）构成。任一变化意味着池已重建、旧 fd 可能已失效
/// 或被内核复用，此时必须清空 `DmabufImporter`。
///
/// 刻意只取"布局相关"字段：framerate/colorimetry 等变化不会重建 surface 池，
/// 不应触发误清除。drm-format 自带 fourcc+modifier，能区分 sysmem/dmabuf 与
/// modifier 重协商。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheEpoch {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: String,
    pub(crate) drm_format: Option<String>,
}

impl CacheEpoch {
    pub(crate) fn from_caps(caps: &gstreamer::CapsRef) -> Option<Self> {
        let structure = caps.structure(0)?;
        Some(Self {
            width: structure.get::<i32>("width").ok()? as u32,
            height: structure.get::<i32>("height").ok()? as u32,
            format: structure.get::<String>("format").ok()?,
            drm_format: structure.get::<String>("drm-format").ok(),
        })
    }
}

/// 色度平面视图：交错（NV12/P010 单 UV 平面）或独立 U/V（YU12/I420）
pub(crate) enum ChromaViews {
    Interleaved(wgpu::TextureView),
    Planar(wgpu::TextureView, wgpu::TextureView),
    Packed,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum PlaneRole {
    Luma,
    ChromaU,
    ChromaV,
    ChromaInterleaved,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct PlaneCacheKey {
    pub(crate) role: PlaneRole,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: wgpu::TextureFormat,
}

pub(crate) struct CachedPlane {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

pub(crate) fn chroma_views(
    layout: &PlaneLayout,
    views: &[wgpu::TextureView],
) -> Option<ChromaViews> {
    if views.is_empty() {
        return None;
    }
    if layout.is_packed() {
        return Some(ChromaViews::Packed);
    }
    if layout.is_planar() {
        // 按 chroma_order 映射 U/V：YV12 的平面顺序是 Y、V、U，views[1] 是 V。
        let Some((u_idx, v_idx)) = layout.planar_chroma_plane_indices() else {
            return None;
        };
        return Some(ChromaViews::Planar(
            views.get(u_idx)?.clone(),
            views.get(v_idx)?.clone(),
        ));
    }
    Some(ChromaViews::Interleaved(views.get(1)?.clone()))
}

/// 一帧的平面视图集合，供离屏 YUV→RGB pass 采样
pub(crate) struct FramePlanes {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// 显示宽度（= 存储宽度 × 像素宽高比 PAR 的水平换算）。PAR 跟随实际
    /// 输入，不再由 GStreamer 重采样成 1:1；UI 尺寸与参考框都以此为准。
    pub(crate) display_width: u32,
    pub(crate) y_view: wgpu::TextureView,
    pub(crate) chroma: ChromaViews,
    pub(crate) layout: PlaneLayout,
    pub(crate) matrix_is_601: bool,
    pub(crate) full_range: bool,
    /// 零拷贝导入成功（mmap/系统内存上传为 false，用于首帧诊断日志）
    pub(crate) zero_copy: bool,
}

/// 存储宽度 → 显示宽度：PAR=N/D 表示每个像素在水平方向是 N/D 个"方像素"，
/// 显示宽度 = round(storage × N / D)。PAR 无效或 1:1 时原样返回。
/// 高度不受 PAR 影响（PAR 只定义像素水平形状）。
pub(crate) fn display_width_from_par(storage_width: u32, par: gstreamer::Fraction) -> u32 {
    let (num, den) = (par.numer(), par.denom());
    if num <= 0 || den <= 0 || num == den {
        storage_width
    } else {
        ((storage_width as f64) * (num as f64) / (den as f64)).round() as u32
    }
}

/// Slint::Image 持有导入的 wgpu::Texture 句柄，因此输出纹理不能在提交给
/// Slint 后立即覆盖。固定大小的槽位池在 AfterRendering 后等待对应 queue
/// submission 完成，再允许槽位复用。
pub(crate) const OUTPUT_TEXTURE_POOL_SIZE: usize = 3;

pub(crate) struct OutputTextureSlot {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) in_flight: Arc<AtomicBool>,
}

impl SlintWGPUSink {
    /// 一帧 → RGBA slint::Image：按 caps 分派 dmabuf（零拷贝导入）
    /// 或 sysmem（map 后上传）路径，产出平面视图后共用离屏转色。
    pub(crate) fn sample_to_image(
        &mut self,
        device: &Arc<wgpu::Device>,
        queue: &Arc<wgpu::Queue>,
        sample: gstreamer::Sample,
    ) -> Option<slint::Image> {
        let caps = sample.caps()?.to_owned();
        let buffer = sample.buffer()?.to_owned();

        // VA surface 池重建（分辨率/格式切换等重协商）后旧 dmabuf fd 失效、新池
        // 分配新 buffer，缓存里的旧 VkImage 必须立即释放——池通常只有个位数
        // buffer，不显式清空会一直滞留到 LRU 上限。用 caps 代际 + DISCONT 标志
        // 在导入前识别重建时刻（同时防御内核复用 fd 号导致的 FrameKey 伪命中）。
        // 注意：检测放在渲染线程而非流线程——缓存只在"处理帧"时被查询，而池重建
        // 后到达的任何帧都携带新 caps，因此每帧对比必能捕获所有会引发伪命中的
        // 重协商（包括 dropping sink 丢帧场景）。
        let epoch = CacheEpoch::from_caps(caps.as_ref());
        if epoch != self.last_cache_epoch
            || buffer.flags().contains(gstreamer::BufferFlags::DISCONT)
        {
            if let Some(importer) = self.dmabuf.as_mut() {
                importer.clear();
            }
            self.last_cache_epoch = epoch;
        }

        let is_dmabuf = is_dma_drm_caps(caps.as_ref());
        let planes = if is_dmabuf {
            self.dmabuf_planes(device, queue, &caps, &buffer)
        } else {
            self.sysmem_planes(device, queue, &caps, &buffer)
        }?;

        // 记录显示尺寸变化（首次/切换），供 UI 调整尺寸。注意用 display_width：
        // PAR 非 1:1 时 UI 的世界尺寸/参考框必须以显示宽度为准（纹理仍是 width）。
        let size_changed = match self.last_video_size {
            Some((w, h)) => w != planes.display_width || h != planes.height,
            None => true,
        };
        if size_changed {
            self.last_video_size = Some((planes.display_width, planes.height));
            self.video_size_changed = true;
        }

        // Slint::Image 会在后续渲染提交中继续持有并采样这张纹理。使用固定
        // 槽位池；当前槽位在 AfterRendering 后等待 GPU 完成，避免覆盖旧帧。
        let (slot_index, out_tex, out_view) =
            self.acquire_output_texture(device, planes.width, planes.height)?;
        let Some(convert) = self.convert.as_ref() else {
            self.release_output_slot(slot_index);
            return None;
        };
        convert.run(device, queue, &planes, &out_view);

        // 零拷贝路径：dmabuf 由 VkImage 直接采样，GstBuffer（持有 VA surface
        // 占用）必须等 GPU 采样完才能归还 surface 池，否则解码器覆写显存
        if planes.zero_copy {
            let queue = queue.clone();
            queue.on_submitted_work_done(move || drop(sample));
        }

        match slint::Image::try_from(out_tex) {
            Ok(image) => {
                self.active_output_slot = Some(slot_index);
                Some(image)
            }
            Err(e) => {
                self.release_output_slot(slot_index);
                log::error!("转换为 Slint Image 失败: {:?}", e);
                None
            }
        }
    }

    fn acquire_output_texture(
        &mut self,
        device: &Arc<wgpu::Device>,
        width: u32,
        height: u32,
    ) -> Option<(usize, wgpu::Texture, wgpu::TextureView)> {
        for index in 0..self.output_pool.len() {
            let slot = &self.output_pool[index];
            if slot
                .in_flight
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                if slot.texture.width() == width && slot.texture.height() == height {
                    return Some((index, slot.texture.clone(), slot.view.clone()));
                }
                slot.in_flight.store(false, Ordering::Release);
            }
        }

        if self.output_pool.len() >= OUTPUT_TEXTURE_POOL_SIZE {
            // 分辨率切换后，只有所有旧尺寸槽位都空闲才能整池替换。
            // 仍在使用的旧槽位不能提前销毁，否则 Slint 的旧 Image 可能悬空。
            if self
                .output_pool
                .iter()
                .any(|slot| slot.in_flight.load(Ordering::Acquire))
            {
                log::trace!("RGBA 输出纹理池暂时没有可用槽位，丢弃一帧");
                return None;
            }
            self.output_pool.clear();
        }

        let texture = self.create_rgba_output(device, width, height);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let in_flight = Arc::new(AtomicBool::new(true));
        let index = self.output_pool.len();
        self.output_pool.push(OutputTextureSlot {
            texture: texture.clone(),
            view: view.clone(),
            in_flight,
        });
        Some((index, texture, view))
    }

    fn release_output_slot(&self, index: usize) {
        if let Some(slot) = self.output_pool.get(index) {
            slot.in_flight.store(false, Ordering::Release);
        }
    }

    /// dmabuf 帧 → 平面视图：每 plane 一个 fd，使用 Vulkan 外部内存零拷贝导入。
    pub(crate) fn dmabuf_planes(
        &mut self,
        device: &Arc<wgpu::Device>,
        _queue: &Arc<wgpu::Queue>,
        caps: &gstreamer::Caps,
        buffer: &gstreamer::Buffer,
    ) -> Option<FramePlanes> {
        let caps_ref = caps.as_ref();
        let Ok(info) = VideoInfoDmaDrm::from_caps(caps_ref) else {
            log::error!("解析 DMA_DRM caps 失败");
            return None;
        };
        let Some(layout) = dmabuf::plane_layout(info.fourcc()) else {
            log::warn!("不支持的 dmabuf fourcc 0x{:08x}，丢弃当前帧", info.fourcc());
            return None;
        };
        let width = info.width();
        let height = info.height();
        if width == 0 || height == 0 {
            log::warn!("dmabuf 帧尺寸无效: {width}x{height}");
            return None;
        }
        let colorimetry = info.colorimetry();

        // VideoMeta 提供各平面的全局 offset/stride；底层既可能是整帧单 fd，
        // 也可能是每平面独立 fd，必须按 offset 定位对应的 GstMemory。
        let Some(meta) = buffer.meta::<VideoMeta>() else {
            log::error!("dmabuf 帧缺少 VideoMeta");
            return None;
        };
        let strides = meta.stride();
        let offsets = meta.offset();
        let n_planes = 1 + layout.chroma_plane_count();
        if strides.len() < n_planes || offsets.len() < n_planes {
            log::error!("dmabuf 帧平面元数据不完整（需要 {n_planes} 平面）");
            return None;
        }
        let (cw, ch) = (
            width.div_ceil(layout.chroma_width_divisor),
            height.div_ceil(layout.chroma_height_divisor),
        );
        // 各平面尺寸与 wgpu 格式：Y 全幅，色度尺寸由布局决定。
        let plane_desc = |i: usize| -> (u32, u32, wgpu::TextureFormat) {
            match i {
                0 => (width, height, layout.y_format),
                _ => (cw, ch, layout.chroma_format),
            }
        };

        // 尝试零拷贝导入：支持任意 modifier（tiled / linear），
        // 由 wgpu 30 原生 texture_from_dmabuf_fd（VK_EXT_image_drm_format_modifier）处理。
        // iHD 可能以单 fd 导出整帧（平面按 offset 切分），其他实现也可能按
        // 平面导出多个 fd；每个 PlaneDesc 均保存自己的 fd 与 fd 内偏移。
        if let Some(importer) = self.dmabuf.as_mut() {
            let mut descs = Vec::with_capacity(n_planes);
            for i in 0..n_planes {
                let (w, h, fmt) = plane_desc(i);
                let plane = dmabuf_plane(buffer, offsets[i], strides[i], i)?;
                descs.push(super::dmabuf::PlaneDesc {
                    fd: plane.fd,
                    offset: plane.offset,
                    stride: plane.stride,
                    width: w,
                    height: h,
                    format: fmt,
                    modifier: info.modifier(),
                });
            }
            if let Some(views) = importer.import_frame(device, info.fourcc(), &descs)
                && views.len() == n_planes
            {
                let chroma = chroma_views(&layout, &views)?;
                return Some(FramePlanes {
                    width,
                    height,
                    display_width: display_width_from_par(width, info.par()),
                    y_view: views[0].clone(),
                    chroma,
                    layout,
                    matrix_is_601: colorimetry_is_601(&colorimetry),
                    full_range: colorimetry_is_full(&colorimetry),
                    zero_copy: true,
                });
            }
        }

        log::warn!(
            "DMA-BUF Vulkan 导入失败: fourcc=0x{:08x}, modifier=0x{:x}, stride={strides:?}, offset={offsets:?}",
            info.fourcc(),
            info.modifier()
        );
        None
    }

    /// sysmem 帧 → 平面视图：map 整 buffer 后按平面偏移上传。
    /// 平面格式由 GStreamer 协商结果决定。4:2:0 保留 NV12/P010/I420/YV12，
    /// 4:2:2 使用 Y42B，4:4:4 使用 Y444 或 packed Y410，不在 sink 内强制降采样。
    pub(crate) fn sysmem_planes(
        &mut self,
        device: &Arc<wgpu::Device>,
        queue: &Arc<wgpu::Queue>,
        caps: &gstreamer::Caps,
        buffer: &gstreamer::Buffer,
    ) -> Option<FramePlanes> {
        let info = VideoInfo::from_caps(caps.as_ref()).ok()?;
        let layout = sysmem_plane_layout(info.format())?;
        let (width, height) = (info.width(), info.height());
        if width == 0 || height == 0 {
            log::warn!("sysmem 帧尺寸无效: {width}x{height}");
            return None;
        }
        // VideoMeta 描述实际的分配器/VA surface 行布局。部分 VA 下载路径的
        // buffer payload 尺寸只覆盖已写入的数据，不包含 surface 尾部的对齐空间，
        // 所以不能用 buffer.size() 反推紧凑行距；否则每行都会错位并产生条纹。
        // 没有 VideoMeta 时才回退到 caps 推导值。
        let (strides, offsets) = match buffer.meta::<VideoMeta>() {
            Some(meta) => (meta.stride().to_vec(), meta.offset().to_vec()),
            None => (info.stride().to_vec(), info.offset().to_vec()),
        };
        let n_planes = 1 + layout.chroma_plane_count();
        if strides.len() < n_planes || offsets.len() < n_planes {
            log::error!("帧平面元数据不完整（需要 {n_planes} 平面）");
            return None;
        }

        let map = buffer.map_readable().ok()?;
        let data = map.as_slice();

        if (0..n_planes).any(|i| {
            let rows = plane_rows(&layout, height, i);
            plane_rows_avail(
                data.len(),
                offsets[i],
                u32::try_from(strides[i]).unwrap_or(0),
                rows,
            ) < rows
        }) {
            log::debug!(
                "sysmem 平面行数被 payload 截断，钳制后渲染: stride={strides:?} offset={offsets:?} payload={}B",
                data.len()
            );
        }

        let y_stride = positive_stride(strides[0], 0)?;
        let y_off = offsets[0];
        let y_rows = plane_rows_avail(data.len(), y_off, y_stride, height);
        let y_range = plane_range(data.len(), y_off, strides[0], y_rows, 0)?;

        let y_view = self.upload_plane(
            device,
            queue,
            PlaneRole::Luma,
            width,
            y_rows,
            layout.y_format,
            &data[y_range],
            y_stride,
        )?;

        // Y410 是单平面 packed 32-bit：Y/U/V 各 10 bit，最高 2 bit 为 alpha。
        // 绑定组仍需要色度槽位，但 shader 在 MODE_Y410 下不会读取它。
        if layout.is_packed() {
            return Some(FramePlanes {
                width,
                height,
                display_width: display_width_from_par(width, info.par()),
                y_view,
                chroma: ChromaViews::Packed,
                layout,
                matrix_is_601: colorimetry_is_601(&info.colorimetry()),
                full_range: colorimetry_is_full(&info.colorimetry()),
                zero_copy: false,
            });
        }

        // 色度：交错（NV12/P010 单 UV 平面）或平面（I420/YU12 独立 U/V 平面）。
        // 平面式布局按 chroma_order 取 U/V：YV12 的平面顺序是 Y、V、U。
        let chroma = if layout.is_planar() {
            let (cw, ch) = (
                width.div_ceil(layout.chroma_width_divisor),
                height.div_ceil(layout.chroma_height_divisor),
            );
            let (u_idx, v_idx) = layout.planar_chroma_plane_indices().unwrap_or((1, 2));
            let u_stride = positive_stride(strides[u_idx], u_idx)?;
            let v_stride = positive_stride(strides[v_idx], v_idx)?;
            let u_off = offsets[u_idx];
            let v_off = offsets[v_idx];
            let u_rows = plane_rows_avail(data.len(), u_off, u_stride, ch);
            let v_rows = plane_rows_avail(data.len(), v_off, v_stride, ch);
            let u_range = plane_range(data.len(), u_off, strides[u_idx], u_rows, u_idx)?;
            let v_range = plane_range(data.len(), v_off, strides[v_idx], v_rows, v_idx)?;
            let u_view = self.upload_plane(
                device,
                queue,
                PlaneRole::ChromaU,
                cw,
                u_rows,
                layout.chroma_format,
                &data[u_range],
                u_stride,
            )?;
            let v_view = self.upload_plane(
                device,
                queue,
                PlaneRole::ChromaV,
                cw,
                v_rows,
                layout.chroma_format,
                &data[v_range],
                v_stride,
            )?;
            ChromaViews::Planar(u_view, v_view)
        } else {
            let uv_stride = positive_stride(strides[1], 1)?;
            let uv_off = offsets[1];
            let (uv_w, uv_h) = (
                width.div_ceil(layout.chroma_width_divisor),
                height.div_ceil(layout.chroma_height_divisor),
            );
            let uv_rows = plane_rows_avail(data.len(), uv_off, uv_stride, uv_h);
            let uv_range = plane_range(data.len(), uv_off, strides[1], uv_rows, 1)?;
            let uv_view = self.upload_plane(
                device,
                queue,
                PlaneRole::ChromaInterleaved,
                uv_w,
                uv_rows,
                layout.chroma_format,
                &data[uv_range],
                uv_stride,
            )?;
            ChromaViews::Interleaved(uv_view)
        };
        Some(FramePlanes {
            width,
            height,
            display_width: display_width_from_par(width, info.par()),
            y_view,
            chroma,
            layout,
            matrix_is_601: colorimetry_is_601(&info.colorimetry()),
            full_range: colorimetry_is_full(&info.colorimetry()),
            zero_copy: false,
        })
    }

    /// 上传一个平面到缓存纹理并返回视图（平面+尺寸+格式复用，不每帧重分配）
    fn upload_plane(
        &mut self,
        device: &Arc<wgpu::Device>,
        queue: &wgpu::Queue,
        role: PlaneRole,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        data: &[u8],
        stride: u32,
    ) -> Option<wgpu::TextureView> {
        // U/V can have identical dimensions and formats; include the semantic
        // plane role so planar layouts never alias the two chroma textures.
        let key = PlaneCacheKey {
            role,
            width,
            height,
            format,
        };
        let (tex, view) = match self.plane_cache.get(&key) {
            Some(cached_plane) => (cached_plane.texture.clone(), cached_plane.view.clone()),
            None => {
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("video-plane"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                self.plane_cache.insert(
                    key,
                    CachedPlane {
                        texture: tex.clone(),
                        view: view.clone(),
                    },
                );
                (tex, view)
            }
        };
        if Self::write_plane(queue, &tex, data, stride, width, height, format) {
            Some(view)
        } else {
            None
        }
    }

    /// 处理 `COPY_BYTES_PER_ROW_ALIGNMENT=256` 约束：非对齐 stride 需垫零后上传。
    /// stride/数据均为字节数，与格式无关（P010 的 16bit 样本按 2 字节/像素折算）。
    fn write_plane(
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        data: &[u8],
        stride: u32,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> bool {
        const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let Some(bytes_per_pixel) = bytes_per_pixel(format) else {
            log::warn!("不支持的平面纹理格式: {format:?}");
            return false;
        };
        let Some(min_stride) = bytes_per_pixel.checked_mul(width) else {
            log::warn!("平面行距计算溢出: width={width}, format={format:?}");
            return false;
        };
        if stride < min_stride {
            log::warn!("平面行距不足: stride={stride}, required={min_stride}");
            return false;
        }
        let Some(required_len) = (stride as usize).checked_mul(height as usize) else {
            log::warn!("平面数据长度计算溢出");
            return false;
        };
        if data.len() < required_len {
            log::warn!("平面数据不足: got={}, required={required_len}", data.len());
            return false;
        }
        if stride.is_multiple_of(ALIGN) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            return true;
        }

        let Some(aligned) = stride.div_ceil(ALIGN).checked_mul(ALIGN) else {
            log::warn!("对齐行距计算溢出");
            return false;
        };
        let Some(padded_len) = (aligned as usize).checked_mul(height as usize) else {
            log::warn!("对齐平面数据长度计算溢出");
            return false;
        };
        let mut padded = vec![0u8; padded_len];
        for y in 0..height as usize {
            let src = y * stride as usize;
            let dst = y * aligned as usize;
            padded[dst..dst + stride as usize].copy_from_slice(&data[src..src + stride as usize]);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        true
    }

    /// 创建可被 Slint 导入的独立 RGBA 输出纹理。
    ///
    /// `COPY_SRC` 是 Slint/wgpu 纹理跨渲染器提交时所需的可复制 usage；输出
    /// 纹理不能缓存复用，因为每个 Slint Image 可能跨帧继续采样自己的纹理。
    fn create_rgba_output(
        &self,
        device: &Arc<wgpu::Device>,
        width: u32,
        height: u32,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video-rgba-out"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
}

/// 把 GStreamer 协商出的 sysmem `VideoFormat` 映射为上传和 shader 所需的布局。
/// 这里不把 Y42B/Y444/YV12 归并到 I420，避免丢失色度采样比例或 U/V 顺序。
pub(crate) fn sysmem_plane_layout(format: gstreamer_video::VideoFormat) -> Option<PlaneLayout> {
    super::dmabuf::capability_for_video_format(format).map(|capability| capability.sysmem_layout)
}

pub(crate) struct DmabufPlane {
    pub(crate) fd: i32,
    pub(crate) offset: u64,
    pub(crate) stride: u32,
}

/// 根据 VideoMeta 的全局平面 offset 找到唯一承载该平面的 GstMemory，并返回
/// memory 内局部 offset。跨多个 GstMemory 的单个平面不适合直接导入或 mmap。
fn plane_memory_start(
    buffer: &gstreamer::BufferRef,
    offset: usize,
    plane: usize,
) -> Option<(usize, usize)> {
    let end = offset.checked_add(1)?;
    let Some((memory_range, local_offset)) = buffer.find_memory(offset..end) else {
        log::error!("视频平面 {plane} 起始偏移无法映射到 GstMemory");
        return None;
    };
    if memory_range.len() != 1 {
        log::warn!("视频平面 {plane} 起始偏移对应多个 GstMemory");
        return None;
    }
    Some((memory_range.start, local_offset))
}

pub(crate) fn dmabuf_plane(
    buffer: &gstreamer::BufferRef,
    offset: usize,
    stride: i32,
    plane: usize,
) -> Option<DmabufPlane> {
    let stride = positive_stride(stride, plane)?;
    // GstMemory::size() is the logical payload size, while a DMA-BUF fd can
    // have a larger aligned allocation described by VideoMeta. Locating the
    // memory with `stride * rows` therefore rejects valid VA surfaces (for
    // example, aligned 4K Y410 frames). Only the plane start must lie in the
    // GstBuffer; Vulkan validates the fd allocation during image import.
    let (memory_index, local_offset) = plane_memory_start(buffer, offset, plane)?;
    let Some(memory) = buffer
        .peek_memory(memory_index)
        .downcast_memory_ref::<DmaBufMemory>()
    else {
        log::warn!("视频平面 {plane} 不是 DMA-BUF memory");
        return None;
    };
    Some(DmabufPlane {
        fd: memory.fd(),
        offset: local_offset.try_into().ok()?,
        stride,
    })
}

pub(crate) fn positive_stride(stride: i32, plane: usize) -> Option<u32> {
    u32::try_from(stride)
        .ok()
        .filter(|&stride| stride > 0)
        .or_else(|| {
            log::warn!("视频平面 {plane} 行距无效: {stride}");
            None
        })
}

pub(crate) fn plane_range(
    data_len: usize,
    offset: usize,
    stride: i32,
    rows: u32,
    plane: usize,
) -> Option<Range<usize>> {
    let stride = positive_stride(stride, plane)? as usize;
    let size = stride.checked_mul(rows as usize)?;
    let end = offset.checked_add(size)?;
    if end > data_len {
        log::error!(
            "视频平面 {plane} 数据不足: offset={offset}, stride={stride}, rows={rows}, buffer={data_len}"
        );
        return None;
    }
    Some(offset..end)
}

fn plane_rows(layout: &PlaneLayout, height: u32, plane: usize) -> u32 {
    if plane == 0 {
        height
    } else {
        height.div_ceil(layout.chroma_height_divisor)
    }
}

/// 某平面在映射 payload 内实际可容纳的行数（截断/上游分配不对齐时小于逻辑行数）。
/// stride 为 0 或 offset 超出 payload 时返回 0。
pub(crate) fn plane_rows_avail(
    data_len: usize,
    offset: usize,
    stride: u32,
    logical_rows: u32,
) -> u32 {
    let Some(stride) = usize::try_from(stride).ok() else {
        return 0;
    };
    if stride == 0 {
        return 0;
    }
    let avail = data_len.saturating_sub(offset) / stride;
    u32::try_from(avail)
        .ok()
        .map(|rows| rows.min(logical_rows))
        .unwrap_or(0)
}

fn bytes_per_pixel(format: wgpu::TextureFormat) -> Option<u32> {
    match format {
        wgpu::TextureFormat::R8Unorm => Some(1),
        wgpu::TextureFormat::Rg8Unorm => Some(2),
        wgpu::TextureFormat::R16Unorm => Some(2),
        wgpu::TextureFormat::Rg16Unorm => Some(4),
        wgpu::TextureFormat::Rgba8Unorm => Some(4),
        wgpu::TextureFormat::Rgb10a2Unorm => Some(4),
        _ => None,
    }
}

fn colorimetry_is_601(c: &gstreamer_video::VideoColorimetry) -> bool {
    matches!(
        c.matrix(),
        gstreamer_video::VideoColorMatrix::Bt601
            | gstreamer_video::VideoColorMatrix::Smpte240m
            | gstreamer_video::VideoColorMatrix::Fcc
    )
}

fn colorimetry_is_full(c: &gstreamer_video::VideoColorimetry) -> bool {
    c.range() == gstreamer_video::VideoColorRange::Range0_255
}
