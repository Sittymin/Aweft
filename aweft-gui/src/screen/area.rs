use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use anyhow::Result;
use bitflags::bitflags;
use slotmap::new_key_type;

use crate::bridge::types::ThemeSettings;

use super::{
    primitives::Rcti,
    vertex::{ScrVert, ScrVertKey, VertPosition},
};

new_key_type! {
    pub struct ScrAreaKey;
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct AreaFlags: u32 {
        // NOTE: Maybe those are not necessary
        /// 禁用区域头部的下拉菜单。
        const HEADER_NO_PULLDOWN = 0b0000_0001;

        /// 标记区域内的 Regions 需要更新大小。
        const REGION_SIZE_UPDATE = 0b0000_0010;

        /// 标记活动工具需要更新。
        const ACTIVE_TOOL_UPDATE = 0b0000_0100;

        /// 标记为堆叠式全屏模式（例如文件浏览器或渲染视图的临时全屏）。
        const STACKED_FULLSCREEN = 0b0000_1000;

        /// 即使鼠标指针不在操作区域（Action Zones）内，也强制更新。
        const ACTIONZONES_UPDATE = 0b0001_0000;

        /// 标记该区域为离屏渲染。
        const OFFSCREEN = 0b0010_0000;
    }
}

/// 定义编辑器 UI 中不同区域的类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaType {
    Empty,
    // 序列的表格视图（行编辑器）：与时间线同源（DataId::Sequence），
    // 按层级扁平展示 Group/Event，支持折叠/时序/文本编辑
    LineEditor,
    // 实际渲染并可以使用交互式工具的地方(也是视频播放的地方)
    Viewport,
    // 像视频轨道一样使用长方形填充占用的时间部分
    Timeline,
    // 样式链编辑器（aweft-model 的 node_graph）
    StyleGraphEditor,
    // 应用图编辑器（aweft-model 的 application_graph）
    AppGraphEditor,
    // 曲线编辑器（aweft-model 的 animation::Action/FCurve）
    GraphEditor,
    // 摄影表 / 动作编辑器（Dope Sheet：每通道一行按时间铺开关键帧）
    DopeSheet,
    // NLA 编辑器（aweft-model 的 animation::AnimData：非破坏性动画轨道/片断）
    NlaEditor,
    // StyleManager,
    // FileBrowser,

    // // Displays original script or notes to aid in translation.
    // TranslationAssistant,

    // // A panel with tools for advanced timing adjustments (e.g., shift, stretch).
    // TimingProcessor,

    // Shows properties of the video, audio, or subtitle file.
    Properties,
}
/// 文件浏览器的视图模式
#[derive(Debug, Clone)]
pub enum ViewMode {
    List,
    Icons,
    Details,
}

/// 文件排序方式
#[derive(Debug, Clone)]
pub enum SortBy {
    Name,
    Date,
    Size,
    Type,
}

/// 时间处理器的操作类型
#[derive(Debug, Clone)]
pub enum TimingOperation {
    Shift,
    Stretch,
    ScaleToFit,
    AutoTiming,
}

/// 属性面板的标签页
#[derive(Debug, Clone)]
pub enum PropertiesTab {
    Video,
    Audio,
    Subtitle,
    Style,
    Project,
}

/// 文本选择范围
#[derive(Debug, Clone)]
pub struct TextSelection {
    pub start: usize,
    pub end: usize,
}

/// 时间轴区域的数据
#[derive(Debug, Clone)]
pub struct TimelineData {
    pub current_time: f64,           // 当前播放时间
    pub zoom_level: f32,             // 缩放级别
    pub scroll_offset: f32,          // 滚动偏移
    pub selected_events: Vec<usize>, // 选中的事件索引
    pub snap_to_keyframes: bool,     // 是否吸附到关键帧
    pub waveform_visible: bool,      // 是否显示音频波形
    pub video_preview: bool,         // 是否显示视频预览
}

/// 视频播放器区域的数据
#[derive(Debug, Clone)]
pub struct VideoData {
    pub current_frame: u32,      // 当前帧
    pub playback_rate: f32,      // 播放速率
    pub aspect_ratio: f32,       // 宽高比
    pub zoom: f32,               // 视频缩放
    pub pan_offset: (f32, f32),  // 平移偏移
    pub overlay_subtitles: bool, // 是否在视频上叠加字幕
    pub safe_area_visible: bool, // 是否显示安全区域
}

/// 字幕编辑器区域的数据
#[derive(Debug, Clone)]
pub struct SubtitleEditorData {
    pub current_line: Option<usize>, // 当前编辑行
    pub selection: TextSelection,    // 文本选择
    pub syntax_highlighting: bool,   // 是否启用语法高亮
    pub word_wrap: bool,             // 是否自动换行
    pub font_size: f32,              // 字体大小
    pub show_line_numbers: bool,     // 是否显示行号
    pub auto_complete: bool,         // 是否启用自动补全
}

/// 样式管理器区域的数据
#[derive(Debug, Clone)]
pub struct StyleManagerData {
    pub selected_style: Option<String>,             // 选中的样式名称
    pub preview_text: String,                       // 用于预览样式的文本
    pub categories_expanded: HashMap<String, bool>, // 各分类的展开状态
    pub search_filter: String,                      // 搜索过滤器文本
}

/// 文件浏览器区域的数据
#[derive(Debug, Clone)]
pub struct FileBrowserData {
    pub current_path: PathBuf,        // 当前路径
    pub selected_files: Vec<PathBuf>, // 选中的文件
    pub view_mode: ViewMode,          // 视图模式
    pub sort_by: SortBy,              // 排序方式
    pub show_hidden: bool,            // 是否显示隐藏文件
    pub file_filters: Vec<String>,    // 文件类型过滤器
}

/// 翻译助手区域的数据
#[derive(Debug, Clone)]
pub struct TranslationAssistantData {
    pub source_language: String,           // 源语言
    pub target_language: String,           // 目标语言
    pub original_script: String,           // 原始脚本内容
    pub translation_notes: String,         // 翻译笔记
    pub glossary: HashMap<String, String>, // 术语表
    pub auto_translate: bool,              // 是否启用自动翻译
}

/// 时间处理器区域的数据
#[derive(Debug, Clone)]
pub struct TimingProcessorData {
    pub operation_type: TimingOperation,    // 操作类型
    pub time_shift: f64,                    // 时间偏移量
    pub stretch_factor: f32,                // 拉伸因子
    pub selected_range: Option<(f64, f64)>, // 选中的时间范围
    pub preview_enabled: bool,              // 是否启用预览
}

/// 属性面板区域的数据
#[derive(Debug, Clone)]
pub struct PropertiesData {
    pub name: String,
    pub active_tab: PropertiesTab,          // 当前活动的标签页
    pub expanded_sections: HashSet<String>, // 已展开的部分
    pub property_filter: String,            // 属性搜索过滤器
}

/// 容纳所有具体区域数据的枚举
#[derive(Debug, Clone)]
pub enum AreaData {
    Empty,
    Timeline(TimelineData),
    Video(VideoData),
    SubtitleEditor(SubtitleEditorData),
    StyleManager(StyleManagerData),
    FileBrowser(FileBrowserData),
    TranslationAssistant(TranslationAssistantData),
    TimingProcessor(TimingProcessorData),
    Properties(PropertiesData),
}

/// 屏幕区域（Screen Area），是 UI 布局的基本单元
#[derive(Debug, Clone)]
pub struct ScrArea {
    // 定义区域的四个角顶点（Slint坐标系，左上角原点）
    v_top_left: ScrVertKey,
    v_top_right: ScrVertKey,
    v_bottom_right: ScrVertKey,
    v_bottom_left: ScrVertKey,

    // 重要：这些几何属性设为私有，只能通过计算方法访问
    // Rect bound by v_buttom_left...
    tot_rct: Rcti,

    pub area_type: AreaType,
    // pub keymap_flags: Ke
    pub area_data_history: (AreaType, AreaData),
    // NOTE: Optional and need Optimize
    // pub area_history: Vec<AreaData>,

    // Temporarily used while switching area type, otherwise this should be AreaType::Empty.
    next_area_type: Option<AreaType>,

    // Size - 设为私有
    winx: u32,
    winy: u32,

    // for area_type refresh callback
    do_refresh: bool,

    flag: AreaFlags,

    // 边距计算相关参数
    pub is_fullscreen: bool,
    pub is_temp_window: bool,
    pub is_single_area: bool,
}

impl ScrArea {
    /// 读取左上角顶点键
    pub fn v_top_left(&self) -> ScrVertKey {
        self.v_top_left
    }

    /// 读取右上角顶点键
    pub fn v_top_right(&self) -> ScrVertKey {
        self.v_top_right
    }

    /// 读取右下角顶点键
    pub fn v_bottom_right(&self) -> ScrVertKey {
        self.v_bottom_right
    }

    /// 读取左下角顶点键
    pub fn v_bottom_left(&self) -> ScrVertKey {
        self.v_bottom_left
    }
    pub fn set_vert(&mut self, vert_pos_and_key: VertPosition) -> Result<()> {
        match vert_pos_and_key {
            VertPosition::TopLeft(tl) => self.v_top_left = tl,
            VertPosition::TopRight(tr) => self.v_top_right = tr,
            VertPosition::BottomLeft(bl) => self.v_bottom_left = bl,
            VertPosition::BottomRight(br) => self.v_bottom_right = br,
        }
        Ok(())
    }
    pub fn tot_rct(&self) -> Rcti {
        self.tot_rct
    }
    /// 创建新的ScrArea实例 - 适配Slint坐标系（左上角原点）
    /// 只接受顶点引用和区域类型，所有几何属性都需要后续计算
    pub fn new(
        v_top_left: ScrVertKey,
        v_top_right: ScrVertKey,
        v_bottom_right: ScrVertKey,
        v_bottom_left: ScrVertKey,
        area_type: AreaType,
        is_temp_window: bool,
        is_fullscreen: bool,
        is_single_area: bool,
    ) -> Self {
        let area_data = Self::create_area_data(area_type.clone());

        Self {
            v_top_left,
            v_top_right,
            v_bottom_right,
            v_bottom_left,
            // 初始化为零值，必须通过calculate_totrct_and_size()计算
            tot_rct: Rcti {
                xmin: 0,
                xmax: 0,
                ymin: 0,
                ymax: 0,
            },
            area_type: area_type.clone(),
            area_data_history: (area_type, area_data),
            next_area_type: None,
            // 初始化为零，必须通过calculate_totrct_and_size()计算
            winx: 0,
            winy: 0,
            do_refresh: false,
            flag: AreaFlags::empty(),
            is_fullscreen,
            is_temp_window,
            is_single_area,
        }
    }

    /// 基于Blender的area_calc_totrct实现，从顶点位置计算所有几何属性
    /// 这是唯一应该修改tot_rct、winx、winy的方法
    ///
    /// 重要：这个方法体现了"区域几何完全由顶点决定"的设计原则
    pub fn calculate_totrct_and_size(
        &mut self,
        vertices: &slotmap::SlotMap<ScrVertKey, ScrVert>,
        theme_settings: &ThemeSettings,
    ) {
        // 1. 从顶点计算基础矩形 - 这是几何的"真相来源"
        let tl = &vertices.get(self.v_top_left).unwrap().vec;
        let tr = &vertices.get(self.v_top_right).unwrap().vec;
        let bl = &vertices.get(self.v_bottom_left).unwrap().vec;

        // 直接从顶点位置设置边界（Slint坐标系：左上角原点，y向下增长）
        self.tot_rct.xmin = tl.x;
        self.tot_rct.xmax = tr.x;
        self.tot_rct.ymin = tl.y;
        self.tot_rct.ymax = bl.y;

        // 2. 根据DPI和UI设置计算边距
        let px = theme_settings.calculate_margin(false); // 区域间边距

        // 3. 应用边距调整
        // 只有非全屏、非临时窗口、且非单 area 时才应用边距
        if self.has_margin() {
            // 左边距
            self.tot_rct.xmin += px;

            // 右边距
            self.tot_rct.xmax -= px;

            // 上边距（Slint坐标系中y=0是顶部）
            self.tot_rct.ymin += px;

            // 下边距（Slint坐标系中y增长是向下）
            self.tot_rct.ymax -= px;
        }

        // 4. 最终计算winx和winy（Blender注释："for speedup"）
        self.winx = self.tot_rct.width();
        self.winy = self.tot_rct.height();
    }

    /// 该 area 是否应用区域间边距（margin）。
    ///
    /// margin 只在非全屏、非临时窗口、非单 area 时存在。单 area 时没有相邻
    /// 区域，收缩边距会让内容偏离窗口边缘。
    pub fn has_margin(&self) -> bool {
        !self.is_fullscreen && !self.is_temp_window && !self.is_single_area
    }

    /// 布局矩形（**含 margin 收缩**）：UI 显示、命中检测、预览计算用。
    ///
    /// 与 `geometry_rect`（原始顶点坐标）差一个 margin。**几何运算**（创建/
    /// 移动顶点、join/split 内部）必须用 `geometry_rect`；**UI 呈现**必须用
    /// `layout_rect`。二者不要混用——坐标基准不一致是历史上 bug 的根源。
    pub fn layout_rect(&self) -> (&Rcti, u32, u32) {
        (&self.tot_rct, self.winx, self.winy)
    }

    /// 几何矩形（**原始顶点坐标，不含 margin**）：几何运算用。
    ///
    /// 与 `layout_rect`（含 margin 收缩）差一个 margin。**几何运算**（创建/
    /// 移动顶点、join/split 内部）必须用 `geometry_rect`；**UI 呈现**必须用
    /// `layout_rect`。二者不要混用——坐标基准不一致是历史上 bug 的根源。
    pub fn geometry_rect(&self, vertices: &slotmap::SlotMap<ScrVertKey, ScrVert>) -> Rcti {
        let tl = &vertices.get(self.v_top_left).unwrap().vec;
        let tr = &vertices.get(self.v_top_right).unwrap().vec;
        let bl = &vertices.get(self.v_bottom_left).unwrap().vec;

        Rcti {
            xmin: tl.x,
            xmax: tr.x,
            ymin: tl.y,
            ymax: bl.y,
        }
    }

    /// 检查区域几何是否已正确计算（用于调试）
    pub fn is_geometry_calculated(&self) -> bool {
        self.winx > 0 && self.winy > 0
    }

    /// 获取私有字段flag的引用
    pub fn flag(&self) -> AreaFlags {
        self.flag
    }

    /// 设置私有字段flag
    pub fn set_flag(&mut self, flag: AreaFlags) {
        self.flag = flag;
    }

    /// 获取do_refresh字段
    pub fn do_refresh(&self) -> bool {
        self.do_refresh
    }

    /// 设置do_refresh字段
    pub fn set_do_refresh(&mut self, refresh: bool) {
        self.do_refresh = refresh;
    }

    pub fn create_area_data(area_type: AreaType) -> AreaData {
        match area_type {
            AreaType::Timeline => AreaData::Timeline(TimelineData {
                current_time: 0.0,
                zoom_level: 1.0,
                scroll_offset: 0.0,
                selected_events: Vec::new(),
                snap_to_keyframes: false,
                waveform_visible: true,
                video_preview: true,
            }),

            _ => AreaData::Empty,
        }
    }
}
