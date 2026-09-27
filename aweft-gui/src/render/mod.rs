//! 渲染进 Slint 的后端：字幕渲染（wgpu）+ 视频播放（gstreamer → Slint）。

pub(crate) mod output_pool;
pub(crate) mod subtitle;
pub mod video_sink;

#[cfg(test)]
pub(crate) mod wgpu_test_support;
