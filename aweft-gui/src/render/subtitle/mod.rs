//! 字幕渲染通路：核心序列 → ASS 脚本 → libass 图块 → 整帧 RGBA 纹理 → 视口覆盖层。
//!
//! - `compositor`：wgpu 侧的图块合成与常驻资源
//! - `hits`：图块列表还原成每行的包围框（视口命中与拖动靠它）
//! - `pipeline`：脚本字节灌进 libass、逐帧出图与槽位归还
//!

pub(crate) mod compositor;
pub(crate) mod hits;
pub(crate) mod pipeline;

pub(crate) use compositor::Composition;
pub(crate) use pipeline::SubtitlePipeline;

#[cfg(test)]
mod tests;
