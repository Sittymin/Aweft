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
    /// 追加一个视频会话（首个视频自动成为主视频）
    Append { uri: String },
    /// 原地替换某个视频会话（右键"更换视频"；重新选择文件替换当前）
    Replace { index: usize, uri: String },
    /// 切换指定视频的音频静音状态。
    ToggleMute { index: usize },
    /// 选择指定视频的实时播放音轨（波形 Area 点击左侧音轨时触发）。
    SelectAudioTrack { video_index: usize, track_index: usize },
    /// 更新视口视频选择（普通点击替换；Ctrl 切换；Shift 追加；None 普通点击清空）。
    Select {
        index: Option<usize>,
        modifier: crate::bridge::types::SlintSelectionModifier,
    },
    /// 设置主视频（右键非主视频"设为主视频"）
    SetMain { index: usize },
    /// 拖拽移动非主视频在视口中的位置（世界坐标，相对主视频世界矩形左上角）
    MoveVideo { index: usize, x: f64, y: f64 },
    /// 播放视口选择集合；无选择时播放主视频
    Play,
    /// 暂停视口选择集合；无选择时暂停主视频
    Pause,
    /// 选择集合播放/暂停切换（时间线 Header 播放按钮）
    TogglePlayback,
    /// 时间条拖拽开始：普通拖拽只选中该视频，批量拖拽作用于选择集合。
    SeekBegin { index: usize, is_batch: bool },
    /// 时间条拖拽中：按拖拽锚点的时间增量移动各目标视频到指定毫秒。
    SeekPreview { timestamp: i64 },
    /// 时间条拖拽结束：各目标按拖拽前的播放态从新位置继续。
    SeekCommit,
    /// 独立 seek：绝对定位指定视频会话，不参与拖拽会话、不改变视口选中集合；
    /// 该视频不是焦点视频时也不改动画域当前时间与全局播放头（音频波形标尺用）。
    SeekStandalone { index: usize, timestamp: i64 },
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
