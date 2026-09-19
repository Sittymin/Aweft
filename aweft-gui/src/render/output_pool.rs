//! RGBA 输出纹理槽位池：`slint::Image` 持有 `wgpu::Texture` 句柄，交给 Slint 之后
//! 不能立刻被下一帧覆盖。视频帧与字幕帧共用这一份归还规则：取到槽位即占住，Slint
//! 画完一帧（`AfterRendering`）后再等对应 queue submission 完成才置空闲；全部占用
//! 时返回 None 让调用方丢帧，UI 不反向拖住生产者。
//!
//! 池的容量与「旧尺寸槽位必须等全部空闲才能整池替换」这条约束都只在这里维护一份。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::wgpu_30::wgpu;

/// 同时在飞的帧数上限：Slint 采样期间 + 下一帧写入期间留出余量。
const POOL_SIZE: usize = 3;

struct Slot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// true = 已交给 Slint 或正在被采样，不可复用
    in_flight: Arc<AtomicBool>,
}

pub(crate) struct OutputSlotPool {
    label: &'static str,
    slots: Vec<Slot>,
    /// 本帧交出去的那个槽位，`after_rendering` 时归还
    active: Option<usize>,
}

impl OutputSlotPool {
    pub(crate) fn new(label: &'static str) -> Self {
        let mut slots = Vec::with_capacity(POOL_SIZE);
        slots.reserve_exact(POOL_SIZE);
        Self {
            label,
            slots,
            active: None,
        }
    }

    /// 取一个尺寸合适的空闲槽位并占住它；没有可用槽位时返回 None。
    pub(crate) fn acquire(
        &mut self,
        device: &Arc<wgpu::Device>,
        width: u32,
        height: u32,
    ) -> Option<(usize, wgpu::Texture, wgpu::TextureView)> {
        for (index, slot) in self.slots.iter().enumerate() {
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

        if self.slots.len() >= POOL_SIZE {
            // 仍在使用的旧尺寸槽位不能提前销毁，否则 Slint 手里的旧 Image 会悬空。
            if self
                .slots
                .iter()
                .any(|slot| slot.in_flight.load(Ordering::Acquire))
            {
                log::trace!("{} 输出纹理池暂时没有可用槽位，丢弃一帧", self.label);
                return None;
            }
            self.slots.clear();
        }

        let texture = self.create_texture(device, width, height);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let index = self.slots.len();
        self.slots.push(Slot {
            texture: texture.clone(),
            view: view.clone(),
            in_flight: Arc::new(AtomicBool::new(true)),
        });
        Some((index, texture, view))
    }

    /// 本帧已交给 Slint：记下占用的槽位，等 `after_rendering` 归还。
    pub(crate) fn mark_active(&mut self, index: usize) {
        self.active = Some(index);
    }

    /// 占到了但没用上（后续步骤失败）：立刻释放。
    pub(crate) fn discard(&mut self, index: usize) {
        if let Some(slot) = self.slots.get(index) {
            slot.in_flight.store(false, Ordering::Release);
        }
    }

    /// Slint 每帧绘制结束后调用：等对应 GPU submission 完成再置空闲。
    /// `queue` 为 None 表示 GPU 句柄已释放（teardown 之后），没有 submission
    /// 可等，直接放行槽位。
    pub(crate) fn after_rendering(&mut self, queue: Option<&wgpu::Queue>) {
        let Some(index) = self.active.take() else {
            return;
        };
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        let in_flight = slot.in_flight.clone();
        let Some(queue) = queue else {
            in_flight.store(false, Ordering::Release);
            return;
        };
        queue.on_submitted_work_done(move || {
            in_flight.store(false, Ordering::Release);
        });
    }

    /// 释放整池（teardown、重新连接 GPU 句柄时）。
    pub(crate) fn clear(&mut self) {
        self.slots.clear();
        self.active = None;
    }

    /// 池内纹理，供无头测试回读像素（不参与任何生产逻辑）。
    #[cfg(test)]
    pub(crate) fn textures(&self) -> impl Iterator<Item = &wgpu::Texture> {
        self.slots.iter().map(|slot| &slot.texture)
    }

    fn create_texture(&self, device: &Arc<wgpu::Device>, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(self.label),
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
