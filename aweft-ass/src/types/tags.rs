use super::{drawing::DrawingCommand, BoldType, PointF64, PointI32, WrapStyle};
use aweft_libass::Alignment;
use std::time::Duration;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KaraokeKind {
    // centiseconds, ie. a duration of 100 is equivalent to 1 second
    Karaoke,
    KaraokeKf,
    KaraokeKo,
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlurEdgesTypes {
    BlurEdges,
    NonBlurEdges,
    Weight(u32),
}
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Italic(bool),
    Bold(BoldType),
    // 如果同一行有 Bord 会忽略 XBord 和 YBord
    // 如果为负数转换为 0
    // NOTE: 拆散到 X Y, 或从 XBord, YBord 绑定合并
    Bord(f64),
    XBord(f64),
    YBord(f64),
    Underline(bool),
    Strikeout(bool),
    Border(i32),
    // NOTE: 需要拆散为 XShadow YShadow , 或从 XShadow, YShadow 绑定合并
    // WARN: Shadow 需要大于 0
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
    // NOTE: 重置 ScaleX/Y 为当前行 Style 指定的缩放, 即删除前两个的定义
    FontScaleReset,
    FontSpacing(f64),
    FontRotationX(f64),
    FontRotationY(f64),
    FontRotationZ(f64),
    // 十六进制颜色 WARN:<bbggrr>
    // WARN: here not use ASSColor
    PrimaryColor(u32),
    // 次要颜色
    SecondaryColor(u32),
    OutlineColor(u32),
    BackColor(u32),
    // WARN: 只用于样式覆盖类型，需要与颜色合并
    AllAlpha(u32),
    PrimaryAlpha(u32),
    SecondaryAlpha(u32),
    OutlineAlpha(u32),
    BackAlpha(u32),
    // 需要解析 \a 和 \an 并应用正确
    FactorX(f64),
    FactorY(f64),
    Alignment(Alignment),
    Karaoke {
        duration: Duration,
        kind: KaraokeKind,
    },
    WrapStyle(WrapStyle),

    // Functions
    Transform {
        // WARN: 从字幕显示时间开始计算, 单位是毫秒
        start_time: Option<Duration>,
        end_time: Option<Duration>,
        // 计算系数方式
        // pow((t-start_time)/(end_time-start_time), acceleration)，其中 t 为字幕的时间偏移量
        acceleration: Option<f64>,
        tags: Vec<AnimatedTag>,
    },
    Move {
        start: PointF64,
        end: PointF64,
        // WARN: 从字幕显示时间开始计算, 单位是毫秒
        start_time: Option<Duration>,
        end_time: Option<Duration>,
    },
    // 相当于 \move(<x>, <y>, <x>, <y>, 0, 0) 简化版本
    Position(PointF64),
    // 修改原点
    Origin(PointF64),
    Fade {
        // WARN: 是透明度 需要转换为不透明度 (ASS 文件中是0不透明，255完全透明)
        alpha1: u8,
        alpha2: u8,
        alpha3: u8,
        // NOTE: 毫秒
        time1: Duration,
        time2: Duration,
        time3: Duration,
        time4: Duration,
    },
    SimpleFade {
        // NOTE: 毫秒
        fade_in_time: Duration,
        fade_out_time: Duration,
    },
    ClipRectangle {
        p1: PointI32,
        p2: PointI32,
        // 是否内部是需要显示的部分
        is_display: bool,
    },
    ClipDrawing {
        // 相当于 /p<scale> 所以直接开启绘图模式
        scale: u32,
        // 绘画矢量
        drawing: Vec<DrawingCommand>,
        is_display: bool,
    },

    // WARN: DrawingScale DrawingBaselineOffset DrawingCommands 三个会相互作用
    // 绘图模式缩放比例，对应 \p<scale>
    // 缩放比例被解释为二的(<缩放比例> - 1)次方
    // {\p4} 与坐标 (8,16) 的效果等同于 {\p1} 与 (1,2), (2 的 3 次方)
    DrawingScale(u32), // 当 scale 为 0 时，表示关闭绘图模式
    // 绘图基线偏移，对应 \pbo<y_offset>
    DrawingBaselineOffset(f64),

    // 参数为 None 是为重置为当前行样式，否则查找指定 Style
    Reset(Option<String>),
    Unknown,
}
// https://aegisub.org/zh-cn/docs/latest/ass_tags/
#[derive(Debug, Clone, PartialEq)]
pub enum AnimatedTag {
    // Font
    FontSize(f64),
    FontSpacing(f64),
    PrimaryColor(u32),
    SecondaryColor(u32),
    OutlineColor(u32),
    BackColor(u32),
    AllAlpha(u32),
    PrimaryAlpha(u32),
    SecondaryAlpha(u32),
    OutlineAlpha(u32),
    BackAlpha(u32),

    // Geometry
    FontScaleX(f64),
    FontScaleY(f64),
    FontRotationX(f64),
    FontRotationY(f64),
    FontRotationZ(f64),
    FactorX(f64),
    FactorY(f64),
    // Other effects
    Bord(f64),
    XBord(f64),
    YBord(f64),
    Shadow(f64),
    XShadow(f64),
    YShadow(f64),
    ClipRectangle {
        p1: PointI32,
        p2: PointI32,
        // 是否内部是需要显示的部分
        is_display: bool,
    },
    ClipDrawing {
        // 相当于 /p<scale> 所以直接开启绘图模式
        scale: u32,
        // 绘画矢量
        drawing: Vec<DrawingCommand>,
        is_display: bool,
    },
    BlurEdges(BlurEdgesTypes),
    Blur(f64),
}
