//! DMA-BUF 零拷贝导入：把 VA-API 导出的 dmabuf fd 直接包装成 wgpu 纹理
//! （wgpu 30 原生 `vulkan::Device::texture_from_dmabuf_fd`，背后是
//! `VK_EXT_external_memory_dma_buf` + `VK_EXT_image_drm_format_modifier`），
//! 跳过 GPU→CPU→GPU 两次拷贝与 CPU 端垫零上传。
//!
//! 支持任意 DRM format modifier（tiled / linear），每平面独立导入为 VkImage，
//! 通过 `texture_from_dmabuf_fd` 包成 wgpu 纹理。
//! GPU 硬件原生处理 tiled 布局的采样，无需 tile→linear 转换。
//!
//! 任一校验失败（扩展缺失 / 非 Vulkan 后端 / modifier 不支持）返回 `None`，
//! 上层回退到 mmap 上传路径。
//!
//! 导入结果按 fd、fourcc、modifier 和完整平面布局缓存：VA surface 池循环复用同一批
//! dmabuf，稳态下每个 buffer 只导入一次。淘汰不设固定容量——按实测池循环
//! 长度淘汰不再存活的孤儿条目；池重建时由上层显式清空。淘汰时 VkImage/内存
//! 由 wgpu 的 drop 机制统一销毁。

use std::collections::HashMap;
use std::os::unix::io::FromRawFd;
use std::sync::OnceLock;

use slint::wgpu_30::wgpu;

use gstreamer_video::VideoFormat;

/// 驱逐安全系数：条目空闲超过"实测池循环长度 × 该系数"即判定不再存活。
/// 池循环长度（同一 buffer 两次到达的间隔帧数）稳态下即 VA surface 池实际
/// 大小，是运行时实测值；此系数只吸收丢帧/渲染抖动，不决定容量本身。
/// 池重建（可观测的重协商）由上层按 caps 代际/DISCONT 显式 `clear()`，
/// 这里只兜底"无法观测到的池重建"产生的孤儿条目。
const IDLE_EVICT_MARGIN: u64 = 2;

/// 条目是否已空闲到可淘汰：未观测到任何完整循环前（max_reuse_gap=0）不淘汰，
/// 避免热身期误清；此后空闲超过实测循环 × 系数即判定不在存活集。
fn is_stale(now: u64, last_use: u64, max_reuse_gap: u64) -> bool {
    let threshold = max_reuse_gap.saturating_mul(IDLE_EVICT_MARGIN);
    threshold > 0 && now - last_use > threshold
}

// DRM fourcc 不在此手写，统一由 gstreamer-video 的 `dma_drm_fourcc_from_format`
// 推导（内部查 `gst_video_dma_drm_fourcc_from_format` 的 format_map 表），
// 见 `drm_fourcc_for`，避免与 GStreamer 映射脱节。

/// 着色器模式号（uniform 第二个 vec4 的 x 分量）
const MODE_NV12: u32 = 0;
const MODE_PLANAR: u32 = 1;
const MODE_P010: u32 = 2;
const MODE_Y410: u32 = 3;

/// wgpu 平面格式与帧布局信息：由 DRM fourcc 推导。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlaneLayout {
    /// Y 平面 wgpu 格式（8bit=R8Unorm / 10bit=R16Unorm）
    pub y_format: wgpu::TextureFormat,
    /// 色度平面 wgpu 格式：交错（RG8/RG16，1 平面）或平面（R8/R16，2 平面）
    pub chroma_format: wgpu::TextureFormat,
    /// 色度平面组织方式。
    pub storage: PlaneStorage,
    /// 每个色度平面的宽度是亮度平面的 `1 / chroma_width_divisor`。
    pub chroma_width_divisor: u32,
    /// 每个色度平面的高度是亮度平面的 `1 / chroma_height_divisor`。
    pub chroma_height_divisor: u32,
    /// 平面顺序是否为 Y、V、U（YV12）。
    pub chroma_order: ChromaOrder,
    /// 10bit（P010/Y410 类）与否（NV12/YU12 类为 8bit）。
    pub bit_depth: BitDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChromaOrder {
    Uv,
    Vu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BitDepth {
    Eight,
    Ten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaneStorage {
    Planar,
    Interleaved,
    Packed,
}

impl PlaneLayout {
    pub(crate) const fn planar(
        y_format: wgpu::TextureFormat,
        chroma_format: wgpu::TextureFormat,
        chroma_width_divisor: u32,
        chroma_height_divisor: u32,
        chroma_order: ChromaOrder,
    ) -> Self {
        Self {
            y_format,
            chroma_format,
            storage: PlaneStorage::Planar,
            chroma_width_divisor,
            chroma_height_divisor,
            chroma_order,
            bit_depth: BitDepth::Eight,
        }
    }

    pub(crate) const fn interleaved(
        y_format: wgpu::TextureFormat,
        chroma_format: wgpu::TextureFormat,
        chroma_width_divisor: u32,
        chroma_height_divisor: u32,
        bit_depth: BitDepth,
    ) -> Self {
        Self {
            y_format,
            chroma_format,
            storage: PlaneStorage::Interleaved,
            chroma_width_divisor,
            chroma_height_divisor,
            chroma_order: ChromaOrder::Uv,
            bit_depth,
        }
    }

    pub(crate) const fn packed(y_format: wgpu::TextureFormat) -> Self {
        Self {
            y_format,
            chroma_format: wgpu::TextureFormat::R8Unorm,
            storage: PlaneStorage::Packed,
            chroma_width_divisor: 1,
            chroma_height_divisor: 1,
            chroma_order: ChromaOrder::Uv,
            bit_depth: BitDepth::Ten,
        }
    }

    pub(crate) const fn is_planar(self) -> bool {
        matches!(self.storage, PlaneStorage::Planar)
    }

    pub(crate) const fn is_packed(self) -> bool {
        matches!(self.storage, PlaneStorage::Packed)
    }

    /// 着色器模式号（uniform 的 mode.x），由存储组织与位深推导，
    /// 避免各构造器分散维护。
    pub(crate) const fn shader_mode(self) -> u32 {
        match self.storage {
            PlaneStorage::Planar => MODE_PLANAR,
            PlaneStorage::Interleaved => match self.bit_depth {
                BitDepth::Eight => MODE_NV12,
                BitDepth::Ten => MODE_P010,
            },
            PlaneStorage::Packed => MODE_Y410,
        }
    }

    /// 色度平面数（交错=1：UV 交错单平面；平面=2：U、V 各一；packed=0）。
    pub(crate) const fn chroma_plane_count(self) -> usize {
        match self.storage {
            PlaneStorage::Planar => 2,
            PlaneStorage::Interleaved => 1,
            PlaneStorage::Packed => 0,
        }
    }

    /// 平面式布局中 U、V 平面各自的索引。GStreamer 的平面顺序跟随 DRM 布局：
    /// I420/YU12 为 Y、U、V（`Uv`），YV12 为 Y、V、U（`Vu`）。交错与 packed
    /// 布局没有独立 U/V 平面，返回 None。
    pub(crate) const fn planar_chroma_plane_indices(self) -> Option<(usize, usize)> {
        if !matches!(self.storage, PlaneStorage::Planar) {
            return None;
        }
        match self.chroma_order {
            ChromaOrder::Uv => Some((1, 2)),
            ChromaOrder::Vu => Some((2, 1)),
        }
    }
}

/// 视频格式能力注册表。sysmem 协商、sysmem 上传和 DMABuf 导入都从这里派生，
/// 避免格式名称、GStreamer 枚举和 DRM fourcc 分别维护而产生静默不一致。
pub(crate) struct VideoFormatCapability {
    // GStreamer 格式名（caps 字符串用）
    pub(crate) name: &'static str,
    // gstreamer-video 枚举（VideoInfo 解析结果）
    pub(crate) format: VideoFormat,
    pub(crate) drm_fourcc: Option<u32>,
    pub(crate) sysmem_layout: PlaneLayout,
    pub(crate) dmabuf_layout: Option<PlaneLayout>,
    pub(crate) requires_16bit_norm: bool,
}

/// 由 gstreamer-video 的格式↔DRM fourcc 映射表推导 fourcc（单一事实来源，
/// 即 `gst_video_dma_drm_fourcc_from_format` 内部 format_map，GStreamer 改了
/// 这里自动跟随）。
fn drm_fourcc_for(format: VideoFormat) -> Option<u32> {
    gstreamer_video::dma_drm_fourcc_from_format(format).ok()
}

/// ABGR8888 的 DRM fourcc（'AB24'）：单平面 packed RGBA，字节序与 wgpu
/// Rgba8Unorm 一致，是唯一满足 Slint `Image::try_from` 格式要求的直出 fourcc。
#[cfg(test)]
pub(crate) fn ab24_fourcc() -> u32 {
    drm_fourcc_for(gstreamer_video::VideoFormat::Rgba)
        .expect("VideoFormat::Rgba 必有对应 DRM fourcc")
}

pub(crate) fn video_format_capabilities() -> &'static [VideoFormatCapability] {
    static CAPABILITIES: OnceLock<Vec<VideoFormatCapability>> = OnceLock::new();
    CAPABILITIES.get_or_init(|| {
        vec![
            VideoFormatCapability {
                name: "Y410",
                format: VideoFormat::Y410,
                drm_fourcc: drm_fourcc_for(VideoFormat::Y410),
                sysmem_layout: PlaneLayout::packed(wgpu::TextureFormat::Rgba8Unorm),
                dmabuf_layout: Some(PlaneLayout::packed(wgpu::TextureFormat::Rgb10a2Unorm)),
                requires_16bit_norm: false,
            },
            VideoFormatCapability {
                name: "Y444",
                format: VideoFormat::Y444,
                drm_fourcc: drm_fourcc_for(VideoFormat::Y444),
                sysmem_layout: PlaneLayout::planar(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::R8Unorm,
                    1,
                    1,
                    ChromaOrder::Uv,
                ),
                dmabuf_layout: None,
                requires_16bit_norm: false,
            },
            VideoFormatCapability {
                name: "Y42B",
                format: VideoFormat::Y42b,
                drm_fourcc: drm_fourcc_for(VideoFormat::Y42b),
                sysmem_layout: PlaneLayout::planar(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::R8Unorm,
                    2,
                    1,
                    ChromaOrder::Uv,
                ),
                dmabuf_layout: Some(PlaneLayout::planar(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::R8Unorm,
                    2,
                    1,
                    ChromaOrder::Uv,
                )),
                requires_16bit_norm: false,
            },
            VideoFormatCapability {
                name: "P010_10LE",
                format: VideoFormat::P01010le,
                drm_fourcc: drm_fourcc_for(VideoFormat::P01010le),
                sysmem_layout: PlaneLayout::interleaved(
                    wgpu::TextureFormat::R16Unorm,
                    wgpu::TextureFormat::Rg16Unorm,
                    2,
                    2,
                    BitDepth::Ten,
                ),
                dmabuf_layout: Some(PlaneLayout::interleaved(
                    wgpu::TextureFormat::R16Unorm,
                    wgpu::TextureFormat::Rg16Unorm,
                    2,
                    2,
                    BitDepth::Ten,
                )),
                requires_16bit_norm: true,
            },
            VideoFormatCapability {
                name: "I420",
                format: VideoFormat::I420,
                drm_fourcc: drm_fourcc_for(VideoFormat::I420),
                sysmem_layout: PlaneLayout::planar(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::R8Unorm,
                    2,
                    2,
                    ChromaOrder::Uv,
                ),
                dmabuf_layout: Some(PlaneLayout::planar(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::R8Unorm,
                    2,
                    2,
                    ChromaOrder::Uv,
                )),
                requires_16bit_norm: false,
            },
            VideoFormatCapability {
                name: "YV12",
                format: VideoFormat::Yv12,
                drm_fourcc: drm_fourcc_for(VideoFormat::Yv12),
                sysmem_layout: PlaneLayout::planar(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::R8Unorm,
                    2,
                    2,
                    ChromaOrder::Vu,
                ),
                dmabuf_layout: Some(PlaneLayout::planar(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::R8Unorm,
                    2,
                    2,
                    ChromaOrder::Vu,
                )),
                requires_16bit_norm: false,
            },
            VideoFormatCapability {
                name: "NV12",
                format: VideoFormat::Nv12,
                drm_fourcc: drm_fourcc_for(VideoFormat::Nv12),
                sysmem_layout: PlaneLayout::interleaved(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::Rg8Unorm,
                    2,
                    2,
                    BitDepth::Eight,
                ),
                dmabuf_layout: Some(PlaneLayout::interleaved(
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureFormat::Rg8Unorm,
                    2,
                    2,
                    BitDepth::Eight,
                )),
                requires_16bit_norm: false,
            },
        ]
    })
}

pub(crate) fn sysmem_capabilities(
    ten_bit: bool,
) -> impl Iterator<Item = &'static VideoFormatCapability> {
    video_format_capabilities()
        .iter()
        .filter(move |capability| ten_bit || !capability.requires_16bit_norm)
}

pub(crate) fn capability_for_video_format(
    format: VideoFormat,
) -> Option<&'static VideoFormatCapability> {
    video_format_capabilities()
        .iter()
        .find(|capability| capability.format == format)
}

/// 按 DRM fourcc 解析支持的平面布局；不支持的格式返回 None 走回退。
/// 注意 iHD 驱动下 NV12/P010 只能以 Intel CCS modifier 导出（wgpu 无法导入），
/// 唯一线性导出的是 YU12（I420 三平面）——零拷贝路径因此请求 YU12。
pub(crate) fn plane_layout(fourcc: u32) -> Option<PlaneLayout> {
    video_format_capabilities()
        .iter()
        .find(|capability| capability.drm_fourcc == Some(fourcc))
        .and_then(|capability| capability.dmabuf_layout)
}

/// 从 GStreamer 的 `drm-format` 字符串解析 fourcc。
/// 字符串可能带运行时 modifier，例如 `NV12:0x0100000000000009`。
/// 优先用 gstreamer-video 的 `dma_drm_fourcc_from_str`（库内部解析
/// `fourcc:modifier`）；对无修饰符的裸 fourcc 回退到标准库的小端打包，
/// 避免手写位运算。
pub(crate) fn plane_layout_from_drm_format(name: &str) -> Option<PlaneLayout> {
    let code = match gstreamer_video::dma_drm_fourcc_from_str(name) {
        Ok((fourcc, _)) => fourcc,
        Err(_) => {
            let bytes: [u8; 4] = name.as_bytes().try_into().ok()?;
            u32::from_le_bytes(bytes)
        }
    };
    plane_layout(code)
}

/// 单个平面的导入描述（由 VideoMeta + VADRMPRIMESurfaceDescriptor 提取）
pub(crate) struct PlaneDesc {
    /// 导出该平面的 dmabuf fd；单 fd 布局下各平面可以复用同一个 fd。
    pub fd: i32,
    /// dmabuf 内偏移（字节）
    pub offset: u64,
    /// 每行字节数
    pub stride: u32,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    /// DRM format modifier（由 VA-API 导出时报告，如 LINEAR / Y_TILED / 4_TILED 等）
    pub modifier: u64,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct FrameKey {
    fourcc: u32,
    planes: Vec<PlaneKey>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct PlaneKey {
    fd: i32,
    offset: u64,
    stride: u32,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    modifier: u64,
}

impl From<&PlaneDesc> for PlaneKey {
    fn from(plane: &PlaneDesc) -> Self {
        Self {
            fd: plane.fd,
            offset: plane.offset,
            stride: plane.stride,
            width: plane.width,
            height: plane.height,
            format: plane.format,
            modifier: plane.modifier,
        }
    }
}

struct CachedFrame {
    /// 各平面 (纹理, 视图)；VkImage/内存由 wgpu 的 drop 机制统一释放
    planes: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// 最近一次被使用的帧序号（`DmabufImporter::frame_index`）
    last_use: u64,
}

/// dmabuf → wgpu::Texture 导入器。`try_new` 校验 wgpu 后端为 Vulkan 且设备
/// 启用了 `VULKAN_EXTERNAL_MEMORY_DMA_BUF` 特性；失败（如 GL 后端）则上层
/// 全程走回退路径。实际导入委托给 wgpu 30 原生 `texture_from_dmabuf_fd`。
///
/// 缓存淘汰不设固定容量：稳态下条目数天然等于 VA surface 池大小（key 集稳定），
/// 池重建由上层 `clear()` 显式清空。这里只测量"实测池循环长度"（同一 buffer
/// 两次到达的间隔帧数），淘汰空闲超过它、已不在存活集的孤儿条目。
pub(crate) struct DmabufImporter {
    cache: HashMap<FrameKey, CachedFrame>,
    /// 已处理的导入帧计数（渲染线程逐帧推进），用于计算条目空闲时长。
    frame_index: u64,
    /// 实测的最大池循环长度（同一 key 两次命中之间的最大间隔帧数）。
    max_reuse_gap: u64,
}

impl DmabufImporter {
    pub(crate) fn try_new(device: &wgpu::Device) -> Option<Self> {
        // 仅在 Vulkan 后端返回 Some（texture_from_dmabuf_fd 需要 hal::vulkan::Device）
        if (unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }).is_none() {
            log::debug!("dmabuf 导入不可用：当前 wgpu 后端不是 Vulkan");
            return None;
        }
        if !device
            .features()
            .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
        {
            log::debug!("dmabuf 导入不可用：Vulkan device 未启用 VULKAN_EXTERNAL_MEMORY_DMA_BUF");
            return None;
        }
        log::info!("✅ dmabuf 零拷贝导入器就绪（wgpu 30 texture_from_dmabuf_fd）");
        Some(Self {
            cache: HashMap::new(),
            frame_index: 0,
            max_reuse_gap: 0,
        })
    }

    /// 当前缓存的导入帧数（测试用）
    #[cfg(test)]
    pub(crate) fn cached_frames(&self) -> usize {
        self.cache.len()
    }

    /// 实测的池循环长度（测试用）：稳态下 ≈ VA surface 池大小。
    #[cfg(test)]
    pub(crate) fn measured_reuse_gap(&self) -> u64 {
        self.max_reuse_gap
    }

    /// 清空导入缓存：VA surface 池重建（分辨率/格式切换等重协商）后旧 fd 失效、
    /// 新池分配新 buffer，旧池的 VkImage 立即无用。此刻显式释放所有陈旧条目。
    /// 同时防御内核复用 fd 号导致的 `FrameKey` 伪命中。顺带重置池循环测量，
    /// 让新池从零开始观测。
    pub(crate) fn clear(&mut self) {
        self.cache.clear();
        self.max_reuse_gap = 0;
    }

    /// 淘汰已不在存活集的孤儿条目：空闲超过实测池循环 × 系数即淘汰。
    /// 在 miss 路径调用（稳态命中时不会产生孤儿，无需扫描）。
    fn evict_stale(&mut self) {
        let frame_index = self.frame_index;
        let max_reuse_gap = self.max_reuse_gap;
        self.cache
            .retain(|_, frame| !is_stale(frame_index, frame.last_use, max_reuse_gap));
    }

    /// 导入（或命中缓存）一个 dmabuf 帧，返回各平面纹理视图。
    /// `planes` 为完整平面布局（Y + 色度），全部成功才返回 Some。
    pub(crate) fn import_frame(
        &mut self,
        device: &wgpu::Device,
        fourcc: u32,
        planes: &[PlaneDesc],
    ) -> Option<Vec<wgpu::TextureView>> {
        if planes.is_empty() {
            log::warn!("dmabuf 帧没有平面，跳过导入");
            return None;
        }
        if planes
            .iter()
            .any(|plane| plane.fd < 0 || plane.stride == 0 || plane.width == 0 || plane.height == 0)
        {
            log::warn!("dmabuf 平面描述无效，跳过导入");
            return None;
        }
        let key = FrameKey {
            fourcc,
            planes: planes.iter().map(PlaneKey::from).collect(),
        };
        // 每处理一帧推进计数；命中时顺便更新实测池循环长度。
        self.frame_index += 1;
        if let Some(frame) = self.cache.get_mut(&key) {
            let reuse_gap = self.frame_index - frame.last_use;
            self.max_reuse_gap = self.max_reuse_gap.max(reuse_gap);
            frame.last_use = self.frame_index;
            return Some(frame.planes.iter().map(|(_, v)| v.clone()).collect());
        }
        // 未命中：先淘汰空闲超时的孤儿条目，再导入新帧。
        self.evict_stale();
        let imported = self.import_uncached(device, planes)?;
        let views: Vec<wgpu::TextureView> = imported.iter().map(|(_, v)| v.clone()).collect();
        self.cache.insert(
            key,
            CachedFrame {
                planes: imported,
                last_use: self.frame_index,
            },
        );
        Some(views)
    }

    /// 单帧导入：每平面独立 fd → wgpu 30 原生 `texture_from_dmabuf_fd` →
    /// `create_texture_from_hal` 包成 wgpu 纹理。任一步失败返回 None，由上层丢弃
    /// 当前 DMA-BUF 帧；DMA-BUF 不能按普通系统内存映射读取。
    fn import_uncached(
        &mut self,
        device: &wgpu::Device,
        planes: &[PlaneDesc],
    ) -> Option<Vec<(wgpu::Texture, wgpu::TextureView)>> {
        let Some(hal_guard) = (unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }) else {
            return None;
        };

        let mut result = Vec::with_capacity(planes.len());
        for (plane_index, desc) in planes.iter().enumerate() {
            // 单 fd 布局下各平面复用同一 fd：texture_from_dmabuf_fd 会取得 fd 的所有权
            //（成功则归 Vulkan、失败则关闭），因此每平面必须独立 dup 一个副本，
            // 避免同一 fd 号被两个 OwnedFd 重复接管（double-close）。
            let dup_fd = unsafe { libc::dup(desc.fd) };
            if dup_fd < 0 {
                log::warn!("dup dmabuf fd（平面）失败");
                return None;
            }
            let fd = unsafe { std::os::unix::io::OwnedFd::from_raw_fd(dup_fd) };
            let hal_desc = wgpu::hal::TextureDescriptor {
                label: Some("video-dmabuf-plane"),
                size: wgpu::Extent3d {
                    width: desc.width,
                    height: desc.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: desc.format,
                usage: wgpu::TextureUses::RESOURCE,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: Vec::new(),
            };
            // 原生调用在内部创建 DRM_FORMAT_MODIFIER_EXT 的 VkImage 并绑定导入内存；
            // 失败（驱动不支持 modifier/内存类型）时返回 Err，此处整体丢弃该帧。
            let hal_tex = match unsafe {
                hal_guard.texture_from_dmabuf_fd(
                    fd,
                    &hal_desc,
                    desc.modifier,
                    u64::from(desc.stride),
                    desc.offset,
                )
            } {
                Ok(texture) => texture,
                Err(error) => {
                    log::warn!(
                        "DMA-BUF 平面导入失败: plane={plane_index} fd={} format={:?} modifier=0x{:x} stride={} offset={} error={error:?}",
                        desc.fd,
                        desc.format,
                        desc.modifier,
                        desc.stride,
                        desc.offset,
                    );
                    return None;
                }
            };
            let tex = unsafe {
                device.create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                    hal_tex,
                    &wgpu::TextureDescriptor {
                        label: Some("video-dmabuf-plane"),
                        size: hal_desc.size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: desc.format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                )
            };
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            result.push((tex, view));
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 驱逐判定随实测池循环自适应：未测到循环前不淘汰；空闲超过循环×系数才淘汰。
    #[test]
    fn test_is_stale_follows_measured_reuse_gap() {
        // 热身期：max_reuse_gap=0（还没观测到完整循环）→ 永不淘汰
        assert!(!is_stale(10, 9, 0));
        assert!(!is_stale(10, 1, 0));

        // 池循环=3（实测），系数=2 → 阈值 6
        // 空闲 ≤ 6：仍在存活集（含刚超过一个循环、即将被复用的条目）
        assert!(!is_stale(10, 4, 3));
        assert!(!is_stale(10, 5, 3));
        // 空闲 > 6：孤儿
        assert!(is_stale(10, 3, 3));

        // 池变大（循环=16）：阈值自动放宽到 32，不再误清
        assert!(!is_stale(40, 9, 16));
        assert!(is_stale(40, 4, 16));
    }
}
