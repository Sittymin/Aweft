use std::collections::HashMap;

pub enum WrapStyle {
    /// 默认值 (当头部信息不存在时)
    /// 溢出的行会在 ASCII 空格(U+0020)或\n处自动断行
    /// 尝试保持每行长度大致相等
    Smart = 0,

    /// 溢出的行会在 ASCII 空格或\n处在造成溢出的单词之前精确断行
    /// 不会尝试使行长度均匀
    EndWord = 1,

    /// 永不自动断行
    /// 但\N和\n都被视为强制换行
    NoWrap = 2,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScriptType {
    SSAv4,
    ASS,
    ASS2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YCbCrMatrix {
    Default,     // 头部信息缺失
    Unknown,     // 头部信息无法正确解析
    None,        // "None" 特殊值
    BT601TV,     // TV.601
    BT601PC,     // PC.601
    BT709TV,     // TV.709
    BT709PC,     // PC.709
    SMPTE240MTV, // TV.240m
    SMPTE240MPC, // PC.240m
    FCCTV,       // TV.fcc
    FCCPC,       // PC.fcc
}

pub struct ScriptInfo {
    // 这一部分的 ; 行不需要读取，是注释
    // 渲染相关
    // SSA 版本号
    pub script_type: ScriptType,
    // 缩放边框与阴影 设置为 no 没有好处, 始终为 yes
    pub scaled_border_and_shadow: bool,
    // 色彩空间
    pub ycbcr_matrix: YCbCrMatrix,
    // 字幕作者视频宽度
    pub layout_res_x: u32,
    // 字幕作者视频高度
    pub layout_res_y: u32,
    // WARN: 需要写入文件
    // NOTE: 需要和 layout 一致, 仅用于兼容
    // pub play_res_x: u32,
    // pub play_res_y: u32,
    // 换行样式
    pub wrap_style: WrapStyle,

    // 渲染无关
    // 描述
    pub title: Option<String>,
    // 原始作者
    pub original_script: Option<String>,
    // 原始翻译
    pub original_translation: Option<String>,
    // 原始校对
    pub original_editing: Option<String>,
    // 原始时间轴
    pub original_timing: Option<String>,
    // 字幕更新者
    pub script_updated_by: Option<String>,
    // 更新描述
    pub update_details: Option<String>,
    // 自定义字段 !开头字段  例如 !: Release Date: 2009-08-07
    pub other_fields: HashMap<String, String>,
}

#[derive(Debug, PartialEq)]
pub struct ASSColor(pub u32);

#[derive(Debug, PartialEq)]
pub enum BoldType {
    // Same with Weight(700)
    Bold,
    // Same with Weight(400)
    NonBold,
    Weight(u32),
}

#[derive(Debug, PartialEq)]
pub enum StyleBool {
    True = -1,
    False = 0,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    BottomLeft = 1,
    BottomCenter,
    BottomRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    TopLeft,
    TopCenter,
    TopRight,
}
#[derive(Debug, PartialEq)]
pub enum BorderStyle {
    OutlineAndDropShadow = 1,
    OpaqueBox = 3, // 矩形框
}

// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_types.h#L105-L133
#[derive(Debug, PartialEq)]
pub struct StyleWithoutName {
    pub fontname: String,
    pub fontsize: f64,
    // 默认显示的颜色 读取和写入需要解析类似 &H00FFFFFF
    pub primary_color: ASSColor,
    // 次要颜色
    pub secondary_color: ASSColor,
    // 轮廓的颜色和透明度
    pub outline_color: ASSColor,
    // 阴影的颜色
    pub back_color: ASSColor,
    // 粗体
    pub bold: BoldType,
    // 斜体
    pub italic: StyleBool,
    // 下划线
    pub underline: StyleBool,
    // 删除线
    pub strikeout: StyleBool,
    // 字体宽度伸缩率 [百分比]
    pub scale_x: f64,
    // 字体的高度伸缩率 [百分比]
    pub scale_y: f64,
    // 字符间距
    pub spacing: f64,
    // 角度 [度]
    pub angle: f64,
    pub border_style: BorderStyle,
    // 轮廓宽度 [像素]
    pub outline: f64,
    // 阴影的深度为像素数
    pub shadow: f64,
    pub alignment: Alignment,
    // 距离边界距离
    pub margin_l: i32,
    pub margin_r: i32,
    // 对于字幕来说是距离底部的距离
    pub margin_v: i32,
    // 必须是 1
    pub encoding: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ASSTime {
    pub hours: u16,       // 0 ~ 595 this limit will cancel in libass
    pub minutes: u8,      // 0 ~ 59
    pub seconds: u8,      // 0 ~ 59
    pub centiseconds: u8, // 0 ~ 99
}

#[derive(Debug, PartialEq, Eq)]
pub struct EventMargin(pub u16);

#[derive(Debug, Clone)]
pub enum Effect {
    ScrollUp {
        // y1 < y2
        y1: u16,
        y2: u16,
        delay: u16,
    },
    ScrollDown {
        y1: u16,
        y2: u16,
        delay: u16,
    },
    Banner {
        delay: u8, // [0,100]
        left_to_right: Option<bool>,
    },
    Karaoke,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    // https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/fuzz/ass.dict#L108-L110
    Dialogue,
    Comment,
}

// ASS tag is blow
pub struct StyledTextSegment {
    pub text: String,
    pub styles: HashMap<StyleOverride, String>,
}

pub enum KaraokeOverrideType {
    Duration(u32),
    FillUp(bool),
    OutlineHighlighting(bool),
}

// TODO: 比对是否内容正确
pub enum StyleModifier {
    PrimaryColor(u32),
    // 次要颜色
    SecondaryColor(u32),
    OutlineColor(u32),
    BackColor(u32),
    PrimaryAlpha(u32),
    SecondaryAlpha(u32),
    OutlineAlpha(u32),
    BackAlpha(u32),

    FontSize(u32),
    FontRotation(f32),
    FontScaleX(f32),
    FontScaleY(f32),
    FontSpacing(i32),
    Border(i32),
    Shadow(i32),
}

pub enum DrawingCommand {
    // 绘图当中如果重复一条命令，命令标识符可以省略
    // l 100 0 100 100 0 100 = l 100 0 l 100 100 l 0 100

    // 移动命令
    /// 移动光标到指定坐标（闭合形状）
    MoveTo { x: f32, y: f32 }, // 对应命令 'm'

    /// 移动光标到指定坐标（不闭合未封闭的形状）
    MoveToNoClose { x: f32, y: f32 }, // 对应命令 'n'

    // 绘制命令
    /// 画一条线到指定坐标
    LineTo { x: f32, y: f32 }, // 对应命令 'l'

    /// 绘制三次贝塞尔曲线
    BezierCurveTo {
        control_point1: (f32, f32),
        control_point2: (f32, f32),
        end_point: (f32, f32),
    }, // 对应命令 'b'

    /// 绘制三次均匀 B 样条曲线
    BSplineTo {
        points: Vec<(f32, f32)>, // 至少包含三个坐标点
    }, // 对应命令 's'

    /// 扩展 B 样条曲线到指定坐标
    ExtendBSplineTo { x: f32, y: f32 }, // 对应命令 'p'

    /// 关闭 B 样条曲线
    CloseBSpline, // 对应命令 'c'
}

pub struct Drawing {
    // 绘图模式缩放比例，对应 \p<scale>
    // 缩放比例被解释为二的(<缩放比例> - 1)次方
    // {\p4} 与坐标 (8,16) 的效果等同于 {\p1} 与 (1,2), (2 的 3 次方)
    pub scale: i32, // 当 scale 为 0 时，表示关闭绘图模式
    // 绘图基线偏移，对应 \pbo<y_offset>
    pub baseline_offset: f32,
    // 绘图命令列表
    pub commands: Vec<DrawingCommand>,
}
pub enum BlurEdgesTypes {
    BlurEdges,
    NonBlurEdges,
    Weight(u32),
}

pub enum StyleOverride {
    Italic(bool),
    Bold(BoldType),
    // 如果同一行有 Bold 会忽略 XBold 和 YBold
    // 如果为负数转换为 0
    XBold(f64),
    YBold(f64),
    Underline(bool),
    Strikeout(bool),
    Border(i32),
    Shadow(f64),
    // 这个可以负数
    XShadow(f64),
    YShadow(f64),
    BlurEdges(BlurEdgesTypes),
    Blur(f64),
    FontName(String),
    FontSize(f64),
    // \t 的情况使用 \fsc \fscx \fscy 改变字形 而不是 \fs
    FontScaleX(f64),
    FontScaleY(f64),
    FontSpacing(f64),
    FontRotationX(f64),
    FontRotationY(f64),
    FontRotationZ(f64),
    FontEncoding(String),
    // 十六进制颜色 WARN:<bbggrr>
    PrimaryColor(u32),
    // 次要颜色
    SecondaryColor(u32),
    OutlineColor(u32),
    BackColor(u32),
    // 只用于类型，需要与颜色合并
    AllAlpha(u8),
    PrimaryAlpha(u8),
    SecondaryAlpha(u8),
    OutlineAlpha(u8),
    BackAlpha(u8),
    // 需要解析 \a 和 \an 并应用正确
    FactorX(f64),
    FactorY(f64),
    Alignment(Alignment),
    Karaoke(KaraokeOverrideType),

    // Functions
    Transformation {
        start_time: Option<u32>,
        end_time: Option<u32>,
        // 计算系数方式
        // pow((t-start_time)/(end_time-start_time), acceleration)，其中 t 为字幕的时间偏移量
        acceleration: Option<f64>,
        style_modifiers: Vec<StyleModifier>,
    },
    Move {
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        start_time: Option<u32>,
        end_time: Option<u32>,
    },
    // 相当于 \move(<x>, <y>, <x>, <y>, 0, 0) 简化版本
    Position {
        x: f64,
        y: f64,
    },
    // 修改原点
    Origin {
        x: f64,
        y: f64,
    },
    Fade {
        alpha1: u32,
        alpha2: u32,
        alpha3: u32,
        time1: u32,
        time2: u32,
        time3: u32,
        time4: u32,
    },
    SimpleFade {
        fade_in_time: u32,
        fade_out_time: u32,
    },
    ClipRectangle {
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        // 是否内部是需要显示的部分
        is_display: bool,
    },
    ClipDrawing {
        // 相当于 /p<scale> 所以直接开启绘图模式
        scale: f64,
        // 绘画矢量
        drawing_commands: Vec<DrawingCommand>,
        is_display: bool,
    },

    Drawing(Drawing),
}

pub struct Event {
    // Comment 也使用 Format 的格式
    pub event_type: EventType,
    pub layer: u32,
    pub start: ASSTime,
    pub end: ASSTime,
    pub style: String,
    pub name: Option<String>,
    pub margin_l: EventMargin,
    pub margin_r: EventMargin,
    pub margin_v: EventMargin,
    pub effect: Option<Effect>,
    pub raw_text: String,
    pub parsed_text: Option<Vec<StyledTextSegment>>,
}

pub struct ASSFile {
    pub script_info: ScriptInfo,
    pub styles: HashMap<String, StyleWithoutName>,
    pub events: Vec<Event>,
    // TODO: 嵌入字体
}
