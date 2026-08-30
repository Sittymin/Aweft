use std::rc::Rc;

use crate::bridge::types::{SplitPreviewOverlayData, ThemeSettings};
use crate::screen::area::ScrAreaKey;
use crate::screen::edge::ScrEdgeKey;
use crate::screen::map::ScrAreaMap;
use crate::screen::primitives::{Adjacency, Rcti};
use crate::screen::vertex::ScrVertKey;
use aweft_model::project::Project;
use slint::PhysicalSize;
use std::sync::atomic::AtomicBool;

/// 拖拽操作的状态信息
#[derive(Debug, Clone)]
pub enum DragOperation {
    /// 无拖拽操作
    None,
    /// 分割操作 - 存储分割的源区域和预览数据
    Split {
        source_area: ScrAreaKey,
        preview_data: SplitPreviewOverlayData,
    },
    /// 合并操作 - 存储源区域、目标区域和邻接关系
    Join {
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
        adjacency: Adjacency,
    },
    /// 边移动操作 - 存储移动状态信息
    MoveEdge {
        /// 轴方向：true=水平轴(改变Y坐标)，false=垂直轴(改变X坐标)
        dir_axis_horizontal: bool,
        /// 原始轴坐标值
        origval: u32,
        /// 向正方向的最大位移距离
        bigger: u32,
        /// 向负方向的最大位移距离
        smaller: u32,
        /// 参与移动的顶点键集合
        moving_vert_keys: Vec<ScrVertKey>,
        // 影响到的 Edge 和 Area （用于局部更新 UI）
        edge_keys_rc: Rc<Vec<ScrEdgeKey>>,
        area_keys_rc: Rc<Vec<ScrAreaKey>>,
    },
}

/// 单个视频会话：一条 GStreamer 管线 + WGPU sink + 视口布局/播放状态。
/// 多视频 = `AppState::video_sessions` 的多条会话；`main_video` 是主视频下标，
/// `selected_videos` 是有序选择集合，末项即当前焦点。
pub struct VideoSession {
    pub uri: String,
    // 保持 GStreamer 管道生命周期，防止被提前释放
    pub pipeline: gstreamer::Pipeline,
    // 保持 Sink 生命周期
    pub sink_element: gstreamer::Element,
    // WGPU 视频 Sink 实例
    pub wgpu_sink: crate::render::video_sink::SlintWGPUSink,
    // 视口世界坐标位置（相对主视频世界矩形左上角；级联错开，可拖拽移动）
    pub position_x: f64,
    pub position_y: f64,
    // 当前播放位置（秒）。暂停时编辑器时间权威，这里保留最后一次位置供时间条显示。
    pub current_time: f64,
    // 是否显式静音该视频音轨；与选择/transport 解耦。
    pub muted: bool,
}

/// 一次视频时间条拖拽中的单个目标快照。
#[derive(Debug, Clone, Copy)]
pub struct VideoSeekTarget {
    pub index: usize,
    pub origin_time: f64,
    pub was_playing: bool,
}

/// 视频时间条拖拽会话。批量拖拽以锚点的时间增量移动全部目标。
#[derive(Debug, Clone)]
pub struct VideoSeekSession {
    pub anchor_time: f64,
    pub targets: Vec<VideoSeekTarget>,
}

/// 应用状态和逻辑的唯一管理者
pub struct AppState {
    // 单一持久化数据源
    pub project: Project,

    pub screen_map: ScrAreaMap,
    pub theme_settings: ThemeSettings,
    pub window_size: PhysicalSize,
    pub initialized: bool,
    // 当前拖拽操作状态
    pub current_drag_operation: DragOperation,
    // 全部视频会话（多视频播放/导航的核心数据）
    pub video_sessions: Vec<VideoSession>,
    // 与 video_sessions 同下标的波形后台任务取消令牌。媒体替换/teardown 时置位，
    // 防止迟到的后台结果覆盖新会话。
    pub waveform_cancellations: Vec<std::sync::Arc<AtomicBool>>,
    // 主视频下标：唯一在视口带矩形参考线；无会话时为 None
    pub main_video: Option<usize>,
    // 视口选择集合（有序、去重）；时间线 transport 控制它，为空时控制主视频
    pub selected_videos: Vec<usize>,
    // 播放状态（供播放/暂停按钮图标与后续状态同步；随管线实际状态逐帧校正）
    pub video_playing: bool,
    // Some 表示视频时间条拖拽中；会话显式记录目标、原始时间和播放态。
    pub video_seek_session: Option<VideoSeekSession>,
    // 渲染器 GPU 句柄（RenderingSetup 时缓存；打开视频时按需连接 Sink）
    pub video_wgpu: Option<crate::render::video_sink::WgpuHandles>,
    // 字幕渲染器
    pub subtitle_renderer: Option<crate::render::subtitle::SubtitleRenderer>,
    // libass 字幕渲染器
    pub libass_renderer: Option<aweft_libass::SubtitleRenderer>,
}

impl AppState {
    /// 构造函数，接收预先创建的 ThemeSettings
    pub fn new(window: &slint::Window, theme_settings: ThemeSettings) -> Self {
        Self {
            project: Project::new(),
            screen_map: ScrAreaMap::new(),
            theme_settings,
            window_size: window.size(),
            initialized: false,
            current_drag_operation: DragOperation::None,
            video_sessions: Vec::new(),
            waveform_cancellations: Vec::new(),
            main_video: None,
            selected_videos: Vec::new(),
            video_playing: false,
            video_seek_session: None,
            video_wgpu: None,
            subtitle_renderer: None,
            libass_renderer: None,
        }
    }

    /// 以包含端坐标返回当前窗口的物理像素矩形 (0,0)-(w-1,h-1)
    pub fn current_window_rect(&self) -> Rcti {
        Rcti {
            xmin: 0,
            ymin: 0,
            xmax: self.window_size.width.saturating_sub(1),
            ymax: self.window_size.height.saturating_sub(1),
        }
    }

    /// 当前焦点视频：选择集合末项优先，无选择时回落主视频。
    pub fn active_video(&self) -> Option<usize> {
        self.selected_videos.last().copied().or(self.main_video)
    }
}
