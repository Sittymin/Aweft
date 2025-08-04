use crate::screen::geometry::*;
use anyhow::{Context, Result, anyhow};
use bitflags::bitflags;
use slint::PhysicalSize;
use slotmap::{SlotMap, new_key_type};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
// Slint 类型在 lib.rs 中导入了
use crate::{ActionZoneCorner, SplitPreviewOverlayData};

new_key_type! {
    pub struct ScrVertKey;
    pub struct ScrEdgeKey;
    pub struct ScrAreaKey;
}

/// UI缩放相关的设置
///
/// 设计原则（修正版）：
/// - window().size() 返回物理像素大小
/// - 内部所有计算使用物理像素（与Blender一致）
/// - 边距等UI元素在物理像素基础上缩放
/// - 最终传递给Slint时除以scale_factor转换为逻辑坐标
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UISettings {
    scale_factor: f64,                // 缩放因子（从Slint获取）
    base_border_width: f64,           // 基础边框宽度（物理像素）
    base_actionzone_side_length: f64, // 边角区域边长（物理像素）
}

impl Default for UISettings {
    fn default() -> Self {
        Self {
            scale_factor: 1.0,
            base_border_width: 1.0,
            base_actionzone_side_length: 5.0,
        }
    }
}

impl UISettings {
    /// 从Slint窗口获取缩放设置
    pub fn from_slint_window(window: &slint::Window) -> Self {
        Self {
            scale_factor: window.scale_factor() as f64,
            base_border_width: 1.0, // 默认边框宽度
            base_actionzone_side_length: 10.0,
        }
    }

    pub fn update_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// 计算边距像素值（返回物理像素）
    pub fn calculate_margin(&self, is_edge: bool) -> u32 {
        if is_edge {
            // 窗口边缘的边距（物理像素）
            (self.get_scale_factor() * 2.0).min(self.get_border_width()) as u32
        } else {
            // 区域间的边距（物理像素）
            (self.get_border_width() * self.get_scale_factor()).max(self.get_scale_factor()) as u32
        }
    }

    pub fn get_scale_factor(&self) -> f64 {
        self.scale_factor
    }

    // NOTE: 边缘大小 Slint 会自动缩放
    // pub fn get_border_width_logical(&self) -> f64 {
    //     self.base_border_width / self.scale_factor
    // }
    pub fn get_border_width(&self) -> f64 {
        self.base_border_width
    }
    // NOTE: 边缘大小 Slint 会自动缩放
    // pub fn get_actionzone_side_length_logical(&self) -> f64 {
    //     self.base_actionzone_side_length / self.scale_factor
    // }
    pub fn get_actionzone_side_length(&self) -> f64 {
        self.base_actionzone_side_length
    }
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

/// 2D 整数向量
#[derive(Debug, Clone)]
pub struct Vec2u32 {
    pub x: u32,
    pub y: u32,
}
#[derive(Debug, Clone)]
pub struct Vec2f64 {
    pub x: f64,
    pub y: f64,
}
impl Vec2u32 {
    /// 安全地将 Vec2f32 转换为 Vec2u32，返回 Result
    ///
    /// # Errors
    ///
    /// 当输入的 x 或 y 坐标为负数时返回错误，因为无法将负数转换为无符号整数
    ///
    /// # Examples
    ///
    /// ```
    /// # use aweft_gui::types::{Vec2u32, Vec2f32};
    /// let positive = Vec2f32 { x: 10.5, y: 20.3 };
    /// let result = Vec2u32::try_from_f32(positive);
    /// assert!(result.is_ok());
    ///
    /// let negative = Vec2f32 { x: -5.0, y: 10.0 };
    /// let result = Vec2u32::try_from_f32(negative);
    /// assert!(result.is_err());
    /// ```
    pub fn try_from_f32(value: Vec2f64) -> Result<Self> {
        if value.x < 0.0 || value.y < 0.0 {
            return Err(anyhow!(
                "无法将负数坐标转换为无符号整数: x={}, y={}。负数会导致数据丢失",
                value.x,
                value.y
            ));
        }
        Ok(Self {
            x: value.x as u32,
            y: value.y as u32,
        })
    }
}

/// 整数定义的矩形区域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rcti {
    pub xmin: u32,
    pub xmax: u32,
    pub ymin: u32,
    pub ymax: u32,
}

impl Rcti {
    /// 从四个顶点坐标计算矩形边界（类似Blender中的实现）
    /// 顶点顺序不重要，函数会自动计算边界框
    pub fn from_vertices(v1: &Vec2u32, v2: &Vec2u32, v3: &Vec2u32, v4: &Vec2u32) -> Self {
        // 根据四个顶点找出最小和最大的x、y坐标
        let xmin = v1.x.min(v2.x).min(v3.x).min(v4.x);
        let xmax = v1.x.max(v2.x).max(v3.x).max(v4.x);
        let ymin = v1.y.min(v2.y).min(v3.y).min(v4.y);
        let ymax = v1.y.max(v2.y).max(v3.y).max(v4.y);

        Self {
            xmin,
            xmax,
            ymin,
            ymax,
        }
    }

    /// 获取矩形的宽度
    pub fn width(&self) -> u32 {
        self.xmax - self.xmin + 1
    }

    /// 获取矩形的高度
    pub fn height(&self) -> u32 {
        self.ymax - self.ymin + 1
    }
}

/// 浮点数定义的矩形区域
#[derive(Debug, Clone)]
pub struct Rctf {
    pub xmin: f32,
    pub xmax: f32,
    pub ymin: f32,
    pub ymax: f32,
}

/// 定义编辑器 UI 中不同区域的类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaType {
    Empty,
    // The main timeline view with audio waveform and subtitle events.
    Timeline,
    Video,
    SubtitleEditor,
    StyleManager,
    FileBrowser,

    // Displays original script or notes to aid in translation.
    TranslationAssistant,

    // A panel with tools for advanced timing adjustments (e.g., shift, stretch).
    TimingProcessor,

    // Shows properties of the video, audio, or subtitle file.
    Properties,
}

/// 屏幕顶点（Screen Vertex），构成区域布局的基础元素
#[derive(Debug, Clone)]
pub struct ScrVert {
    // 移除手动维护的双向链表指针 - 让SlotMap来管理顺序
    // 保留 newv 字段用于重复顶点合并功能
    pub newv: Option<ScrVertKey>, // 用于重复顶点合并的新顶点引用
    pub vec: Vec2u32,
    // NOTE: Maybe is not necessary (Copy from Blender code)
    // first one used internally, second one for tools
    pub flag: i16,
    pub edit_flag: i16,
}

/// 屏幕边缘（Screen Edge），连接两个顶点
#[derive(Debug, Clone)]
pub struct ScrEdge {
    // 移除手动维护的双向链表指针 - 让SlotMap来管理顺序
    pub v1: ScrVertKey, // 起始顶点
    pub v2: ScrVertKey, // 结束顶点
    pub border: bool,   // 标记是否为屏幕最外围的边缘
    pub flag: i16,
}

// WARN: This datas defination is AI generated

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

/// 屏幕交互区域（Action Zones），用于处理如区域分割、合并等UI交互
#[derive(Debug, Clone)]
pub enum ActionZoneType {
    Area, // 用于区域的分割、合并或交换
    Region {
        // region: RegionId,   // 关联的 Region
        edge: EdgeDirection, // 可拖拽的边缘方向
    },
    Fullscreen {
        // region: RegionId, // 退出全屏后要恢复到的 Region
    },
    RegionScroll {
        // region: RegionId,       // 关联的 Region
        direction: ScrollDirection, // 滚动方向
    },
}

/// 可拖拽边缘的方向
#[derive(Debug, Clone)]
pub enum EdgeDirection {
    RightToTopLeft,   // 对应 AE_RIGHT_TO_TOPLEFT
    LeftToTopRight,   // 对应 AE_LEFT_TO_TOPRIGHT
    TopToBottomRight, // 对应 AE_TOP_TO_BOTTOMRIGHT
    BottomToTopLeft,  // 对应 AE_BOTTOM_TO_TOPLEFT
}

/// 滚动方向
#[derive(Debug, Clone)]
pub enum ScrollDirection {
    Vertical,   // 对应 AZ_SCROLL_VERT
    Horizontal, // 对应 AZ_SCROLL_HOR
}

/// 定义一个具体的操作区域
#[derive(Debug, Clone)]
pub struct ActionZone {
    pub zone_type: ActionZoneType, // 包含了类型、关联区域、边缘等信息
    pub rect: Rcti,                // 操作区域的矩形范围
    pub alpha: f32,                // 用于淡入淡出的透明度
}

/// 屏幕区域（Screen Area），是 UI 布局的基本单元
#[derive(Debug, Clone)]
pub struct ScrArea {
    // 定义区域的四个角顶点（Slint坐标系，左上角原点）
    pub v_top_left: ScrVertKey,
    pub v_top_right: ScrVertKey,
    pub v_bottom_right: ScrVertKey,
    pub v_bottom_left: ScrVertKey,

    // 重要：这些几何属性设为私有，只能通过计算方法访问
    // Rect bound by v_buttom_left...
    tot_rct: Rcti,

    pub area_type: AreaType,
    // pub keymap_flags: Ke
    pub area_data_history: (AreaType, AreaData),
    // NOTE: Optional and need Optimize
    // pub area_history: Vec<AreaData>,

    // Temporarily used while switching area type, otherwise this should be AreaType::Empty.
    pub next_area_type: Option<AreaType>,

    // Size - 设为私有
    winx: u32,
    winy: u32,

    // for area_type refresh callback
    do_refresh: bool,

    flag: AreaFlags,

    // region_active_win

    // 改为可选的ActionZone列表，以支持多个action zone
    pub action_zones: Option<Vec<ActionZone>>,
}

impl ScrArea {
    /// 创建新的ScrArea实例 - 适配Slint坐标系（左上角原点）
    /// 只接受顶点引用和区域类型，所有几何属性都需要后续计算
    pub fn new(
        v_top_left: ScrVertKey,
        v_top_right: ScrVertKey,
        v_bottom_right: ScrVertKey,
        v_bottom_left: ScrVertKey,
        area_type: AreaType,
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
            action_zones: None,
        }
    }

    /// 基于Blender的area_calc_totrct实现，从顶点位置计算所有几何属性
    /// 这是唯一应该修改tot_rct、winx、winy的方法
    ///
    /// 重要：这个方法体现了"区域几何完全由顶点决定"的设计原则
    pub fn calculate_totrct_and_size(
        &mut self,
        vertices: &slotmap::SlotMap<ScrVertKey, ScrVert>,
        ui_settings: &UISettings,
        is_fullscreen: bool,
        is_temp_window: bool,
        is_single_area: bool,
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
        let px = ui_settings.calculate_margin(false); // 区域间边距

        // 3. 应用边距调整
        // 只有在非全屏、非临时窗口、且有多个区域的情况下才应用边距
        if !is_fullscreen && !is_temp_window && !is_single_area {
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

    /// 只读访问几何属性
    /// 不要使用这个属性来计算位置（它包含边距调整）
    pub fn geometry(&self) -> (&Rcti, u32, u32) {
        (&self.tot_rct, self.winx, self.winy)
    }

    /// 获取原始几何信息（边距调整之前的）
    pub fn get_raw_geometry(&self, vertices: &slotmap::SlotMap<ScrVertKey, ScrVert>) -> Rcti {
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

    // NOTE: 完整的编辑器生命周期管理（参考 Blender 设计）
    // TODO: 在完成整体 Demo 后再具体实现
    //
    // 1. operatortypes() -> 注册所有可用的操作符
    // 2. keymap() -> 设置键盘快捷键映射
    // 3. dropboxes() -> 设置拖放操作的处理逻辑
    // 4. gizmos() -> 注册 3D 视图中的操作小工具
    //
    // 编辑器实例管理
    // 5. create() -> 创建一个新的编辑器实例
    // 6. init() -> 初始化编辑器状态和事件处理
    // 7. listener() -> 监听数据变化通知并响应
    // 8. deactivate() -> 处理鼠标移出编辑器的事件
    // 9. refresh() -> 在文件加载或数据变更后刷新视图
    // 10. duplicate() -> 复制当前编辑器的状态
    // 11. exit() -> 当编辑器被隐藏或移除时执行清理
    // 12. free() -> 释放编辑器占用的所有内存资源
    //
    // 数据管理
    // 13. id_remap() -> 当数据块的 ID 发生变化时进行重映射
    // 14. foreach_id() -> 遍历编辑器引用的所有 ID
    // 15. blend_read/write() -> 在文件读写时序列化或反序列化编辑器数据
}

// 用于在ScrAreaMap当中匹配设置ScrArea
pub enum VertPosition {
    TopLeft(ScrVertKey),
    TopRight(ScrVertKey),
    BottomRight(ScrVertKey),
    BottomLeft(ScrVertKey),
}

/// 屏幕区域地图，管理所有的顶点、边和区域
#[derive(Debug)]
pub struct ScrAreaMap {
    pub verts: SlotMap<ScrVertKey, ScrVert>, // 存储所有顶点
    pub edges: SlotMap<ScrEdgeKey, ScrEdge>, // 存储所有边
    pub areas: SlotMap<ScrAreaKey, ScrArea>, // 存储所有区域
}

impl ScrAreaMap {
    pub fn new() -> Self {
        ScrAreaMap {
            verts: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            areas: SlotMap::with_key(),
        }
    }
    pub fn set_area_vert(
        &mut self,
        area_key: ScrAreaKey,
        vert_pos_and_key: VertPosition,
    ) -> Result<()> {
        let area = self
            .areas
            .get_mut(area_key)
            .context(format!("不存在的area: {:?}", area_key))?;
        match vert_pos_and_key {
            VertPosition::TopLeft(tl) => area.v_top_left = tl,
            VertPosition::TopRight(tr) => area.v_top_right = tr,
            VertPosition::BottomLeft(bl) => area.v_bottom_left = bl,
            VertPosition::BottomRight(br) => area.v_bottom_right = br,
        }
        Ok(())
    }
    /// 计算指定角落的绝对坐标位置
    fn calculate_corner_position(
        area: &ScrArea,
        corner: ActionZoneCorner,
        offset_x: f64,
        offset_y: f64,
        actionzone_side_length: f64,
    ) -> Vec2f64 {
        let base_pos = match corner {
            ActionZoneCorner::TopLeft => Vec2f64 {
                x: area.tot_rct.xmin as f64,
                y: area.tot_rct.ymin as f64,
            },
            ActionZoneCorner::TopRight => Vec2f64 {
                x: (area.tot_rct.xmax as f64 - actionzone_side_length),
                y: area.tot_rct.ymin as f64,
            },
            ActionZoneCorner::BottomLeft => Vec2f64 {
                x: area.tot_rct.xmin as f64,
                y: (area.tot_rct.ymax as f64 - actionzone_side_length),
            },
            ActionZoneCorner::BottomRight => Vec2f64 {
                x: (area.tot_rct.xmax as f64 - actionzone_side_length),
                y: (area.tot_rct.ymax as f64 - actionzone_side_length),
            },
        };

        Vec2f64 {
            x: base_pos.x + offset_x,
            y: base_pos.y + offset_y,
        }
    }

    pub fn find_area_with_actionzone(
        &self,
        area_key: ScrAreaKey,
        corner: ActionZoneCorner,
        offset_x: f64,
        offset_y: f64,
        actionzone_side_length: f64,
    ) -> Option<ScrAreaKey> {
        let area = self.areas.get(area_key).unwrap();
        let mouse_pos = Self::calculate_corner_position(
            area,
            corner,
            offset_x,
            offset_y,
            actionzone_side_length,
        );
        println!("当前计算出的鼠标绝对位置: {:#?}", mouse_pos);

        self.find_area_at_position(mouse_pos.x as u32, mouse_pos.y as u32)
    }

    /// 将偏移量转换为相对于区域左上角的偏移量
    pub fn convert_corner_offset_to_relative_offset(
        &self,
        area_key: ScrAreaKey,
        corner: ActionZoneCorner,
        offset_x: f64,
        offset_y: f64,
        actionzone_side_length: f64,
    ) -> (f64, f64) {
        let area = self.areas.get(area_key).unwrap();
        let absolute_pos = Self::calculate_corner_position(
            area,
            corner,
            offset_x,
            offset_y,
            actionzone_side_length,
        );

        (
            absolute_pos.x - area.tot_rct.xmin as f64,
            absolute_pos.y - area.tot_rct.ymin as f64,
        )
    }
    pub fn calculate_split_areas(
        &self,
        area_key: ScrAreaKey,
        original_offset_x: f64,
        original_offset_y: f64,
        relative_offset_x: f64,
        relative_offset_y: f64,
    ) -> SplitPreviewOverlayData {
        let area = self.areas.get(area_key).unwrap();
        let (tot_rct, winx, winy) = area.geometry();

        // 定义最小区域尺寸（像素）
        const MIN_AREA_SIZE: u32 = 50;

        // 根据原始偏移量的绝对值大小决定分割方向
        let is_horizontal_split = original_offset_x.abs() > original_offset_y.abs();

        if is_horizontal_split {
            // 水平分割（垂直分割线）
            let split_x = tot_rct.xmin + relative_offset_x as u32;

            // WARN: TODO: 这里返回如果实现边拆分后需要合并距离近的顶点
            // 检查是否可以进行水平分割
            if winx < MIN_AREA_SIZE * 2
                || split_x < tot_rct.xmin + MIN_AREA_SIZE
                || split_x > tot_rct.xmax - MIN_AREA_SIZE
            {
                // 无法分割，返回完整的原始区域数据
                return SplitPreviewOverlayData {
                    x1: tot_rct.xmin as f32,
                    y1: tot_rct.ymin as f32,
                    width1: winx as f32,
                    height1: winy as f32,
                    x2: 0.0,
                    y2: 0.0,
                    width2: 0.0,
                    height2: 0.0,
                };
            }

            // 第一个区域（左侧）
            let x1 = tot_rct.xmin;
            let y1 = tot_rct.ymin;
            let width1 = split_x - tot_rct.xmin;
            let height1 = winy;

            // 第二个区域（右侧）
            let x2 = split_x;
            let y2 = tot_rct.ymin;
            // ymax - ymin 会少一个像素，所以加上(后续类似的相减一样)
            let width2 = tot_rct.xmax - split_x + 1;
            let height2 = winy;

            SplitPreviewOverlayData {
                x1: x1 as f32,
                y1: y1 as f32,
                width1: width1 as f32,
                height1: height1 as f32,
                x2: x2 as f32,
                y2: y2 as f32,
                width2: width2 as f32,
                height2: height2 as f32,
            }
        } else {
            // 垂直分割（水平分割线）
            let split_y = tot_rct.ymin + relative_offset_y as u32;

            // 检查是否可以进行垂直分割
            if winy < MIN_AREA_SIZE * 2
                || split_y < tot_rct.ymin + MIN_AREA_SIZE
                || split_y > tot_rct.ymax - MIN_AREA_SIZE
            {
                // 无法分割，返回完整的原始区域数据
                return SplitPreviewOverlayData {
                    x1: tot_rct.xmin as f32,
                    y1: tot_rct.ymin as f32,
                    width1: winx as f32,
                    height1: winy as f32,
                    x2: 0.0,
                    y2: 0.0,
                    width2: 0.0,
                    height2: 0.0,
                };
            }

            // 第一个区域（上方）
            let x1 = tot_rct.xmin;
            let y1 = tot_rct.ymin;
            let width1 = winx;
            let height1 = split_y - tot_rct.ymin;

            // 第二个区域（下方）
            let x2 = tot_rct.xmin;
            let y2 = split_y;
            let width2 = winx;
            let height2 = tot_rct.ymax - split_y + 1;

            SplitPreviewOverlayData {
                x1: x1 as f32,
                y1: y1 as f32,
                width1: width1 as f32,
                height1: height1 as f32,
                x2: x2 as f32,
                y2: y2 as f32,
                width2: width2 as f32,
                height2: height2 as f32,
            }
        }
    }
    /// 根据物理像素坐标查找鼠标所在的区域
    pub fn find_area_at_position(&self, x: u32, y: u32) -> Option<ScrAreaKey> {
        for (area_key, area) in &self.areas {
            let (totrct, _, _) = area.geometry();

            // 使用物理像素坐标进行比较
            if x >= totrct.xmin && x <= totrct.xmax && y >= totrct.ymin && y <= totrct.ymax {
                return Some(area_key);
            }
        }
        None
    }

    /// 根据区域类型查找区域（类似Blender的spacetype过滤）
    pub fn find_area_at_position_with_type(
        &self,
        x: f32,
        y: f32,
        area_type: Option<AreaType>,
    ) -> Option<ScrAreaKey> {
        for (area_key, area) in &self.areas {
            let (totrct, _, _) = area.geometry();

            if x >= totrct.xmin as f32
                && x <= totrct.xmax as f32
                && y >= totrct.ymin as f32
                && y <= totrct.ymax as f32
            {
                // 如果指定了类型，检查类型是否匹配
                if let Some(target_type) = &area_type {
                    if &area.area_type == target_type {
                        return Some(area_key);
                    }
                } else {
                    // 没有指定类型，返回第一个匹配的区域
                    return Some(area_key);
                }
            }
        }
        None
    }
    pub fn scale_vertices_to_window(
        &mut self,
        old_rect: &Rcti,
        new_rect: &Rcti,
        ui_setting: &UISettings,
    ) {
        // 1. 计算缩放比例
        let scale_x =
            (new_rect.xmax - new_rect.xmin) as f64 / (old_rect.xmax - old_rect.xmin) as f64;
        let scale_y =
            (new_rect.ymax - new_rect.ymin) as f64 / (old_rect.ymax - old_rect.ymin) as f64;

        // 2. 缩放所有顶点
        for (_, vertex) in &mut self.verts {
            // 计算相对位置
            let rel_x = (vertex.vec.x - old_rect.xmin) as f64;
            let rel_y = (vertex.vec.y - old_rect.ymin) as f64;

            // 应用缩放
            vertex.vec.x = new_rect.xmin + (rel_x * scale_x).round() as u32;
            vertex.vec.y = new_rect.ymin + (rel_y * scale_y).round() as u32;

            // 边界限制
            vertex.vec.x = vertex.vec.x.clamp(new_rect.xmin, new_rect.xmax);
            vertex.vec.y = vertex.vec.y.clamp(new_rect.ymin, new_rect.ymax);
        }

        // 3. 重新计算所有区域的几何
        self.update_all_areas_rcti(ui_setting, false, false);
    }
}
