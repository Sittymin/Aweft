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

pub enum StyleBool {
    True = -1,
    False,
}
pub enum BorderStyle {
    OutlineAndDropShadow = 1,
    OpaqueBox = 3,
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
pub struct StyleWithoutName {
    pub fontname: String,
    pub fontsize: u32,
    // 默认显示的颜色 读取和写入需要解析类似 &H00FFFFFF
    pub primary_color: u32,
    // 次要颜色
    pub secondary_color: u32,
    // WARN: 官方定义中与实际不符
    // 轮廓的颜色和透明度
    pub outline_color: u32,
    // 阴影的颜色
    pub back_color: u32,
    // 粗体
    pub bold: StyleBool,
    // 斜体
    pub italic: StyleBool,
    // 下划线
    pub underline: StyleBool,
    // 删除线
    pub strikeout: StyleBool,
    // 字体宽度伸缩率 [百分比]
    pub scale_x: u32,
    // 字体的高度伸缩率 [百分比]
    pub scale_y: u32,
    // 字符间距 [像素]
    pub spacing: u32,
    // 角度 [度]
    pub angle: f32,
    pub border_style: BorderStyle,
    // 轮廓宽度 [像素]
    pub outline: u32,
    // 阴影的深度为像素数
    pub shadow: u32,
    pub alignment: Alignment,
    // 距离边界距离
    pub margin_l: u32,
    pub margin_r: u32,
    // 对于字幕来说是距离底部的距离
    pub margin_v: u32,
    // 似乎使用 Unicode 就是 1
    pub encoding: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssTime {
    pub hours: u8,        // 0 ~ 9
    pub minutes: u8,      // 0 ~ 59
    pub seconds: u8,      // 0 ~ 59
    pub centiseconds: u8, // 0 ~ 99
}

pub struct EventMargin(pub u16);

#[derive(Debug, Clone)]
pub enum Effect {
    Karaoke,
    ScrollUp {
        y1: u16,
        y2: u16,
        delay: u16,
        fadeaway_height: Option<u16>,
    },
    ScrollDown {
        y1: u16,
        y2: u16,
        delay: u16,
        fadeaway_height: Option<u16>,
    },
    Banner {
        delay: u16,
        left_to_right: Option<bool>,
        fadeaway_width: Option<u16>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    Dialogue,
    Comment,
    Picture,
    Sound,
    Movie,
    Command,
}
pub struct StyledTextSegment {
    pub text: String,
    pub styles: HashMap<String, StyleOverride>,
}

pub enum BoldOverrideType {
    Bold,
    NonBold,
    Weight(u32),
}

enum ScaleDirection {
    Horizontal,
    Vertical,
}

pub enum RotationAxis {
    X,
    Y,
    Z,
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

pub enum StyleOverride {
    Bold(BoldOverrideType),
    Italic(bool),
    Underline(bool),
    Strikeout(bool),
    Border(i32),
    Shadow(i32),
    BlurEdges(bool),
    FontName(String),
    FontSize(u32),
    FontScale {
        direction: ScaleDirection,
        percent: i32,
    },
    FontSpacing(i32),
    FontRotation {
        axis: RotationAxis,
        degrees: f32,
    },
    FontEncoding(String),
    // 十六进制颜色 WARN:<bbggrr>
    PrimaryColor(u32),
    // 次要颜色
    SecondaryColor(u32),
    OutlineColor(u32),
    BackColor(u32),
    PrimaryAlpha(u32),
    SecondaryAlpha(u32),
    OutlineAlpha(u32),
    BackAlpha(u32),
    // 需要解析 \a 和 \an 并应用正确
    Alignment(Alignment),
    Karaoke(KaraokeOverrideType),

    // Functions
    Transformation {
        start_time: Option<u32>,
        end_time: Option<u32>,
        // 计算系数方式
        // pow((t-start_time)/(end_time-start_time), acceleration)，其中 t 为字幕的时间偏移量
        acceleration: Option<f32>,
        style_modifiers: Vec<StyleModifier>,
    },
    Move {
        start_x: f32,
        start_y: f32,
        end_x: f32,
        end_y: f32,
        start_time: Option<u32>,
        end_time: Option<u32>,
    },
    // 相当于 \move(<x>, <y>, <x>, <y>, 0, 0) 简化版本
    Position {
        x: f32,
        y: f32,
    },
    // 修改原点
    Origin {
        x: f32,
        y: f32,
    },
    Fade {
        alpha1: u8,
        alpha2: u8,
        alpha3: u8,
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
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    ClipDrawing {
        // 相当于 /p<scale> 所以直接开启绘图模式
        scale: Option<f32>,
        // TODO: 记得更改类型
        drawing_commands: Vec<DrawingCommand>,
    },

    Drawing(Drawing),
}

pub struct Event {
    // Comment 也使用 Format 的格式
    pub event_type: EventType,
    pub layer: i32,
    pub start: AssTime,
    pub end: AssTime,
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
    // TODO: 嵌入字体/图片编码
}
