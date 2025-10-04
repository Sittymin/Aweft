use crate::error::{BoldTypeParseError, WrapStyleParseError};
use std::collections::HashMap;
use std::fmt::{self, Display};
use std::str::FromStr;

use event::Event;
use script::ScriptInfo;
use style::StyleWithoutName;
pub mod drawing;
pub mod event;
pub mod parse_drawing_command;
pub mod parse_style_override;
pub mod script;
pub mod style;
pub mod tags;
pub mod time;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointF64 {
    pub x: f64,
    pub y: f64,
}
impl From<(f64, f64)> for PointF64 {
    fn from(tuple: (f64, f64)) -> Self {
        PointF64 {
            x: tuple.0,
            y: tuple.1,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointI32 {
    pub x: i32,
    pub y: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapStyle {
    /// 默认值 (libass默认为0)
    /// 智能换行：溢出的行会在 ASCII 空格或\n处自动断行
    /// 会重新平衡行长度，尽可能使各行长度相等
    /// 注：libass中值3与值0行为完全相同
    Smart = 0,

    /// 在单词边界换行，但不重新平衡行长度
    /// 溢出的行会在 ASCII 空格或\n处断行，但不会调整行长度使其均匀
    EndOfWord = 1,

    /// 不自动换行
    /// 只有\N和\n会强制换行，文本可能会超出屏幕边界
    NoWrap = 2,
}
impl FromStr for WrapStyle {
    type Err = WrapStyleParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "0" => Ok(WrapStyle::Smart),
            "1" => Ok(WrapStyle::EndOfWord),
            "2" => Ok(WrapStyle::NoWrap),
            "3" => Ok(WrapStyle::Smart), // libass中值3与值0行为相同
            _ => Err(WrapStyleParseError::InvalidValue {
                value: s.to_string(),
            }),
        }
    }
}
impl TryFrom<u32> for WrapStyle {
    type Error = WrapStyleParseError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(WrapStyle::Smart),
            1 => Ok(WrapStyle::EndOfWord),
            2 => Ok(WrapStyle::NoWrap),
            3 => Ok(WrapStyle::Smart), // libass中值3与值0行为相同
            _ => Err(WrapStyleParseError::InvalidValue {
                value: value.to_string(),
            }),
        }
    }
}

impl Display for WrapStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            WrapStyle::Smart => "0",
            WrapStyle::EndOfWord => "1",
            WrapStyle::NoWrap => "2",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoldType {
    // Same with Weight(700)
    Bold,
    // Same with Weight(400)
    NonBold,
    Weight(u32),
}
impl FromStr for BoldType {
    type Err = BoldTypeParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let num = s
            .parse::<i32>()
            .map_err(|_| BoldTypeParseError::ParseIntError {
                input: s.to_string(),
            })?;
        match num {
            -1 => Ok(BoldType::Bold),
            0 => Ok(BoldType::NonBold),
            n if n > 0 => Ok(BoldType::Weight(n.try_into().unwrap())),
            other => Err(BoldTypeParseError::InvalidWeight { value: other }),
        }
    }
}
impl Display for BoldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoldType::Bold => write!(f, "-1"),
            BoldType::NonBold => write!(f, "0"),
            BoldType::Weight(num) => write!(f, "{}", num),
        }
    }
}

// 字段分块
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    ScriptInfo,
    V4PStyles,
    Events,
}
#[derive(Debug)]
pub struct ASSFile {
    pub script_info: ScriptInfo,
    pub styles: HashMap<String, StyleWithoutName>,
    pub events: Vec<Event>,
    // TODO: 嵌入字体
}
