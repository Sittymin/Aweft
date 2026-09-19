//! ASS 字形块 → 一张整帧 RGBA 纹理。
//!
//! 两件事决定这里的形状：
//! - 输出是**整帧**，不是图块包围盒。整帧尺寸 = 光栅尺寸（覆盖层矩形在屏幕上的
//!   物理像素数，见 `pipeline::raster_size`）。覆盖层因此与视口里的主视频矩形
//!   1:1，不需要任何偏移换算；没有换算就没有换算出错的可能。
//! - 所有 wgpu 资源常驻：管线、采样器、投影 uniform、alpha 图集纹理与它的暂存
//!   缓冲、实例缓冲、输出槽位池。逐帧新建纹理在 Vulkan 上的开销远大于字幕本身
//!   那点混色量（视频通路早有同一结论，见 `docs/architecture/video-playback.md`
//!   的「输出 RGBA 纹理使用固定大小的槽位池」）。
//!
//! 图集按本帧需要**长大**（只增不减）：光栅倍率到 2× 时一行字幕的字形块就能超过
//! 初值宽度。宽必须凑到 `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` 的整数倍，这样暂存
//! 缓冲每行字节数恒等于宽度，行内不需要额外对齐。
//!
//! 输入的 `Tile` 来自 `aweft_ass::render`：本文件不知道 libass 的存在。

use std::sync::Arc;

use anyhow::{Result, anyhow};
use aweft_ass::render::Tile;
use slint::wgpu_30::wgpu;

use crate::render::output_pool::OutputSlotPool;
use crate::render::video_sink::WgpuHandles;

/// 图集初值与单边上限。上限是 R8 纹理 64MB，实际到不了：光栅像素预算
/// （`pipeline::RASTER_PIXEL_BUDGET`）先约束了块尺寸。
const ATLAS_INITIAL: (u32, u32) = (2048, 1024);
const ATLAS_MAX_SIDE: u32 = 8192;

/// 图集宽度凑到暂存行对齐的整数倍（初值即满足，放大时也要保持）
fn atlas_width_bucket(width: u32) -> u32 {
    width.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// 实例缓冲的初始槽位数；只增不减。
const INSTANCE_CAPACITY_STEP: usize = 64;

/// 一个字形块的实例数据。字段与顺序必须和 `subtitle.wgsl` 的 `InstanceData` 一致。
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceData {
    /// 不透明度语义的颜色（`Tile::color` 已把 libass 的透明度约定翻过来）
    color: [f32; 4],
    /// 块左上角在整帧内的像素坐标
    position: [f32; 2],
    size: [f32; 2],
    /// 块在图集里的 UV 偏移与尺寸（0..1）
    atlas_uv_offset: [f32; 2],
    atlas_uv_size: [f32; 2],
}

impl InstanceData {
    fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        // 偏移量对应上面五个字段的内存布局：4·f32, 2·f32, 2·f32, 2·f32, 2·f32
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceData>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 40,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// 一次合成的结果。三种情况在 UI 上是不同动作，不能合成一个 Option 表达。
pub(crate) enum Composition {
    /// 整帧尺寸的一张纹理，可直接交给覆盖层的 `slint::Image`
    Frame(slint::Image),
    /// 这一帧没有字形块：覆盖层清空
    Empty,
    /// 输出槽位全忙：这一帧不提交，覆盖层保持上一帧
    Skipped,
}

/// 字形块在图集里的落点
struct AtlasSlot {
    /// 在 `tiles` 里的下标
    index: usize,
    x: u32,
    y: u32,
}

pub(crate) struct SubtitleCompositor {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    render_pipeline: wgpu::RenderPipeline,
    projection_buffer: wgpu::Buffer,
    /// 投影 uniform 当前对应的光栅尺寸；不一致时先重写
    projected_size: (u32, u32),
    /// 光栅尺寸 = 覆盖层在屏幕上的物理像素数 = 输出纹理尺寸 = libass 的渲染帧
    raster_size: (u32, u32),
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas_texture: wgpu::Texture,
    atlas_staging: wgpu::Buffer,
    /// 图集当前容量（宽恒为暂存行对齐的整数倍），只增不减
    atlas_width: u32,
    atlas_height: u32,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    /// 图集上传用的 CPU 侧缓冲，按本帧实际行数裁剪后写进暂存缓冲
    scratch: Vec<u8>,
    target: OutputSlotPool,
}

impl SubtitleCompositor {
    /// 用 Slint 的 GPU 句柄建好全部常驻资源。光栅尺寸另由 `set_raster_size` 给。
    pub(crate) fn new(handles: &WgpuHandles) -> Self {
        let (_, device, queue) = handles;
        let device = Arc::clone(device);
        let queue = Arc::clone(queue);

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("subtitle-bind-group-layout"),
            entries: &[
                // 正交投影：帧像素坐标 → 裁剪空间，只有顶点着色器用
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 图集：每个块的形状覆盖
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("subtitle-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("subtitle.wgsl"));
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("subtitle-render-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(InstanceData::vertex_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    // 与 OutputSlotPool 建出的纹理格式一致
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // 字形覆盖已含抗锯齿，字幕纹理不再缩放，邻域采样即可
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("subtitle-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let projection_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("subtitle-projection"),
            size: std::mem::size_of::<[[f32; 4]; 4]>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (atlas_texture, atlas_staging) =
            Self::create_atlas_resources(&device, ATLAS_INITIAL.0, ATLAS_INITIAL.1);
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = make_bind_group(
            &device,
            &bind_group_layout,
            &projection_buffer,
            &sampler,
            &atlas_view,
        );
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("subtitle-instances"),
            size: (INSTANCE_CAPACITY_STEP * std::mem::size_of::<InstanceData>())
                as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device,
            queue,
            render_pipeline,
            projection_buffer,
            projected_size: (0, 0),
            raster_size: (0, 0),
            bind_group_layout,
            sampler,
            atlas_texture,
            atlas_staging,
            atlas_width: ATLAS_INITIAL.0,
            atlas_height: ATLAS_INITIAL.1,
            bind_group,
            instances,
            instance_capacity: INSTANCE_CAPACITY_STEP,
            scratch: Vec::new(),
            target: OutputSlotPool::new("subtitle-rgba-out"),
        }
    }

    /// 光栅尺寸 = 输出纹理尺寸 = libass 的渲染帧 = ASS 坐标所在的帧。
    /// 由 `pipeline::raster_size` 档化与钳制后给出；槽位池的尺寸切换由
    /// `OutputSlotPool::acquire` 负责（旧尺寸槽位等全部空闲）。
    pub(crate) fn set_raster_size(&mut self, size: (u32, u32)) {
        self.raster_size = size;
    }

    /// 释放字幕侧的 GPU 资源（渲染器销毁时调用）。
    pub(crate) fn teardown(&mut self) {
        self.target.clear();
        self.scratch.clear();
    }

    /// Slint 画完一轮场景后归还本帧的输出槽位。
    pub(crate) fn after_rendering(&mut self) {
        self.target.after_rendering(Some(&self.queue));
    }

    /// 池内输出纹理，供无头测试回读像素（不参与任何生产逻辑）。
    #[cfg(test)]
    pub(crate) fn target_textures(&self) -> impl Iterator<Item = &wgpu::Texture> {
        self.target.textures()
    }

    /// 图集当前容量，供无头测试断言「确实长大了」。
    #[cfg(test)]
    pub(crate) fn atlas_size(&self) -> (u32, u32) {
        (self.atlas_width, self.atlas_height)
    }

    /// 把本帧的字形块画进整帧目标。图集内容与实例数据逐帧重写，图集容量不足时
    /// 长大一档（只增不减），管线与缓冲都不重建。
    pub(crate) fn compose(&mut self, tiles: &[Tile]) -> Result<Composition> {
        let (frame_width, frame_height) = self.raster_size;
        if frame_width == 0 || frame_height == 0 {
            return Err(anyhow!("字幕合成器还没有光栅尺寸，无法确定输出纹理"));
        }

        let active: Vec<&Tile> = tiles
            .iter()
            .filter(|tile| tile.width > 0 && tile.height > 0)
            .collect();
        if active.is_empty() {
            return Ok(Composition::Empty);
        }

        let widest = active
            .iter()
            .fold(1u32, |widest, tile| widest.max(tile.width as u32));
        let Some((slots, extent)) = self.pack_atlas(&active, widest) else {
            return Ok(Composition::Skipped);
        };

        self.fill_atlas_scratch(&slots, &active, extent);
        self.write_instances(&slots, &active);
        self.write_projection(frame_width, frame_height);

        let Some((slot_index, texture, view)) =
            self.target.acquire(&self.device, frame_width, frame_height)
        else {
            return Ok(Composition::Skipped);
        };

        let mut encoder = self.device.create_command_encoder(&Default::default());
        // 图集拷贝排在字幕 pass 之前：同一 encoder 里的记录顺序即执行顺序
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &self.atlas_staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.atlas_width),
                    rows_per_image: Some(extent.1),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: extent.0,
                height: extent.1,
                depth_or_array_layers: 1,
            },
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subtitle-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.render_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            // 一个矩形两个三角形共 6 个顶点；每个实例取自己的块数据
            pass.draw(0..6, 0..active.len() as u32);
        }
        self.queue.submit(Some(encoder.finish()));

        match slint::Image::try_from(texture) {
            Ok(image) => {
                self.target.mark_active(slot_index);
                Ok(Composition::Frame(image))
            }
            Err(error) => {
                self.target.discard(slot_index);
                Err(anyhow!("字幕纹理交给 Slint 失败: {error:?}"))
            }
        }
    }

    /// 货架式排布：一行放不下就换行。行与行之间不留缝，所以用过的行就是
    /// 图集顶部的连续一段。
    fn atlas_slots(tiles: &[&Tile], atlas_width: u32) -> Vec<AtlasSlot> {
        let mut slots = Vec::with_capacity(tiles.len());
        let mut x = 0u32;
        let mut y = 0u32;
        let mut row_height = 0u32;

        for (index, tile) in tiles.iter().enumerate() {
            let width = tile.width as u32;
            let height = tile.height as u32;
            if x + width > atlas_width {
                x = 0;
                y += row_height;
                row_height = 0;
            }
            slots.push(AtlasSlot { index, x, y });
            x += width;
            row_height = row_height.max(height);
        }
        slots
    }

    /// 本帧实际用到的图集区域：宽到最右块的右边界，高到底部块的下边界。
    fn atlas_extent(slots: &[AtlasSlot], tiles: &[&Tile]) -> (u32, u32) {
        let mut width = 0u32;
        let mut height = 0u32;
        for slot in slots {
            let tile = tiles[slot.index];
            width = width.max(slot.x + tile.width as u32);
            height = height.max(slot.y + tile.height as u32);
        }
        (width.max(1), height.max(1))
    }

    /// 把本帧的块排进图集；当前容量装不下就长大一档重排。返回 None 表示图集已经
    /// 顶到上限仍装不下（放大倍率被预算钳住时理论上到不了这里），调用方丢帧。
    fn pack_atlas(&mut self, tiles: &[&Tile], widest: u32) -> Option<(Vec<AtlasSlot>, (u32, u32))> {
        loop {
            let slots = Self::atlas_slots(tiles, self.atlas_width);
            let extent = Self::atlas_extent(&slots, tiles);
            if extent.0 <= self.atlas_width && extent.1 <= self.atlas_height {
                return Some((slots, extent));
            }
            if !self.grow_atlas(widest, extent) {
                log::warn!(
                    "字幕字形块需要图集 {}x{}（最宽单块 {widest}），超出上限 {ATLAS_MAX_SIDE}，本帧跳过",
                    extent.0,
                    extent.1
                );
                return None;
            }
        }
    }

    /// 图集长大一档：宽凑到放得下最宽的单块（并保持行对齐整数倍），高翻倍。
    /// 只增不减——缩小省不下逐帧拷贝量（那个只跟 extent 走），却会把绑定组换来换去。
    /// 返回 false 表示两个方向都已顶到上限。
    fn grow_atlas(&mut self, widest: u32, extent: (u32, u32)) -> bool {
        let width = atlas_width_bucket(self.atlas_width.max(widest)).min(ATLAS_MAX_SIDE);
        let height = self
            .atlas_height
            .max(extent.1)
            .saturating_mul(2)
            .min(ATLAS_MAX_SIDE);
        if width == self.atlas_width && height == self.atlas_height {
            return false;
        }
        self.resize_atlas(width, height);
        true
    }

    /// 换图集容量：纹理、暂存缓冲与绑定组一起换。旧纹理由 wgpu 的引用计数兜住，
    /// 在途 submission 用完自然释放。
    fn resize_atlas(&mut self, width: u32, height: u32) {
        let (atlas_texture, atlas_staging) =
            Self::create_atlas_resources(&self.device, width, height);
        let view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = make_bind_group(
            &self.device,
            &self.bind_group_layout,
            &self.projection_buffer,
            &self.sampler,
            &view,
        );
        self.atlas_texture = atlas_texture;
        self.atlas_staging = atlas_staging;
        self.bind_group = bind_group;
        self.atlas_width = width;
        self.atlas_height = height;
        log::debug!("字幕图集扩大到 {width}x{height}");
    }

    /// 图集纹理与它的暂存缓冲。宽恒为行对齐的整数倍，所以缓冲大小就是宽×高。
    fn create_atlas_resources(
        device: &Arc<wgpu::Device>,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::Buffer) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("subtitle-atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("subtitle-atlas-staging"),
            size: (width as u64 * height as u64) as wgpu::BufferAddress,
            // 写侧是 queue::write_buffer，读侧是 copy_buffer_to_texture，两边都要用到
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        (texture, staging)
    }

    /// 逐行把块的覆盖值拷进 CPU 侧缓冲。行距就是图集宽（它是行对齐的整数倍），
    /// 行内块之间的空隙保持 0；拷进纹理的动作在 `compose` 的那个 encoder 里。
    fn fill_atlas_scratch(&mut self, slots: &[AtlasSlot], tiles: &[&Tile], extent: (u32, u32)) {
        self.scratch.clear();
        self.scratch
            .resize(extent.1 as usize * self.atlas_width as usize, 0);

        for slot in slots {
            let tile = tiles[slot.index];

            for row in 0..tile.height {
                let start_of_row = row * tile.stride;
                let Some(source) = tile.bitmap.get(start_of_row..start_of_row + tile.width) else {
                    // 覆盖缓冲的最后一行可以短于一整行（aweft-ass render.rs 的
                    // Tile 字段说明）；行序递增，取不到就说明后面也没有
                    break;
                };
                let destination =
                    (slot.y as usize + row) * self.atlas_width as usize + slot.x as usize;
                self.scratch[destination..destination + tile.width].copy_from_slice(source);
            }
        }

        self.queue
            .write_buffer(&self.atlas_staging, 0, &self.scratch);
    }

    /// 实例缓冲只增不减；本帧块数超过容量时整块换大一号。
    fn ensure_instance_capacity(&mut self, count: usize) {
        if count <= self.instance_capacity {
            return;
        }
        let capacity = count.div_ceil(INSTANCE_CAPACITY_STEP) * INSTANCE_CAPACITY_STEP;
        self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("subtitle-instances"),
            size: (capacity * std::mem::size_of::<InstanceData>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = capacity;
    }

    fn write_instances(&mut self, slots: &[AtlasSlot], tiles: &[&Tile]) {
        self.ensure_instance_capacity(tiles.len());

        let instances: Vec<InstanceData> = slots
            .iter()
            .map(|slot| {
                let tile = tiles[slot.index];
                let width = tile.width as u32;
                let height = tile.height as u32;
                InstanceData {
                    color: tile.color(),
                    // 整帧目标：块的帧坐标就是四边形位置，没有包围盒偏移
                    position: [tile.x as f32, tile.y as f32],
                    size: [width as f32, height as f32],
                    atlas_uv_offset: [
                        slot.x as f32 / self.atlas_width as f32,
                        slot.y as f32 / self.atlas_height as f32,
                    ],
                    atlas_uv_size: [
                        width as f32 / self.atlas_width as f32,
                        height as f32 / self.atlas_height as f32,
                    ],
                }
            })
            .collect();
        // 缓冲容量可以大于本帧实例数：draw 的实例数决定顶点着色器跑几次，
        // 尾部多出来的旧数据不会被读到
        self.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
    }

    fn write_projection(&mut self, width: u32, height: u32) {
        if self.projected_size == (width, height) {
            return;
        }
        let matrix = projection_matrix(width as f32, height as f32);
        self.queue
            .write_buffer(&self.projection_buffer, 0, bytemuck::cast_slice(&[matrix]));
        self.projected_size = (width, height);
    }
}

/// 三项绑定的组装处。图集换容量后必须重建绑定组，所以这里只能有一份实现。
fn make_bind_group(
    device: &Arc<wgpu::Device>,
    layout: &wgpu::BindGroupLayout,
    projection_buffer: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    atlas_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("subtitle-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: projection_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// 帧像素坐标（左上角 0,0、Y 向下）→ wgpu 裁剪空间（[-1,1]，Z 取 [0,1]）。
fn projection_matrix(width: f32, height: f32) -> [[f32; 4]; 4] {
    [
        [2.0 / width, 0.0, 0.0, 0.0],
        [0.0, -2.0 / height, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0],
        [-1.0, 1.0, 0.0, 1.0],
    ]
}
