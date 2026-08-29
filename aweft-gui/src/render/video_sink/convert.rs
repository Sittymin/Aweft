//! YUV → RGBA 离屏转色管线：采样 Y + 色度平面（NV12/P010 交错或 YU12 独立
//! U/V），按 colorimetry 输出 RGBA。着色器见 `yuv_convert.wgsl`。

use slint::wgpu_30::wgpu;

use super::frame::{ChromaViews, FramePlanes};

/// 早期调试开关：AWEFT_SWAP_UV=1 时交换 U/V（已验证默认 NV12 为 UV 顺序）
pub(crate) fn swap_uv_enabled() -> bool {
    std::env::var("AWEFT_SWAP_UV").is_ok_and(|v| v == "1")
}

/// YUV→RGB 全屏 pass：采样 Y + 色度平面（NV12/P010 交错或 YU12 独立 U/V），
/// 按 colorimetry 输出 RGBA。着色器见 `render/video_sink/yuv_convert.wgsl`。
pub(crate) struct YuvConvertPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl YuvConvertPipeline {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv-convert"),
            source: wgpu::ShaderSource::Wgsl(include_str!("yuv_convert.wgsl").into()),
        });

        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                texture_entry(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yuv-rp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv-uniform"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform,
            sampler,
        }
    }

    pub(crate) fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        planes: &FramePlanes,
        out_view: &wgpu::TextureView,
    ) {
        // Uniform 的前 16 字节是 ColorParams，后 16 字节是 LayoutParams。
        // mode 使用整数，避免 WGSL 侧用浮点阈值判断布局。
        // UV 交换（调试开关）只作用于交错色度平面（NV12/P010）。平面式布局
        // 的 U/V 已在平面映射时按 chroma_order 就位，shader 不需要再交换。
        let color_params: [f32; 4] = [
            if planes.matrix_is_601 { 1.0 } else { 0.0 },
            if planes.full_range { 1.0 } else { 0.0 },
            if swap_uv_enabled() { 1.0 } else { 0.0 },
            0.0,
        ];
        let layout_params: [u32; 4] = [
            planes.layout.shader_mode(),
            if planes.layout.y_format == wgpu::TextureFormat::Rgb10a2Unorm {
                1
            } else {
                0
            },
            0,
            0,
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&color_params));
        queue.write_buffer(&self.uniform, 16, bytemuck::cast_slice(&layout_params));

        // 未用到的色度槽位以 Y 平面视图占位（着色器按 mode 不会采样）
        let (uv_view, u_view, v_view) = match &planes.chroma {
            ChromaViews::Interleaved(v) => (v, &planes.y_view, &planes.y_view),
            ChromaViews::Planar(u, v) => (&planes.y_view, u, v),
            ChromaViews::Packed => (&planes.y_view, &planes.y_view, &planes.y_view),
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&planes.y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(u_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(v_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("yuv-enc"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yuv-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // 当前使用双线性采样；后续可按缩放比例引入更高质量的缩放算法。
                        // 全屏覆盖绘制，load 值无所谓；Clear 语义最稳
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: Default::default(),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}
