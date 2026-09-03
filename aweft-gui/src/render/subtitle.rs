use anyhow::{Result, anyhow};
use aweft_libass::image::Image;
use slint::wgpu_30::wgpu;
use wgpu::util::DeviceExt; // NOTE: 实例化渲染需要用到 create_buffer_init

// 实例化渲染：每个字幕图块的实例数据
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SubtitleInstanceData {
    color: [f32; 4],
    position: [f32; 2],
    size: [f32; 2],
    // 图块在图集中的 UV 坐标偏移 (0.0-1.0)
    atlas_uv_offset: [f32; 2],
    // 图块在图集中的 UV 坐标尺寸 (0.0-1.0)
    atlas_uv_size: [f32; 2],
}
impl SubtitleInstanceData {
    fn vertex_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            // 结构体 `SubtitleInstanceData` 的总大小
            array_stride: std::mem::size_of::<SubtitleInstanceData>() as wgpu::BufferAddress,
            // 关键：这表明这是一个实例缓冲区，每个实例数据移动一次，而不是每个顶点。
            step_mode: wgpu::VertexStepMode::Instance,
            // 描述 `SubtitleInstanceData` 结构体中每个字段如何映射到顶点着色器的输入
            attributes: &[
                // @location(0): instance_color
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x4,
                },
                // @location(1): instance_position
                wgpu::VertexAttribute {
                    offset: 16, // 4 * f32 = 16 bytes
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(2): instance_size
                wgpu::VertexAttribute {
                    offset: 24, // 16 + 8 = 24 bytes
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(3): instance_atlas_uv_offset
                wgpu::VertexAttribute {
                    offset: 32, // 24 + 8 = 32 bytes
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // @location(4): instance_atlas_uv_size
                wgpu::VertexAttribute {
                    offset: 40, // 32 + 8 = 40 bytes
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

#[derive(Clone)]
pub struct SubtitleRenderer {
    render_pipeline: wgpu::RenderPipeline,
    wgpu_device: wgpu::Device,
    wgpu_queue: wgpu::Queue,
    projection_buffer: wgpu::Buffer,
    // 用于高效更新投影矩阵的*暂存*缓冲区
    projection_staging_buffer: wgpu::Buffer,
    // 用于高效上传纹理图集的*暂存*缓冲区
    texture_staging_buffer: wgpu::Buffer,
    texture_staging_buffer_size: wgpu::BufferAddress,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
}

// GPU 渲染返回的纹理数据
#[derive(Debug)]
pub struct RenderedSubtitleTexture {
    // 混合后的 WGPU 纹理
    pub texture: wgpu::Texture,
    // 纹理的宽度（像素）。
    pub width: u32,
    // 纹理的高度（像素）。
    pub height: u32,
    // 渲染结果在原始视频帧中的左上角 X 坐标。
    pub x: i32,
    // 渲染结果在原始视频帧中的左上角 Y 坐标。
    pub y: i32,
}
// 用于存储单个图片在图集中的布局信息
// NOTE: 单个图片大小可利用索引访问到 width 和 height
struct AtlasLayout {
    /// 原始 image 在列表中的索引
    original_index: usize,
    /// 在图集中的像素 x 坐标
    px: u32,
    /// 在图集中的像素 y 坐标
    py: u32,
}

/// 使用简单的“货架”算法计算纹理图集布局
/// 返回 (布局列表, 图集总尺寸)
fn calculate_atlas_layout(
    images: &[aweft_libass::image::Image],
) -> (Vec<AtlasLayout>, wgpu::Extent3d) {
    const MAX_ATLAS_WIDTH: u32 = 2048; // 可根据需要调整
    if images.is_empty() {
        return (Vec::new(), wgpu::Extent3d::default());
    }

    let mut layouts = Vec::with_capacity(images.len());

    // NOTE:
    // current_x 用于定位当前添加数据的 x 偏移
    // current_y 用于定位当前添加数据的 y 偏移
    // shelf_height 行内图片最大高度
    // atlas_width 所有行的最大宽度
    let mut current_x = 0;
    let mut current_y = 0;
    let mut shelf_height = 0;
    let mut atlas_width = 0;

    for (i, image) in images.iter().enumerate() {
        let img_width = image.width() as u32;
        let img_height = image.height() as u32;

        if img_width == 0 || img_height == 0 {
            // 跳过零尺寸的图片，但仍为其创建一个占位布局
            layouts.push(AtlasLayout {
                original_index: i,
                px: 0,
                py: 0,
            });
            continue;
        }

        // 如果当前行宽度过长就换行，并清零基于当前行的数据
        if current_x + img_width > MAX_ATLAS_WIDTH {
            current_y += shelf_height;
            current_x = 0;
            shelf_height = 0;
        }

        layouts.push(AtlasLayout {
            original_index: i,
            px: current_x,
            py: current_y,
        });

        // 更新偏移和图集尺寸
        current_x += img_width;
        shelf_height = shelf_height.max(img_height);
        atlas_width = atlas_width.max(current_x);
    }

    // 所有行的累积高度
    let atlas_height = current_y + shelf_height;

    // 确保尺寸不为0，即使只有空图片
    let final_atlas_width = atlas_width.max(1);
    let final_atlas_height = atlas_height.max(1);

    (
        layouts,
        wgpu::Extent3d {
            width: final_atlas_width,
            height: final_atlas_height,
            depth_or_array_layers: 1,
        },
    )
}

impl SubtitleRenderer {
    // 3. 【初始化动作】在 `new` 方法中执行一次性的资源创建。
    pub fn new(graphics_api: &slint::GraphicsAPI<'_>) -> Self {
        match graphics_api {
            slint::GraphicsAPI::WGPU30 { device, queue, .. } => {
                let wgpu_device = device.clone();
                let wgpu_queue = queue.clone();

                // 之后在 bind_group 当中使用的数据或容器
                let projection_buffer = Self::create_projection_buffer(&wgpu_device);
                // 创建投影矩阵的*暂存*缓冲区
                let projection_staging_buffer =
                    Self::create_projection_staging_buffer(&wgpu_device);

                // 创建纹理图集的*暂存*缓冲区（初始大小）
                let (texture_staging_buffer, texture_staging_buffer_size) =
                    Self::create_texture_staging_buffer(&wgpu_device, 1024 * 1024); // 初始 1MB

                let sampler = Self::create_sampler(&wgpu_device);

                // 配置 CPU 与 GPU 之间资源交换与流程的管线（蓝图）
                let bind_group_layout = Self::create_bind_group_layout(&wgpu_device);
                let pipeline_layout =
                    Self::create_pipeline_layout(&wgpu_device, &bind_group_layout);
                let render_pipeline = Self::create_render_pipeline(&wgpu_device, &pipeline_layout);

                return Self {
                    render_pipeline,
                    wgpu_device,
                    wgpu_queue,
                    sampler,
                    projection_buffer,
                    projection_staging_buffer,
                    texture_staging_buffer,
                    texture_staging_buffer_size,
                    bind_group_layout,
                };
            }
            _ => panic!("不支持的图形 API，需要 WGPU"),
        }
    }
    fn create_projection_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Subtitle Projection Buffer"),
            // 一个 f32 是 4 字节，一个 4x4 正交投影矩阵有 16 个 f32
            size: (16 * std::mem::size_of::<f32>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    fn create_projection_staging_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Subtitle Projection Staging Buffer"),
            size: (16 * std::mem::size_of::<f32>()) as wgpu::BufferAddress,
            // 允许 CPU 写入，并作为 GPU 拷贝的源
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    fn create_texture_staging_buffer(
        device: &wgpu::Device,
        size: wgpu::BufferAddress,
    ) -> (wgpu::Buffer, wgpu::BufferAddress) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Subtitle Texture Staging Buffer"),
            size,
            // 允许 CPU 写入，并作为 GPU 拷贝的源
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        (buffer, size)
    }
    fn create_sampler(device: &wgpu::Device) -> wgpu::Sampler {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Pixel-Perfect Sampler"),

            // 纹理像素越界时的处理方式
            // ClampToEdge 会将超出部分的坐标拉回纹理的边缘，返回边缘的颜色。
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,

            // mag_filter (Magnification filter) 纹理被放大时的过滤模式
            // min_filter (Minification filter) 纹理被缩小时的过滤模式
            // mipmap_filter 设置在不同 mipmap 等级之间切换时的过滤模式
            // NOTE: 由于 libass 当中包含抗锯齿且字幕不会渲染出来之后放大缩小，所以这里直接使用临近采样
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,

            ..Default::default()
        })
    }
    fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Subtitle Bind Group Layout"),
            entries: &[
                // @binding(0): 正交投影矩阵（全局共享）
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX, // 只有顶点着色器需要
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(1): Alpha Mask Texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT, // 只有片元着色器需要
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // @binding(2): Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT, // 只有片元着色器需要
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }
    fn create_pipeline_layout(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> wgpu::PipelineLayout {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Subtitle Pipeline Layout"),
            bind_group_layouts: &[Some(bind_group_layout)],
            immediate_size: 0,
        })
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
    ) -> wgpu::RenderPipeline {
        let shader_module = device.create_shader_module(wgpu::include_wgsl!("subtitle.wgsl"));

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Subtitle Render Pipeline"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                // WARN: 需要与 wgsl 当中的定义一致
                buffers: &[Some(SubtitleInstanceData::vertex_buffer_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    // WARN: 这里是硬编码的，因为无法获得 surface 对象
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
        })
    }

    pub fn render_subtitles_gpu(
        &mut self,
        images: Vec<aweft_libass::image::Image>,
    ) -> Result<RenderedSubtitleTexture> {
        let device = &self.wgpu_device;
        let queue = &self.wgpu_queue;

        let image_list = images;
        if image_list.is_empty() {
            return Err(anyhow!("空的图片列表"));
        }

        // WARN: 先计算包围盒，因为后面的实例数据生成需要用到 min_x 和 min_y
        let (min_x, min_y, max_x, max_y) = image_list
            .iter()
            // 跳过零尺寸的图片，因为它们不应该影响包围盒
            .filter(|i| i.width() > 0 && i.height() > 0)
            .fold(
                (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
                |(min_x, min_y, max_x, max_y), image| {
                    (
                        min_x.min(image.dst_x()),
                        min_y.min(image.dst_y()),
                        max_x.max(image.dst_x() + image.width() as i32),
                        max_y.max(image.dst_y() + image.height() as i32),
                    )
                },
            );

        // 包围盒的宽高
        let box_width = (max_x - min_x) as u32;
        let box_height = (max_y - min_y) as u32;

        if box_width == 0 || box_height == 0 {
            return Err(anyhow!("字幕图片尺寸无效"));
        }

        // 计算各个图块应当在的位置和总图块大小
        let (atlas_layouts, atlas_size) = calculate_atlas_layout(&image_list);

        if atlas_size.width == 0 || atlas_size.height == 0 {
            return Err(anyhow!("字幕图块尺寸无效"));
        }

        // 创建图集纹理，用于存放所有字幕图块的 Alpha 数据
        let alpha_atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Subtitle Alpha Atlas Texture"),
            size: atlas_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm, // 存储单通道 Alpha
            usage:
                // 可以作为写入目标
                wgpu::TextureUsages::COPY_DST
                // 可以从此纹理读取
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // 计算满足 WGPU 要求的、对齐后的每行字节数 ( copy_buffer_to_texture 需要)
        // R8Unorm 格式，每个像素 1 字节
        let bytes_per_pixel = 1;
        let unaligned_bytes_per_row = atlas_size.width * bytes_per_pixel;
        let aligned_bytes_per_row =
            wgpu::util::align_to(unaligned_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

        // 如果初始缓冲区不够大重新创建
        let required_staging_size =
            (aligned_bytes_per_row * atlas_size.height) as wgpu::BufferAddress;
        if self.texture_staging_buffer_size < required_staging_size {
            (
                self.texture_staging_buffer,
                self.texture_staging_buffer_size,
            ) = Self::create_texture_staging_buffer(device, required_staging_size);
        }

        // 在 CPU 端准备一个包含所有图块数据的大型连续缓冲区（带对齐填充）
        let mut atlas_cpu_buffer = vec![0u8; required_staging_size as usize];
        let dst_stride = aligned_bytes_per_row as usize;

        for layout in &atlas_layouts {
            let image = &image_list[layout.original_index];
            let width = image.width() as usize;
            let height = image.height() as usize;

            if width == 0 || height == 0 {
                continue;
            }

            let src_stride = image.stride() as usize;
            let src_bitmap = image.bitmap();
            let dst_px = layout.px as usize;
            let dst_py = layout.py as usize;

            // 逐行将小图块的像素数据复制到大缓冲区中的正确位置（考虑对齐）
            for row in 0..height {
                let src_start = row * src_stride;
                let src_end = src_start + width;
                let dst_start = (dst_py + row) * dst_stride + dst_px;
                let dst_end = dst_start + width;

                if src_end <= src_bitmap.len() && dst_end <= atlas_cpu_buffer.len() {
                    atlas_cpu_buffer[dst_start..dst_end]
                        .copy_from_slice(&src_bitmap[src_start..src_end]);
                }
            }
        }

        // 将整个 CPU 缓冲区的数据一次性写入 GPU 的暂存缓冲区
        queue.write_buffer(&self.texture_staging_buffer, 0, &atlas_cpu_buffer);

        // 非屏幕显示的渲染目标纹理
        let render_target_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Subtitle Render Target"),
            size: wgpu::Extent3d {
                width: box_width,
                height: box_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // 关键：必须包含 RENDER_ATTACHMENT 用途，才能被当做渲染目标
            // 必须包含 TEXTURE_BINDING 用途，才能最终被 Slint 使用
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let render_target_view = render_target_texture.create_view(&Default::default());
        // 准备实例数据：将所有字幕图块的属性打包成一个大的缓冲区
        let mut instance_data = Vec::<SubtitleInstanceData>::new();
        for layout in &atlas_layouts {
            let image: &Image = &image_list[layout.original_index];
            if image.width() == 0 || image.height() == 0 {
                continue;
            }

            // 计算 UV 坐标 (0-1)
            let atlas_w_f32 = atlas_size.width as f32;
            let atlas_h_f32 = atlas_size.height as f32;
            let uv_offset = [
                layout.px as f32 / atlas_w_f32,
                layout.py as f32 / atlas_h_f32,
            ];
            let uv_size = [
                image.width() as f32 / atlas_w_f32,
                image.height() as f32 / atlas_h_f32,
            ];

            instance_data.push(SubtitleInstanceData {
                color: image.color().to_f32_array(),
                // 注意：这里的 position 是相对于包围盒左上角的
                position: [
                    (image.dst_x() - min_x) as f32,
                    (image.dst_y() - min_y) as f32,
                ],
                size: [image.width() as f32, image.height() as f32],
                atlas_uv_offset: uv_offset,
                atlas_uv_size: uv_size,
            });
        }

        if instance_data.is_empty() {
            return Err(anyhow!("没有有效的字幕图块数据"));
        }

        // 创建实例缓冲区
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Subtitle Instance Buffer"),
            contents: bytemuck::cast_slice(&instance_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let alpha_atlas_view = alpha_atlas_texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Subtitle Atlas Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.projection_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    // 绑定的是整个图集纹理
                    resource: wgpu::BindingResource::TextureView(&alpha_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&Default::default());

        // 记录从暂存缓冲区到最终纹理的拷贝指令
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &self.texture_staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    // 关键：使用对齐后的行字节数
                    bytes_per_row: Some(aligned_bytes_per_row),
                    rows_per_image: Some(atlas_size.height),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture: &alpha_atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            atlas_size,
        );

        // 更新投影矩阵到暂存区，并记录拷贝指令
        let projection_matrix = create_projection_matrix(box_width as f32, box_height as f32);
        queue.write_buffer(
            &self.projection_staging_buffer,
            0,
            bytemuck::cast_slice(&[projection_matrix]),
        );
        encoder.copy_buffer_to_buffer(
            &self.projection_staging_buffer,
            0,
            &self.projection_buffer,
            0,
            std::mem::size_of::<[[f32; 4]; 4]>() as wgpu::BufferAddress,
        );

        // NOTE: 此作用域内会可变的借用 encoder
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Subtitle Offscreen Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &render_target_view,
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

            render_pass.set_pipeline(&self.render_pipeline);

            // 设置绑定组和实例缓冲区
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.set_vertex_buffer(0, instance_buffer.slice(..));

            // 这里的 6 是每个矩形的顶点数(两个三角形)，instance_data.len() 是实例数量
            render_pass.draw(0..6, 0..instance_data.len() as u32);
        }
        queue.submit(Some(encoder.finish()));

        Ok(RenderedSubtitleTexture {
            texture: render_target_texture,
            x: min_x,
            y: min_y,
            width: box_width,
            height: box_height,
        })
    }
}

/// 创建一个将屏幕坐标 (左上角为 0,0) 映射到 WGPU 裁剪空间 ([-1, 1]) 的正交投影矩阵。
///
/// # Arguments
/// * `width`: 渲染目标的宽度。
/// * `height`: 渲染目标的高度。
///
/// # Returns
/// 一个 4x4 的矩阵，可以直接上传到 Uniform Buffer。
pub fn create_projection_matrix(width: f32, height: f32) -> [[f32; 4]; 4] {
    // WGPU 的裁剪空间 Z 轴范围是 [0, 1]。
    let near = 0.0;
    let far = 1.0;

    // 计算左右、上下、远近边界的差值倒数
    let r_l = 1.0 / (width); // right - left
    let t_b = 1.0 / (height); // top - bottom
    let f_n = 1.0 / (far - near); // far - near

    [
        // 第 1 列: 负责 X 轴的缩放和平移
        [
            2.0 * r_l, // X 轴缩放：将 [0, width] 的宽度缩放到 2.0
            0.0,
            0.0,
            0.0,
        ],
        // 第 2 列: 负责 Y 轴的缩放和平移
        [
            0.0,
            -2.0 * t_b, // Y 轴缩放：将 [0, height] 的高度缩放到 -2.0 (同时翻转Y轴 屏幕和裁剪空间方向相反)
            0.0,
            0.0,
        ],
        // 第 3 列: 负责 Z 轴的缩放和平移
        [
            0.0, 0.0, -f_n, // Z 轴缩放：将 [near, far] 的深度范围缩放到 -1.0
            0.0,
        ],
        // 第 4 列: 负责整体平移
        [
            -1.0,        // X 轴平移：将缩放后的 [0, 2] 范围向左移动 1.0，变为 [-1, 1]
            1.0,         // Y 轴平移：将缩放后的 [0, -2] 范围向上移动 1.0，变为 [1, -1]
            -near * f_n, // Z 轴平移：将 [near, far] 映射到 [0, 1]
            1.0,
        ],
    ]
}
