use anyhow::{Result, anyhow};
use slint::wgpu_24::wgpu;
use std::sync::Arc;
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

#[derive(Clone)]
pub struct SubtitleRenderer {
    render_pipeline: wgpu::RenderPipeline,
    wgpu_device: Option<Arc<wgpu::Device>>,
    wgpu_queue: Option<Arc<wgpu::Queue>>,
    projection_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    // NOTE: 实例化渲染不再需要单独的 uniform_buffer，改为使用动态实例缓冲区
    // 这样可以在单次 draw 调用中渲染所有字幕图块
}

// GPU 渲染返回的纹理数据
#[derive(Debug)]
pub struct RenderedSubtitleTexture {
    /// WGPU 纹理，已包含预乘 Alpha 格式的 RGBA 像素数据。
    /// 这个纹理可以直接转换为 `slint::Image`。
    pub texture: wgpu::Texture,
    /// 纹理的宽度（像素）。
    pub width: u32,
    /// 纹理的高度（像素）。
    pub height: u32,
    /// 渲染结果在原始视频帧中的左上角 X 坐标。
    /// Slint 可以用这个来定位组件。
    pub x: i32,
    /// 渲染结果在原始视频帧中的左上角 Y 坐标。
    pub y: i32,
}
/// 用于存储单个图片在图集中的布局信息
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
    images: &[aweft_libass::types::Image],
) -> (Vec<AtlasLayout>, wgpu::Extent3d) {
    const MAX_ATLAS_WIDTH: u32 = 2048; // 可根据需要调整
    if images.is_empty() {
        return (Vec::new(), wgpu::Extent3d::default());
    }

    let mut layouts = Vec::with_capacity(images.len());

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

        // 如果当前货架放不下，就换行
        if current_x + img_width > MAX_ATLAS_WIDTH {
            current_y += shelf_height;
            current_x = 0;
            shelf_height = 0;
        }

        // 放置图片
        layouts.push(AtlasLayout {
            original_index: i,
            px: current_x,
            py: current_y,
        });

        // 更新货架和图集尺寸
        current_x += img_width;
        shelf_height = shelf_height.max(img_height);
        atlas_width = atlas_width.max(current_x);
    }

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
            slint::GraphicsAPI::WGPU24 { device, queue, .. } => {
                let wgpu_device = Some(Arc::new(device.clone()));
                let wgpu_queue = Some(Arc::new(queue.clone()));
                let projection_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Subtitle Projection Buffer"),
                    // 一个 f32 是 4 字节，一个 4x4 矩阵有 16 个 f32
                    size: (16 * std::mem::size_of::<f32>()) as wgpu::BufferAddress,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Pixel-Perfect Sampler"),

                    // AddressMode* 设置当纹理坐标超出 [0.0, 1.0] 范围时的行为。
                    // ClampToEdge 会将超出部分的坐标拉回纹理的边缘，返回边缘的颜色。
                    // 这对于渲染单个字符、不希望出现重复平铺的情况来说是最佳选择。
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    address_mode_w: wgpu::AddressMode::ClampToEdge,

                    // mag_filter (Magnification filter) 设置当纹理被放大时的过滤模式。
                    // Nearest: 选择最接近纹理坐标的那个像素。效果是锐利的、像素化的“马赛克”风格。
                    // 这就是你想要的“直接遍历像素”的效果。
                    mag_filter: wgpu::FilterMode::Nearest,

                    // min_filter (Minification filter) 设置当纹理被缩小时的过滤模式。
                    // Nearest: 同样，选择最接近的像素。
                    min_filter: wgpu::FilterMode::Nearest,

                    // mipmap_filter 设置在不同 mipmap 等级之间切换时的过滤模式。
                    // 我们这里没有用 mipmap，所以设为 Nearest 即可。
                    mipmap_filter: wgpu::FilterMode::Nearest,

                    // 其他参数可以保持默认
                    ..Default::default()
                });
                let bind_group_layout =
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
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
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
                    });
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Subtitle Pipeline Layout"),
                        bind_group_layouts: &[&bind_group_layout],
                        push_constant_ranges: &[],
                    });

                let shader_module =
                    device.create_shader_module(wgpu::include_wgsl!("subtitle.wgsl"));
                let render_pipeline =
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("Subtitle Render Pipeline"),
                        layout: Some(&pipeline_layout), // 使用我们刚刚创建的“契约”
                        // NOTE: 可能以后会使用缓存提高速度
                        cache: None,

                        // --- 顶点着色器阶段 ---
                        vertex: wgpu::VertexState {
                            module: &shader_module,
                            entry_point: Some("vs_main"), // 指定入口函数
                            // NOTE: 实例化渲染需要定义实例缓冲区布局
                            buffers: &[wgpu::VertexBufferLayout {
                                array_stride: std::mem::size_of::<SubtitleInstanceData>() as wgpu::BufferAddress,
                                step_mode: wgpu::VertexStepMode::Instance, // 关键：每个实例读取一次
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
                            }],
                            compilation_options: wgpu::PipelineCompilationOptions::default(),
                        },

                        // --- 片元着色器阶段 ---
                        fragment: Some(wgpu::FragmentState {
                            module: &shader_module,
                            entry_point: Some("fs_main"), // 指定入口函数
                            targets: &[Some(wgpu::ColorTargetState {
                                // WARN: 这里是硬编码的，因为无法获得 surface 对象
                                format: wgpu::TextureFormat::Rgba8Unorm,

                                // #########################################
                                // ### 这是开启 GPU 自动混合的“魔法开关” ###
                                // #########################################
                                blend: Some(wgpu::BlendState::ALPHA_BLENDING),

                                write_mask: wgpu::ColorWrites::ALL, // 写入所有通道 (RGBA)
                            })],
                            compilation_options: wgpu::PipelineCompilationOptions::default(),
                        }),

                        // --- 其他配置 ---
                        primitive: wgpu::PrimitiveState {
                            topology: wgpu::PrimitiveTopology::TriangleList, // 我们画的是三角形
                            strip_index_format: None,
                            front_face: wgpu::FrontFace::Ccw, // 定义三角形的正面方向
                            cull_mode: Some(wgpu::Face::Back), // 剔除背面，提高性能
                            polygon_mode: wgpu::PolygonMode::Fill,
                            ..Default::default()
                        },
                        depth_stencil: None, // 2D 渲染，暂时不需要深度/模板缓冲
                        multisample: wgpu::MultisampleState::default(), // 不使用多重采样
                        multiview: None,
                    });
                return Self {
                    render_pipeline,
                    wgpu_device,
                    wgpu_queue,
                    sampler,
                    projection_buffer,
                    bind_group_layout,
                    pipeline_layout,
                };
            }
            _ => panic!("不支持的图形 API，需要 WGPU"),
        }
    }

    pub fn render_subtitles_gpu(
        &self,
        images: aweft_libass::types::ImageIterator,
    ) -> Result<RenderedSubtitleTexture> {
        let device = self
            .wgpu_device
            .as_ref()
            .ok_or(anyhow!("Not have avaliable device"))?;
        let queue = self
            .wgpu_queue
            .as_ref()
            .ok_or(anyhow!("Not have avaliable queue"))?;

        let image_list: Vec<aweft_libass::types::Image> = images.collect();
        if image_list.is_empty() {
            return Err(anyhow!("空的图片列表"));
        }

        // WARN: 先计算包围盒，因为后面的实例数据生成需要用到 min_x 和 min_y
        let (min_x, min_y, max_x, max_y) = image_list
            .iter()
            // 跳过零尺寸的图片，因为它们不应该影响包围盒
            .filter(|i| i.width() > 0 && i.height() > 0)
            // 使用第一个有效图片的位置来初始化边界
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
        let width = (max_x - min_x) as u32;
        let height = (max_y - min_y) as u32;

        if width == 0 || height == 0 {
            // 处理所有图像都没有尺寸的情况
            return Err(anyhow!("字幕图片尺寸无效"));
        }

        // 1. 计算所有图块在图集中的布局和图集总尺寸
        let (atlas_layouts, atlas_size) = calculate_atlas_layout(&image_list);

        if atlas_size.width == 0 || atlas_size.height == 0 {
            return Err(anyhow!("Subtitle atlas size is zero"));
        }

        // 2. 创建一个大的图集纹理，用于存放所有字幕图块的 Alpha 数据
        let alpha_atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Subtitle Alpha Atlas Texture"),
            size: atlas_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm, // 单通道 Alpha
            usage: 
                // 可以作为写入目标
                wgpu::TextureUsages::COPY_DST
                // 可以从此纹理读取
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // 用于复用的缓冲区
        let mut texture_data_buffer: Vec<u8> = Vec::new();



        // 3. 将所有图块的 bitmap 数据写入图集纹理的对应位置
        for layout in &atlas_layouts {
            let image = &image_list[layout.original_index];
            let width = image.width() as usize;
            let height = image.height() as usize;

            if width == 0 || height == 0 {
                continue; // 跳过零尺寸图像
            }

            // NOTE: libass 处理的内存对齐并不满足 WGPU 的 256 字节对齐
            let stride = image.stride() as usize;
            let bitmap = image.bitmap();
            // 创建紧凑的纹理数据，只包含实际像素，无填充
            // 清空并确保容量，但通常不会重新分配
            texture_data_buffer.clear();
            texture_data_buffer.reserve(width * height);
            // 逐行复制，只复制实际的像素数据，跳过stride中的填充字节
            for row in 0..height {
                let row_start = stride * row;
                let row_end = row_start + width;
                if row_end <= bitmap.len() {
                    texture_data_buffer.extend_from_slice(&bitmap[row_start..row_end]);
                } else {
                    eprintln!("libass 返回渲染结果不是矩形");
                    // 如果数据不足（比如最后一行），用0填充剩余部分
                    let available_bytes = bitmap.len().saturating_sub(row_start);
                    let bytes_to_copy = width.min(available_bytes);

                    if bytes_to_copy > 0 {
                        texture_data_buffer.extend_from_slice(&bitmap[row_start..row_start + bytes_to_copy]);
                    }
                    // 填充剩余的字节为0
                    texture_data_buffer.resize(width * height, 0);
                }
            }

            // 使用 origin 参数将数据写入图集的指定区域
            queue.write_texture(
                wgpu::TexelCopyTextureInfoBase {
                    texture: &alpha_atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: layout.px,
                        y: layout.py,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &texture_data_buffer,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: width as u32,
                    height: height as u32,
                    depth_or_array_layers: 1,
                },
            );
        }

        // 非屏幕显示的渲染目标纹理
        let render_target_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Subtitle Render Target"),
            size: wgpu::Extent3d {
                width,
                height,
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
            let image = &image_list[layout.original_index];
            if image.width() == 0 || image.height() == 0 {
                continue;
            }

            // 计算 UV 坐标
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

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Subtitle Offscreen Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &render_target_view,
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

            // 更新投影矩阵
            let projection_matrix = create_projection_matrix(width as f32, height as f32);
            queue.write_buffer(
                &self.projection_buffer,
                0,
                bytemuck::cast_slice(&[projection_matrix]),
            );

            // 设置绑定组和实例缓冲区
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.set_vertex_buffer(0, instance_buffer.slice(..));

            // NOTE: 实例化渲染的核心优化：单次 draw 调用渲染所有字幕图块
            // 这里的 6 是每个矩形的顶点数，instance_data.len() 是实例数量
            render_pass.draw(0..6, 0..instance_data.len() as u32);
        }
        queue.submit(Some(encoder.finish()));



        Ok(RenderedSubtitleTexture {
            texture: render_target_texture,
            x: min_x,
            y: min_y,
            width,
            height,
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
