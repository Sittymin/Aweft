use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    // === 基础信息 ===
    pub title: String,
    pub author: String,
    pub description: String,

    // === 关键渲染参数 (对应 ASS Script Info) ===
    // 这是一个很大的设计点：ASS 的坐标是依赖分辨率的。
    // 我们必须确立一个“画布分辨率”，所有的 Position 节点都基于此。
    pub play_res_x: u32,
    pub play_res_y: u32,

    // === 媒体关联 ===
    // 存储 Option 是因为用户可能只打开字幕没打开视频
    pub video_path: Option<PathBuf>,
    pub audio_path: Option<PathBuf>,

    // 视频帧率：这对“帧对齐”逻辑至关重要
    // 如果没有视频，默认通常设为 23.976 或 24.0
    // NOTE: 未必需要
    pub fps: f64,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            title: "New Project".to_string(),
            author: "Unknown".to_string(),
            description: "".to_string(),
            play_res_x: 1920,
            play_res_y: 1080,
            video_path: None,
            audio_path: None,
            fps: 23.976,
        }
    }
}
