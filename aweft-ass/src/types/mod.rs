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
impl FromStr for WrapStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "0" => Ok(WrapStyle::Smart),
            "1" => Ok(WrapStyle::EndWord),
            "2" => Ok(WrapStyle::NoWrap),
            _ => Err(format!("Invalid value for WrapStyle: {}", s)),
        }
    }
}
impl TryFrom<u32> for WrapStyle {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(WrapStyle::Smart),
            1 => Ok(WrapStyle::EndWord),
            2 => Ok(WrapStyle::NoWrap),
            _ => Err(format!("Invalid value for WrapStyle: {}", value)),
        }
    }
}

impl Display for WrapStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            WrapStyle::Smart => "0",
            WrapStyle::EndWord => "1",
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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let num = s
            .parse::<i32>()
            .map_err(|_| format!("Invalid value for BoldType: {}", s))?;
        match num {
            -1 => Ok(BoldType::Bold),
            0 => Ok(BoldType::NonBold),
            n if n > 0 => Ok(BoldType::Weight(n.try_into().unwrap())),
            other => Err(format!("Invalid BoldType integer: {}", other)),
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
