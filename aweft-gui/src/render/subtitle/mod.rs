//! 字幕渲染通路：核心序列 → ASS 脚本 → libass 图块 → 整帧 RGBA 纹理 → 视口覆盖层。
//!
//! - `compositor`：wgpu 侧的图块合成与常驻资源
//! - `pipeline`：脚本字节灌进 libass、逐帧出图与槽位归还
//! - `tests`：无头像素校验（libass 给的块坐标必须原样落在整帧输出上）

pub(crate) mod compositor;
pub(crate) mod pipeline;

pub(crate) use compositor::Composition;
pub(crate) use pipeline::SubtitlePipeline;

#[cfg(test)]
mod tests;
