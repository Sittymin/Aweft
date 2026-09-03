use crate::error::{AlignmentParseError, Result};
use crate::ffi;
use bitflags::bitflags;
use std::ffi::CString;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(transparent)]
    pub struct LibassAlignment: i32 {
        const VALIGN_TOP    = 4;
        const VALIGN_CENTER = 8;
        const VALIGN_SUB    = 0;
        const HALIGN_LEFT   = 1;
        const HALIGN_CENTER = 2;
        const HALIGN_RIGHT  = 3;
    }
}

// 定义一个只包含有效组合的枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    BottomLeft,
    BottomCenter,
    BottomRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    TopLeft,
    TopCenter,
    TopRight,
}

impl From<Alignment> for LibassAlignment {
    fn from(align: Alignment) -> Self {
        match align {
            Alignment::BottomLeft => Self::from_bits_retain(0 | 1),
            Alignment::BottomCenter => Self::from_bits_retain(0 | 2),
            Alignment::BottomRight => Self::from_bits_retain(0 | 3),
            Alignment::TopLeft => Self::from_bits_retain(4 | 1),
            Alignment::TopCenter => Self::from_bits_retain(4 | 2),
            Alignment::TopRight => Self::from_bits_retain(4 | 3),
            Alignment::MiddleLeft => Self::from_bits_retain(8 | 1),
            Alignment::MiddleCenter => Self::from_bits_retain(8 | 2),
            Alignment::MiddleRight => Self::from_bits_retain(8 | 3),
        }
    }
}

impl TryFrom<LibassAlignment> for Alignment {
    type Error = AlignmentParseError;

    fn try_from(value: LibassAlignment) -> std::result::Result<Self, Self::Error> {
        // 使用 `bits()` 进行匹配
        match value.bits() {
            1 => Ok(Alignment::BottomLeft),
            2 => Ok(Alignment::BottomCenter),
            3 => Ok(Alignment::BottomRight),
            5 => Ok(Alignment::TopLeft),
            6 => Ok(Alignment::TopCenter),
            7 => Ok(Alignment::TopRight),
            9 => Ok(Alignment::MiddleLeft),
            10 => Ok(Alignment::MiddleCenter),
            11 => Ok(Alignment::MiddleRight),
            _ => Err(AlignmentParseError::InvalidLibassBits { bits: value.bits() }),
        }
    }
}

// INFO: ASS 规范当中是使用小键盘布局
impl FromStr for Alignment {
    type Err = AlignmentParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "1" => Ok(Alignment::BottomLeft),
            "2" => Ok(Alignment::BottomCenter),
            "3" => Ok(Alignment::BottomRight),
            "4" => Ok(Alignment::MiddleLeft),
            "5" => Ok(Alignment::MiddleCenter),
            "6" => Ok(Alignment::MiddleRight),
            "7" => Ok(Alignment::TopLeft),
            "8" => Ok(Alignment::TopCenter),
            "9" => Ok(Alignment::TopRight),
            _ => Err(AlignmentParseError::InvalidValue {
                value: s.to_string(),
            }),
        }
    }
}
impl Display for Alignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Alignment::BottomLeft => "1",
            Alignment::BottomCenter => "2",
            Alignment::BottomRight => "3",
            Alignment::MiddleLeft => "4",
            Alignment::MiddleCenter => "5",
            Alignment::MiddleRight => "6",
            Alignment::TopLeft => "7",
            Alignment::TopCenter => "8",
            Alignment::TopRight => "9",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Banner,
    ScrollUp,
    ScrollDown,
}

// 2. 实现转换逻辑
impl Effect {
    pub fn as_str(&self) -> &'static str {
        match self {
            Effect::Banner => "Banner;",
            Effect::ScrollUp => "Scroll up;",
            Effect::ScrollDown => "Scroll down;",
        }
    }

    pub fn to_cstring(&self) -> Result<CString> {
        Ok(CString::new(self.as_str())?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ShapingLevel {
    Simple = 0,
    Complex = 1,
}
impl TryFrom<u32> for ShapingLevel {
    type Error = ();

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(ShapingLevel::Simple),
            1 => Ok(ShapingLevel::Complex),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HintingType {
    None = 0,
    Light = 1,
    Normal = 2,
    Native = 3,
}
impl TryFrom<u32> for HintingType {
    type Error = ();

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(HintingType::None),
            1 => Ok(HintingType::Light),
            2 => Ok(HintingType::Normal),
            3 => Ok(HintingType::Native),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DefaultFontProvider {
    None = 0,
    AutoDetect = 1,
    CoreText,
    FontConfig,
    DirectWrite,
}
impl From<u32> for DefaultFontProvider {
    fn from(provider: u32) -> Self {
        match provider {
            0 => DefaultFontProvider::None,
            1 => DefaultFontProvider::AutoDetect,
            2 => DefaultFontProvider::CoreText,
            3 => DefaultFontProvider::FontConfig,
            4 => DefaultFontProvider::DirectWrite,
            _ => DefaultFontProvider::None,
        }
    }
}

/// Optional libass track features. Configure before the first render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFeature {
    IncompatibleExtensions,
    BidiBrackets,
    WholeTextLayout,
    WrapUnicode,
}

impl TrackFeature {
    pub(crate) fn to_raw(self) -> ffi::ASS_Feature {
        // bindgen emits ASS_Feature as its underlying integer type; enum values
        // are part of libass's stable ABI and start at zero.
        match self {
            Self::IncompatibleExtensions => 0,
            Self::BidiBrackets => 1,
            Self::WholeTextLayout => 2,
            Self::WrapUnicode => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TrackType {
    Unknown = 0,
    Ass = 1,
    Ssa = 2,
}

/// Style border rendering mode (ASS `BorderStyle` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BorderStyle {
    /// Outline + drop shadow; the default in most scripts.
    Outline = 1,
    /// Opaque box behind the text.
    OpaqueBox = 3,
}

impl BorderStyle {
    pub(crate) fn to_raw(self) -> i32 {
        self as i32
    }
}

impl TryFrom<i32> for BorderStyle {
    type Error = ();

    fn try_from(value: i32) -> std::result::Result<Self, Self::Error> {
        match value {
            1 => Ok(BorderStyle::Outline),
            3 => Ok(BorderStyle::OpaqueBox),
            _ => Err(()),
        }
    }
}

/// Text justification independent of event alignment (ASS `Justify` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Justify {
    Auto = 0,
    Left = 1,
    Center = 2,
    Right = 3,
}

impl Justify {
    pub(crate) fn to_raw(self) -> i32 {
        self as i32
    }
}

impl TryFrom<i32> for Justify {
    type Error = ();

    fn try_from(value: i32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(Justify::Auto),
            1 => Ok(Justify::Left),
            2 => Ok(Justify::Center),
            3 => Ok(Justify::Right),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderChange {
    None,
    Position,
    Content,
    Unknown(i32),
}

impl RenderChange {
    pub(crate) fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Position,
            2 => Self::Content,
            n => Self::Unknown(n),
        }
    }
    pub fn is_changed(self) -> bool {
        !matches!(self, Self::None)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(C)]
    pub struct OverrideBits: u32 {
        /// 无选择性覆盖，仅保留传统 font_scale。
        const DEFAULT = 0;
        /// 对话事件：应用 selective style（= FONT_NAME|FONT_SIZE_FIELDS|COLORS|BORDER|ATTRIBUTES）。
        const STYLE = 1 << 0;
        /// font_scale 仅作用于对话事件。
        const SELECTIVE_FONT_SCALE = 1 << 1;
        /// 覆盖 FontSize/Spacing/ScaleX/ScaleY。
        const FONT_SIZE_FIELDS = 1 << 2;
        /// 覆盖 FontName。
        const FONT_NAME = 1 << 3;
        /// 覆盖 Primary/Secondary/Outline/BackColour。
        const COLORS = 1 << 4;
        /// 覆盖 Bold/Italic/Underline/StrikeOut。
        const ATTRIBUTES = 1 << 5;
        /// 覆盖 BorderStyle/Outline/Shadow。
        const BORDER = 1 << 6;
        /// 覆盖 Alignment。
        const ALIGNMENT = 1 << 7;
        /// 覆盖 MarginL/MarginR/MarginV。
        const MARGINS = 1 << 8;
        /// 无条件替换全部样式字段。
        const FULL_STYLE = 1 << 9;
        /// 覆盖 Justify。
        const JUSTIFY = 1 << 10;
        /// 覆盖 Blur。
        const BLUR = 1 << 11;
    }
}
