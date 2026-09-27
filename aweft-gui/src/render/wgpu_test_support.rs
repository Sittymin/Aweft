//! 无头 wgpu 测试支持：自己建一套 Instance/Device/Queue 并把纹理读回 CPU。
//!
//! 视频与字幕两条通路都需要「不经 Slint 窗口」验证 GPU 结果，因此这份工具放在
//! `render/` 层共用（原本各自长在 `video_sink/tests.rs` 里）。

use std::time::Duration;

use slint::wgpu_30::wgpu;

/// headless wgpu 设备（Vulkan，无 surface）
pub(crate) fn headless_wgpu() -> (wgpu::Instance, wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("无可用 GPU 适配器");
    let (device, queue) =
        futures::executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
            ..Default::default()
        }))
        .expect("创建 wgpu 设备失败");
    (instance, device, queue)
}

/// RGBA 纹理回读（unpad 到紧密行距）
pub(crate) fn readback_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let bytes_per_row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-readback"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("device.poll 失败");
    rx.recv_timeout(Duration::from_secs(10))
        .unwrap()
        .expect("map 失败");
    let mapped = slice.get_mapped_range().expect("mapped range");
    let tight = (0..h as usize)
        .flat_map(|y| {
            mapped[y * bytes_per_row as usize..y * bytes_per_row as usize + w as usize * 4].to_vec()
        })
        .collect();
    drop(mapped);
    buf.unmap();
    tight
}
