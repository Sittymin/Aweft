/// 媒体相关 Action
#[derive(Debug, Clone)]
pub enum MediaAction {
    /// 视频相关操作
    Video(VideoAction),
    /// 字幕相关操作
    Subtitle(SubtitleAction),
}

/// 视频操作
#[derive(Debug, Clone)]
pub enum VideoAction {
    /// 初始化视频
    Initialize { uri: String },
    /// 播放
    Play,
    /// 暂停
    Pause,
    /// 跳转
    Seek { timestamp: i64 },
    /// 更新视频帧
    UpdateFrame,
}

/// 字幕操作
#[derive(Debug, Clone)]
pub enum SubtitleAction {
    /// 加载字幕文件
    Load { path: String },
    /// 渲染字幕
    Render { timestamp: i64 },
}
