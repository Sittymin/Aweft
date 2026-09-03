use aweft_libass::color::AssColorRgb;
use aweft_libass::types::Alignment;

use super::{drawing::DrawingCommand, BoldType, PointF64, PointI32, WrapStyle};

use chrono::TimeDelta;
use std::fmt::{self, Display};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KaraokeKind {
    // centiseconds, ie. a duration of 100 is equivalent to 1 second
    Karaoke,
    KaraokeKf,
    KaraokeKo,
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlurEdgesTypes {
    NonBlurEdges,
    Weight(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibassEncoding {
    /// 日语 Shift_JIS (CP932)
    ShiftJis,
    /// 简体中文 GBK (CP936)
    Gbk,
    /// 韩语 EUC-KR (CP949)
    Wansung, // FreeType 称之为 Wansung
    /// 繁体中文 Big5 (CP950)
    Big5,
    /// 韩语 Johab (CP1361)
    Johab,
    /// 自动检测
    Auto,
}

impl LibassEncoding {
    pub fn from_codepage(codepage: i32) -> Option<Self> {
        match codepage {
            932 => Some(Self::ShiftJis),
            936 => Some(Self::Gbk),
            949 => Some(Self::Wansung),
            950 => Some(Self::Big5),
            1361 => Some(Self::Johab),
            -1 => Some(Self::Auto),
            _ => None, // 对于任何其他值，返回 None
        }
    }

    /// 将 LibassEncoding 枚举转换为它对应的整数代码页。
    pub fn to_codepage(&self) -> i32 {
        match self {
            Self::ShiftJis => 932,
            Self::Gbk => 936,
            Self::Wansung => 949,
            Self::Big5 => 950,
            Self::Johab => 1361,
            Self::Auto => -1,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum StyleTag {
    Italic(bool),
    // WARN: 文件当中是 -1 开启
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
    FontSizeReset,
    // \t 的情况使用 \fsc \fscx \fscy 改变字形 而不是 \fs
    FontScaleX(f64),
    FontScaleY(f64),
    // NOTE: 重置 ScaleX/Y 为当前行 Style 指定的缩放, 即删除前两个的定义
    FontScaleReset,
    FontSpacing(f64),
    FontRotationX(f64),
    FontRotationY(f64),
    // \fr 是 \frz 的简写
    FontRotationZ(f64),
    // 十六进制颜色 WARN:<BBGGRR>
    // WARN: here not use ASSColor
    PrimaryColor(AssColorRgb),
    // 次要颜色
    SecondaryColor(AssColorRgb),
    OutlineColor(AssColorRgb),
    BackColor(AssColorRgb),
    // WARN: 只用于样式覆盖类型，需要与颜色合并
    // 十六进制颜色 WARN:<AA>
    AllAlpha(u8),
    PrimaryAlpha(u8),
    SecondaryAlpha(u8),
    OutlineAlpha(u8),
    BackAlpha(u8),
    // 需要解析 \a 和 \an 并应用正确
    FactorX(f64),
    FactorY(f64),
    Alignment(Alignment),
    Karaoke {
        kind: KaraokeKind,
        // WARN: !!! 这里在 ASS 文件当中是 厘秒
        duration: TimeDelta,
    },
    WrapStyle(WrapStyle),

    // Functions
    Transform {
        // WARN: 从字幕显示时间开始计算, 单位是毫秒，可以为负
        start_time: Option<TimeDelta>,
        end_time: Option<TimeDelta>,
        // 计算系数方式
        // pow((t-start_time)/(end_time-start_time), acceleration)，其中 t 为字幕的时间偏移量
        acceleration: Option<f64>,
        tags: Vec<AnimatedStyleTag>,
    },
    Move {
        start: PointF64,
        end: PointF64,
        // WARN: 从字幕显示时间开始计算, 单位是毫秒
        start_time: Option<TimeDelta>,
        end_time: Option<TimeDelta>,
    },
    // 相当于 \move(<x>, <y>, <x>, <y>, 0, 0) 简化版本
    Position(PointF64),
    // 修改原点
    Origin(PointF64),
    Fade {
        // WARN: 是透明度 需要转换为不透明度 (ASS 文件中是0不透明，255完全透明)
        start_opacity: u8,
        mid_opacity: u8,
        end_opacity: u8,
        // NOTE: 毫秒
        in_start_time: TimeDelta,
        in_end_time: TimeDelta,
        out_start_time: TimeDelta,
        out_end_time: TimeDelta,
    },
    SimpleFade {
        // NOTE: 毫秒
        in_time: TimeDelta,
        out_time: TimeDelta,
    },
    ClipRectangle {
        p1: PointI32,
        p2: PointI32,
        // 是否内部是需要显示的部分
        is_display: bool,
    },
    ClipDrawing {
        // 相当于 /p<scale> 所以直接开启绘图模式
        scale: i32,
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
    Encoding(LibassEncoding),
    Unknown,
    // HACK: 行内对齐方式(无法在 ass 文件中使用，仅可在 libass 当中调用函数使用)
    Justify(i8),
}

impl Display for StyleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StyleTag::Shadow(depth) => write!(f, "\\shad{}", depth),
            StyleTag::XShadow(val) => write!(f, "\\xshad{}", val),
            StyleTag::YShadow(val) => write!(f, "\\yshad{}", val),
            StyleTag::FactorX(val) => write!(f, "\\fax{}", val),
            StyleTag::FactorY(val) => write!(f, "\\fay{}", val),
            StyleTag::Blur(val) => write!(f, "\\blur{}", val),
            StyleTag::FontScaleX(val) => write!(f, "\\fscx{}", val),
            StyleTag::FontScaleY(val) => write!(f, "\\fscy{}", val),
            StyleTag::FontScaleReset => write!(f, "\\fsc"),
            StyleTag::FontSpacing(val) => write!(f, "\\fsp{}", val),
            StyleTag::FontSize(val) => write!(f, "\\fs{}", val),
            StyleTag::FontSizeReset => Ok(()),
            StyleTag::Bord(val) => write!(f, "\\bord{}", val),
            StyleTag::XBord(val) => write!(f, "\\xbord{}", val),
            StyleTag::YBord(val) => write!(f, "\\ybord{}", val),
            StyleTag::Move {
                start: p1,
                end: p2,
                start_time: t1,
                end_time: t2,
            } => match (t1, t2) {
                (Some(start), Some(end)) => {
                    write!(
                        f,
                        "\\move({},{},{},{},{},{})",
                        p1.x,
                        p1.y,
                        p2.x,
                        p2.y,
                        start.num_milliseconds(),
                        end.num_milliseconds()
                    )
                }
                _ => write!(f, "\\move({},{},{},{})", p1.x, p1.y, p2.x, p2.y),
            },
            StyleTag::FontRotationX(val) => write!(f, "\\frx{}", val),
            StyleTag::FontRotationY(val) => write!(f, "\\fry{}", val),
            StyleTag::FontRotationZ(val) => write!(f, "\\frz{}", val),
            StyleTag::FontName(font_name) => write!(f, "\\fn{}", font_name),
            StyleTag::AllAlpha(alpha) => write!(f, "\\alpha&H{:02X}&", alpha),
            StyleTag::PrimaryAlpha(alpha) => write!(f, "\\1a&H{:02X}&", alpha),

            StyleTag::SecondaryAlpha(alpha) => write!(f, "\\2a&H{:02X}&", alpha),
            StyleTag::OutlineAlpha(alpha) => write!(f, "\\3a&H{:02X}&", alpha),
            StyleTag::BackAlpha(alpha) => write!(f, "\\4a&H{:02X}&", alpha),
            StyleTag::Alignment(val) => write!(f, "\\an{}", val),
            StyleTag::Position(p) => write!(f, "\\pos({},{})", p.x, p.y),
            StyleTag::Fade {
                start_opacity: a1,
                mid_opacity: a2,
                end_opacity: a3,
                in_start_time: t1,
                in_end_time: t2,
                out_start_time: t3,
                out_end_time: t4,
            } => write!(
                f,
                "\\fade({},{},{},{},{},{},{})",
                a1,
                a2,
                a3,
                t1.num_milliseconds(),
                t2.num_milliseconds(),
                t3.num_milliseconds(),
                t4.num_milliseconds()
            ),
            StyleTag::Origin(p) => write!(f, "\\org({},{})", p.x, p.y),
            StyleTag::Transform {
                start_time: t1,
                end_time: t2,
                acceleration: accel,
                tags,
            } => match (t1, t2, accel) {
                (Some(t1), Some(t2), Some(accel)) => {
                    write!(
                        f,
                        "\\t({},{},{},",
                        t1.num_milliseconds(),
                        t2.num_milliseconds(),
                        accel
                    )?;
                    for animation_tag in tags {
                        write!(f, "{}", animation_tag)?;
                    }
                    write!(f, ")")
                }
                (Some(t1), Some(t2), None) => {
                    write!(
                        f,
                        "\\t({},{},",
                        t1.num_milliseconds(),
                        t2.num_milliseconds()
                    )?;
                    for animation_tag in tags {
                        write!(f, "{}", animation_tag)?;
                    }
                    write!(f, ")")
                }
                (None, None, Some(accel)) => {
                    write!(f, "\\t({},", accel)?;
                    for animation_tag in tags {
                        write!(f, "{}", animation_tag)?;
                    }
                    write!(f, ")")
                }
                (None, None, None) => {
                    write!(f, "\\t(")?;
                    for animation_tag in tags {
                        write!(f, "{}", animation_tag)?;
                    }
                    write!(f, ")")
                }
                _ => unreachable!(
                    "Invalid transformation combination: t1={:?}, t2={:?}, accel={:?}",
                    t1, t2, accel
                ),
            },
            StyleTag::ClipRectangle { p1, p2, is_display } => {
                write!(f, "\\")?;
                if !is_display {
                    write!(f, "i")?;
                }
                write!(f, "clip({},{},{},{})", p1.x, p1.y, p2.x, p2.y)
            }
            StyleTag::ClipDrawing {
                scale,
                drawing,
                is_display,
            } => {
                write!(f, "\\")?;
                if !is_display {
                    write!(f, "i")?;
                }
                write!(f, "clip({},", scale)?;
                for (index, drawing_command) in drawing.iter().enumerate() {
                    if index > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", drawing_command)?;
                }
                write!(f, ")")
            }
            StyleTag::PrimaryColor(color) => write!(f, "\\c&H{:06X}&", color.to_ass_override_u32()),
            StyleTag::SecondaryColor(color) => {
                write!(f, "\\2c&H{:06X}&", color.to_ass_override_u32())
            }
            StyleTag::OutlineColor(color) => {
                write!(f, "\\3c&H{:06X}&", color.to_ass_override_u32())
            }
            StyleTag::BackColor(color) => write!(f, "\\4c&H{:06X}&", color.to_ass_override_u32()),
            StyleTag::Encoding(c) => write!(f, "\\fe{}", c.to_codepage()),
            StyleTag::Reset(None) => write!(f, "\\r"),
            StyleTag::Reset(Some(style_name)) => write!(f, "\\r{}", style_name),
            StyleTag::BlurEdges(be) => match be {
                BlurEdgesTypes::NonBlurEdges => write!(f, "\\be0"),
                BlurEdgesTypes::Weight(val) => write!(f, "\\be{}", val),
            },
            StyleTag::Bold(b) => match b {
                BoldType::NonBold => write!(f, "\\b0"),
                BoldType::Bold => write!(f, "\\b1"),
                BoldType::Weight(val) => write!(f, "\\b{}", val),
            },
            StyleTag::Italic(val) => write!(f, "\\i{}", if *val { 1 } else { 0 }),
            StyleTag::Karaoke { kind, duration } => match kind {
                KaraokeKind::KaraokeKf => write!(f, "\\kf{}", duration.num_milliseconds()),
                KaraokeKind::KaraokeKo => write!(f, "\\ko{}", duration.num_milliseconds()),
                KaraokeKind::Karaoke => write!(f, "\\k{}", duration.num_milliseconds()),
            },
            StyleTag::Strikeout(is_enable) => {
                write!(f, "\\s{}", if *is_enable { 1 } else { 0 })
            }
            StyleTag::Underline(is_enable) => {
                write!(f, "\\u{}", if *is_enable { 1 } else { 0 })
            }
            StyleTag::DrawingBaselineOffset(val) => write!(f, "\\pbo{}", val),
            StyleTag::DrawingScale(scale) => write!(f, "\\p{}", scale),
            StyleTag::WrapStyle(wrap_style) => write!(f, "\\q{}", wrap_style),
            _ => Ok(()),
        }
    }
}

// https://aegisub.org/zh-cn/docs/latest/ass_tags/
#[derive(Debug, Clone, PartialEq)]
pub enum AnimatedStyleTag {
    // Font
    FontSize(f64),
    FontSpacing(f64),
    PrimaryColor(AssColorRgb),
    SecondaryColor(AssColorRgb),
    OutlineColor(AssColorRgb),
    BackColor(AssColorRgb),
    AllAlpha(u8),
    PrimaryAlpha(u8),
    SecondaryAlpha(u8),
    OutlineAlpha(u8),
    BackAlpha(u8),

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
        scale: i32,
        // 绘画矢量
        drawing: Vec<DrawingCommand>,
        is_display: bool,
    },
    BlurEdges(BlurEdgesTypes),
    Blur(f64),
}

impl Display for AnimatedStyleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Font
            AnimatedStyleTag::FontSize(val) => write!(f, "\\fs{}", val),
            AnimatedStyleTag::FontSpacing(val) => write!(f, "\\fsp{}", val),
            AnimatedStyleTag::PrimaryColor(color) => {
                write!(f, "\\c&H{:06X}&", color.to_ass_override_u32())
            }
            AnimatedStyleTag::SecondaryColor(color) => {
                write!(f, "\\2c&H{:06X}&", color.to_ass_override_u32())
            }
            AnimatedStyleTag::OutlineColor(color) => {
                write!(f, "\\3c&H{:06X}&", color.to_ass_override_u32())
            }
            AnimatedStyleTag::BackColor(color) => {
                write!(f, "\\4c&H{:06X}&", color.to_ass_override_u32())
            }
            AnimatedStyleTag::AllAlpha(alpha) => write!(f, "\\alpha&H{:02X}&", alpha),
            AnimatedStyleTag::PrimaryAlpha(alpha) => write!(f, "\\1a&H{:02X}&", alpha),

            AnimatedStyleTag::SecondaryAlpha(alpha) => write!(f, "\\2a&H{:02X}&", alpha),
            AnimatedStyleTag::OutlineAlpha(alpha) => write!(f, "\\3a&H{:02X}&", alpha),
            AnimatedStyleTag::BackAlpha(alpha) => write!(f, "\\4a&H{:02X}&", alpha),
            // Geometry
            AnimatedStyleTag::FontScaleX(val) => write!(f, "\\fscx{}", val),
            AnimatedStyleTag::FontScaleY(val) => write!(f, "\\fscy{}", val),
            AnimatedStyleTag::FontRotationX(val) => write!(f, "\\frx{}", val),
            AnimatedStyleTag::FontRotationY(val) => write!(f, "\\fry{}", val),
            AnimatedStyleTag::FontRotationZ(val) => write!(f, "\\frz{}", val),
            AnimatedStyleTag::FactorX(val) => write!(f, "\\fax{}", val),
            AnimatedStyleTag::FactorY(val) => write!(f, "\\fay{}", val),
            // Other effects
            AnimatedStyleTag::Bord(val) => write!(f, "\\bord{}", val),
            AnimatedStyleTag::XBord(val) => write!(f, "\\xbord{}", val),
            AnimatedStyleTag::YBord(val) => write!(f, "\\ybord{}", val),
            AnimatedStyleTag::Shadow(depth) => write!(f, "\\shad{}", depth),
            AnimatedStyleTag::XShadow(val) => write!(f, "\\xshad{}", val),
            AnimatedStyleTag::YShadow(val) => write!(f, "\\yshad{}", val),
            AnimatedStyleTag::ClipRectangle { p1, p2, is_display } => {
                write!(f, "\\")?;
                if !is_display {
                    write!(f, "i")?;
                }
                write!(f, "clip({},{},{},{})", p1.x, p1.y, p2.x, p2.y)
            }
            AnimatedStyleTag::ClipDrawing {
                scale,
                drawing,
                is_display,
            } => {
                write!(f, "\\")?;
                if !is_display {
                    write!(f, "i")?;
                }
                write!(f, "clip({},", scale)?;
                for (index, drawing_command) in drawing.iter().enumerate() {
                    if index > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", drawing_command)?;
                }
                write!(f, ")")
            }
            AnimatedStyleTag::BlurEdges(be) => match be {
                BlurEdgesTypes::NonBlurEdges => write!(f, "\\be0"),
                BlurEdgesTypes::Weight(val) => write!(f, "\\be{}", val),
            },
            AnimatedStyleTag::Blur(val) => write!(f, "\\blur{}", val),
        }
    }
}

impl TryFrom<StyleTag> for AnimatedStyleTag {
    type Error = String;

    fn try_from(tag: StyleTag) -> Result<Self, Self::Error> {
        match tag {
            // Font
            StyleTag::FontSize(v) => Ok(AnimatedStyleTag::FontSize(v)),
            StyleTag::FontSpacing(v) => Ok(AnimatedStyleTag::FontSpacing(v)),
            StyleTag::PrimaryColor(v) => Ok(AnimatedStyleTag::PrimaryColor(v)),
            StyleTag::SecondaryColor(v) => Ok(AnimatedStyleTag::SecondaryColor(v)),
            StyleTag::OutlineColor(v) => Ok(AnimatedStyleTag::OutlineColor(v)),
            StyleTag::BackColor(v) => Ok(AnimatedStyleTag::BackColor(v)),
            StyleTag::AllAlpha(v) => Ok(AnimatedStyleTag::AllAlpha(v)),
            StyleTag::PrimaryAlpha(v) => Ok(AnimatedStyleTag::PrimaryAlpha(v)),
            StyleTag::SecondaryAlpha(v) => Ok(AnimatedStyleTag::SecondaryAlpha(v)),
            StyleTag::OutlineAlpha(v) => Ok(AnimatedStyleTag::OutlineAlpha(v)),
            StyleTag::BackAlpha(v) => Ok(AnimatedStyleTag::BackAlpha(v)),
            // Geometry
            StyleTag::FontScaleX(v) => Ok(AnimatedStyleTag::FontScaleX(v)),
            StyleTag::FontScaleY(v) => Ok(AnimatedStyleTag::FontScaleY(v)),
            StyleTag::FontRotationX(v) => Ok(AnimatedStyleTag::FontRotationX(v)),
            StyleTag::FontRotationY(v) => Ok(AnimatedStyleTag::FontRotationY(v)),
            StyleTag::FontRotationZ(v) => Ok(AnimatedStyleTag::FontRotationZ(v)),
            StyleTag::FactorX(v) => Ok(AnimatedStyleTag::FactorX(v)),
            StyleTag::FactorY(v) => Ok(AnimatedStyleTag::FactorY(v)),
            // Other effects
            StyleTag::Bord(v) => Ok(AnimatedStyleTag::Bord(v)),
            StyleTag::XBord(v) => Ok(AnimatedStyleTag::XBord(v)),
            StyleTag::YBord(v) => Ok(AnimatedStyleTag::YBord(v)),
            StyleTag::Shadow(v) => Ok(AnimatedStyleTag::Shadow(v)),
            StyleTag::XShadow(v) => Ok(AnimatedStyleTag::XShadow(v)),
            StyleTag::YShadow(v) => Ok(AnimatedStyleTag::YShadow(v)),
            StyleTag::ClipRectangle { p1, p2, is_display } => {
                Ok(AnimatedStyleTag::ClipRectangle { p1, p2, is_display })
            }
            StyleTag::ClipDrawing {
                scale,
                drawing,
                is_display,
            } => Ok(AnimatedStyleTag::ClipDrawing {
                scale,
                drawing,
                is_display,
            }),
            StyleTag::BlurEdges(v) => Ok(AnimatedStyleTag::BlurEdges(v)),
            StyleTag::Blur(v) => Ok(AnimatedStyleTag::Blur(v)),

            // 所有其他无法转换的 StyleTag 变体，都返回错误
            _ => Err(String::from("不可动画化的标签")),
        }
    }
}
