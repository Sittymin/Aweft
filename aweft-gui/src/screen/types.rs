use bitflags::bitflags;
use slotmap::{SlotMap, new_key_type};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

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
#[derive(Debug, Clone)]
pub struct UIScaleSettings {
    pub scale_factor: f32,    // 缩放因子（从Slint获取）
    pub border_width: i16,    // 基础边框宽度（逻辑像素）
}

impl Default for UIScaleSettings {
    fn default() -> Self {
        Self {
            scale_factor: 1.0,
            border_width: 1,
        }
    }
}

impl UIScaleSettings {
    /// 从Slint窗口获取缩放设置
    pub fn from_slint_window(window_handle: &slint::Window) -> Self {
        Self {
            scale_factor: window_handle.scale_factor(),
            border_width: 1, // 默认边框宽度
        }
    }
    
    /// 计算UI_SCALE_FAC等价值（用于物理像素边距计算）
    pub fn ui_scale_fac(&self) -> f32 {
        self.scale_factor
    }
    
    /// 计算边距像素值（返回物理像素）
    pub fn calculate_margin(&self, is_edge: bool) -> u32 {
        if is_edge {
            // 窗口边缘的边距（物理像素）
            (self.ui_scale_fac() * 2.0).min(self.border_width as f32 * self.ui_scale_fac()) as u32
        } else {
            // 区域间的边距（物理像素）
            (self.border_width as f32 * self.ui_scale_fac()).max(self.ui_scale_fac()) as u32
        }
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

/// 整数定义的矩形区域
#[derive(Debug, Clone)]
pub struct Rcti {
    pub xmin: u32,
    pub xmax: u32,
    pub ymin: u32,
    pub ymax: u32,
}

impl Rcti {
    /// 从四个顶点坐标计算矩形边界（类似Blender中的实现）
    pub fn from_vertices(bl: &Vec2u32, tl: &Vec2u32, tr: &Vec2u32, br: &Vec2u32) -> Self {
        // 根据四个顶点找出最小和最大的x、y坐标
        let xmin = bl.x.min(tl.x).min(tr.x).min(br.x);
        let xmax = bl.x.max(tl.x).max(tr.x).max(br.x);
        let ymin = bl.y.min(tl.y).min(tr.y).min(br.y);
        let ymax = bl.y.max(tl.y).max(tr.y).max(br.y);
        
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

/// 定义区域分割的方向
#[derive(Debug, Clone, Copy)]
pub enum SplitDirection {
    Horizontal, // 水平分割
    Vertical,   // 垂直分割
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
    // 只保留分割操作需要的newv指针
    pub newv: Option<ScrVertKey>, // 用于分割操作的新顶点
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
    pub next: Option<ScrAreaKey>,
    pub prev: Option<ScrAreaKey>,

    // 定义区域的四个角顶点
    pub v_bottom_left: ScrVertKey,
    pub v_top_left: ScrVertKey,
    pub v_top_right: ScrVertKey,
    pub v_bottom_right: ScrVertKey,

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
    /// 创建新的ScrArea实例 - 遵循Blender的screen_addarea_ex实现
    /// 只接受顶点引用和区域类型，所有几何属性都需要后续计算
    pub fn new(
        v_bottom_left: ScrVertKey,
        v_top_left: ScrVertKey,
        v_top_right: ScrVertKey,
        v_bottom_right: ScrVertKey,
        area_type: AreaType,
    ) -> Self {
        let area_data = Self::create_area_data(area_type.clone());
        
        Self {
            next: None,
            prev: None,
            v_bottom_left,
            v_top_left,
            v_top_right,
            v_bottom_right,
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
        window_rect: &Rcti,
        ui_settings: &UIScaleSettings,
        is_fullscreen: bool,
        is_temp_window: bool,
        is_single_area: bool,
    ) {
        // 1. 从顶点计算基础矩形 - 这是几何的"真相来源"
        let bl = &vertices.get(self.v_bottom_left).unwrap().vec;
        let tl = &vertices.get(self.v_top_left).unwrap().vec;
        let tr = &vertices.get(self.v_top_right).unwrap().vec;
        let br = &vertices.get(self.v_bottom_right).unwrap().vec;

        // 直接从顶点位置设置边界（遵循Blender实现）
        self.tot_rct.xmin = bl.x;
        self.tot_rct.xmax = tr.x;
        self.tot_rct.ymin = bl.y;
        self.tot_rct.ymax = tl.y;

        // 2. 根据DPI和UI设置计算边距
        let px = ui_settings.calculate_margin(false); // 区域间边距
        let px_edge = ui_settings.calculate_margin(true); // 窗口边缘边距

        // 3. 应用边距调整（模拟Blender的逻辑）
        // 只有在非全屏、非临时窗口、且有多个区域的情况下才应用边距
        if !is_fullscreen && !is_temp_window && !is_single_area {
            // 左边距
            if self.tot_rct.xmin > window_rect.xmin {
                self.tot_rct.xmin += px;
            } else {
                self.tot_rct.xmin += px_edge;
            }

            // 右边距
            if self.tot_rct.xmax < (window_rect.xmax - 1) {
                self.tot_rct.xmax -= px;
            } else {
                self.tot_rct.xmax -= px_edge;
            }

            // 下边距
            if self.tot_rct.ymin > window_rect.ymin {
                self.tot_rct.ymin += px;
            } else {
                self.tot_rct.ymin += px_edge;
            }

            // 上边距
            if self.tot_rct.ymax < (window_rect.ymax - 1) {
                self.tot_rct.ymax -= px;
            } else if !is_single_area {
                // 顶部栏下方的小间隙
                self.tot_rct.ymax -= ui_settings.border_width as u32; // 使用border_width
            } else {
                self.tot_rct.ymax -= px_edge;
            }
        }

        // 4. 最终计算winx和winy（Blender注释："for speedup"）
        self.winx = self.tot_rct.width();
        self.winy = self.tot_rct.height();
    }
    
    /// 只读访问几何属性 - 强调这些属性不能被直接修改
    /// 
    /// 返回值说明（使用物理像素，与Blender一致）：
    /// - tot_rct: 区域的边界矩形（物理像素坐标，边距已通过scale_factor缩放）
    /// - winx: 区域宽度（物理像素大小）
    /// - winy: 区域高度（物理像素大小）
    /// 
    /// 重要：内部计算使用物理像素，传递给Slint时需要除以scale_factor转换为逻辑坐标
    pub fn geometry(&self) -> (&Rcti, u32, u32) {
        (&self.tot_rct, self.winx, self.winy)
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

/// 屏幕区域地图，管理所有的顶点、边和区域
#[derive(Debug)]
pub struct ScrAreaMap {
    pub verts: SlotMap<ScrVertKey, ScrVert>, // 存储所有顶点
    pub edges: SlotMap<ScrEdgeKey, ScrEdge>, // 存储所有边
    pub areas: SlotMap<ScrAreaKey, ScrArea>, // 存储所有区域
}

impl ScrAreaMap {
    /// 分割一个指定区域的核心函数
    pub fn area_split(
        &mut self,
        _area_key: ScrAreaKey,
        _direction: SplitDirection,
        _factor: f32, // 分割比例，取值范围 0.0-1.0
    ) -> Result<ScrAreaKey, ()> {
        // TODO: 实现区域分割逻辑
        // 1. 获取目标区域的四个顶点。
        // 2. 根据分割方向和比例计算新的顶点和边的位置。
        // 3. 调整原区域的顶点连接关系。
        // 4. 创建一个新的区域。
        // 5. 更新所有相关的几何关系和数据结构。
        unimplemented!()
    }
}
