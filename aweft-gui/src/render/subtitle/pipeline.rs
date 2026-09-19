//! 字幕通路：脚本字节 → ASS 字形块 → 整帧 RGBA 纹理。
//!
//! 三个入口各管一件事，分属两条路径：
//! - `set_script`：换脚本。由 depsgraph 投影调用（用户操作节奏），不在逐帧路径上。
//! - `render_at`：出一帧。由渲染回调逐帧驱动，属于 `docs/architecture/video-playback.md`
//!   里那种「实时例外」，不经 reducer/depsgraph。
//! - `after_rendering`：Slint 画完一轮后归还输出槽位。
//!
//! 帧尺寸在这里是两个概念，别混：
//! - **排版尺寸**（`AppState::preview_frame_size`：主视频显示像素尺寸，无主视频时
//!   兜底 1080P）。脚本的 PlayRes 按它编，覆盖层矩形的比例也是它。
//! - **光栅尺寸**：覆盖层矩形在屏幕上的物理像素数（经 `raster_size` 档化与钳制）。
//!   libass 的渲染帧与输出纹理都用它，因此屏幕上一个像素对应纹理一个像素，
//!   放大字幕不发虚。两轴同倍率 ⇒ 画面就是「导出的 .ass 排版整体缩放」，
//!   位置与比例不分叉，只有字形按屏幕实际像素数重新光栅化。
//!
//! 这里的 `aweft_ass::render::Renderer` 是渲染层与 libass 之间唯一的接缝：
//! aweft-gui 不直接依赖 aweft-libass。

use anyhow::Result;
use aweft_ass::render::Renderer;

use super::compositor::{Composition, SubtitleCompositor};
use crate::render::video_sink::WgpuHandles;

/// 光栅倍率的档位：倍率向上凑到 1/8 的整数倍（1080P 排版即 135 高一档）。
/// 凑整是为了让缩放手势期间不必每一帧都换 libass 渲染帧与输出槽位池。
const RASTER_SCALE_STEPS: f64 = 8.0;

/// 倍率下限。低于 1/4 时字形已经退化成几行像素，再小也不准，
/// 宁可让覆盖层缩小显示一张 1/4 图。
const RASTER_SCALE_MIN_STEPS: u32 = 2;

/// 光栅总像素预算：16MP，约两张 4K UHD。放大后覆盖层可以远大于窗口，而纹理是
/// 整帧的（三份常驻槽位 + 一次全帧混色），不设上限时滚轮几格就能要出几百 MB 显存。
/// 顶到预算后回到「Slint 替我们插值放大」的旧行为，只在极端放大倍率下出现。
const RASTER_PIXEL_BUDGET: f64 = 16_000_000.0;

/// 光栅尺寸 = 排版尺寸 × 倍率。倍率取「覆盖层物理像素 / 排版尺寸」两轴的较大值
/// （只放大不缩小纹理，宁可用一张略大的图），向上凑档后再钳进设备上限与像素预算。
///
/// 覆盖层尺寸还没报上来（首次布局前）时退回排版尺寸，等价于 1 倍光栅。
pub(crate) fn raster_size(
    layout: (u32, u32),
    on_screen_physical: (f32, f32),
    max_dimension: u32,
) -> (u32, u32) {
    let (layout_width, layout_height) = (layout.0 as f64, layout.1 as f64);
    let scale = (f64::from(on_screen_physical.0) / layout_width)
        .max(f64::from(on_screen_physical.1) / layout_height);
    if !scale.is_finite() || scale <= 0.0 {
        // 首次布局前覆盖层尺寸还是 0：按排版尺寸 1:1 出图，下一帧自会纠正
        return layout;
    }
    // 上限：单边不超设备纹理上限，总面积不超预算
    let cap = (f64::from(max_dimension) / layout_width.max(layout_height))
        .min((RASTER_PIXEL_BUDGET / (layout_width * layout_height)).sqrt());
    let max_steps = (cap * RASTER_SCALE_STEPS).floor() as i64;
    let steps = ((scale * RASTER_SCALE_STEPS).ceil() as i64)
        .min(max_steps)
        .max(RASTER_SCALE_MIN_STEPS as i64);
    let scale = steps as f64 / RASTER_SCALE_STEPS;
    // 极端排版尺寸（预算/上限已被钳到不足一档）下逐边再钳一次，保证建纹理不越界
    let width = ((layout_width * scale).round() as u32).clamp(1, max_dimension);
    let height = ((layout_height * scale).round() as u32).clamp(1, max_dimension);
    (width, height)
}

pub(crate) struct SubtitlePipeline {
    compositor: SubtitleCompositor,
    /// 最近一次生成的脚本字节。建轨需要帧尺寸，尺寸到位前只存字节。
    script: Option<Vec<u8>>,
    renderer: Option<Renderer>,
    /// 设备允许的单边纹理尺寸，光栅尺寸钳到它以内
    max_texture_dimension_2d: u32,
}

impl SubtitlePipeline {
    pub(crate) fn new(handles: &WgpuHandles) -> Self {
        let (_, device, _) = handles;
        Self {
            compositor: SubtitleCompositor::new(handles),
            script: None,
            renderer: None,
            max_texture_dimension_2d: device.limits().max_texture_dimension_2d,
        }
    }

    /// 换脚本。旧轨立即作废，下一次出帧按当时的帧尺寸重建。
    pub(crate) fn set_script(&mut self, script: Vec<u8>) {
        self.script = Some(script);
        self.renderer = None;
    }

    /// 出一帧。`layout_size` 是预览帧尺寸（排版基准，无主视频时已是 1080P 兜底值），
    /// `on_screen_physical` 是覆盖层矩形在屏幕上的物理像素尺寸（`.slint` 报来的逻辑值
    /// 乘窗口缩放）。排版基准为零或还没有脚本时返回 `Empty`，覆盖层因此与「当前时刻
    /// 没有字幕」走同一条清空动作。
    pub(crate) fn render_at(
        &mut self,
        now_ms: i64,
        layout_size: (u32, u32),
        on_screen_physical: (f32, f32),
    ) -> Result<Composition> {
        let Some(script) = self.script.as_ref() else {
            return Ok(Composition::Empty);
        };
        let (width, height) = layout_size;
        if width == 0 || height == 0 {
            return Ok(Composition::Empty);
        }

        let raster = raster_size(
            layout_size,
            on_screen_physical,
            self.max_texture_dimension_2d,
        );
        self.compositor.set_raster_size(raster);
        // 换尺寸不换轨：PlayRes 仍读脚本里的排版尺寸，libass 自己按倍率缩放
        let resized = match self.renderer.as_mut() {
            Some(renderer) => renderer.set_size(raster.0, raster.1)?,
            None => {
                self.renderer = Some(Renderer::from_script(raster.0, raster.1, script)?);
                true
            }
        };

        let frame = self
            .renderer
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("字幕渲染轨未建立"))?
            .render(now_ms)?;
        // ASS 渲染自己判定了字形块列表与上一调用是否相同：相同就不重复合成，
        // 静止的字幕因此完全不占 GPU。但换过尺寸的那一帧必须出图——静止且为空的
        // 时刻 libass 不报变化，旧尺寸的画面否则会一直留在覆盖层上。
        if !frame.changed && !resized {
            return Ok(Composition::Skipped);
        }
        self.compositor.compose(&frame.tiles)
    }

    /// Slint 画完一轮场景后归还本帧的输出槽位。
    pub(crate) fn after_rendering(&mut self) {
        self.compositor.after_rendering();
    }

    /// 释放 GPU 资源（渲染器销毁时）。
    pub(crate) fn teardown(&mut self) {
        self.renderer = None;
        self.compositor.teardown();
    }
}
