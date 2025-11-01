use crate::screen::area::ScrAreaKey;
use crate::screen::map::ScrAreaMap;
use crate::screen::primitives::{Adjacency, Rcti};
use crate::screen::vertex::ScrVertKey;
use crate::types::{SplitPreviewOverlayData, UISettings};
use slint::PhysicalSize;

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
    },
}

/// 应用状态和逻辑的唯一管理者
pub struct AppState {
    pub screen_map: ScrAreaMap,
    pub ui_settings: UISettings,
    pub window_size: PhysicalSize,
    pub initialized: bool,
    /// 当前拖拽操作状态
    pub current_drag_operation: DragOperation,
    /// 保持 GStreamer 管道生命周期，防止被提前释放
    pub video_pipeline: Option<gstreamer::Pipeline>,
    /// 保持 Sink 生命周期
    pub video_sink: Option<gstreamer::Element>,
    /// WGPU 视频 Sink 实例
    pub wgpu_sink: Option<crate::slint_video_sink::SlintWGPUSink>,
    /// 字幕渲染器
    pub subtitle_renderer: Option<crate::wgpu_subtitle_renderer::SubtitleRenderer>,
    /// libass 字幕渲染器
    pub libass_renderer: Option<aweft_libass::SubtitleRenderer>,
}

impl AppState {
    /// 构造函数，从 Slint 窗口初始化
    pub fn new(window: &slint::Window) -> Self {
        Self {
            screen_map: ScrAreaMap::new(),
            ui_settings: UISettings::from_slint_window(window),
            window_size: window.size(),
            initialized: false,
            current_drag_operation: DragOperation::None,
            video_pipeline: None,
            video_sink: None,
            wgpu_sink: None,
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
}


