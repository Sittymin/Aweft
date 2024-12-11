use std::collections::HashMap;
use std::fmt;
use std::io::Stdout;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collisions {
    // 总是尽可能靠近垂直（底部）边距定位，如果有足够大的间隙，会填补其他字幕之间的空隙
    Normal,
    // 则字幕将向上移动，以便为后续重叠字幕腾出空间
    Reverse,
}

impl FromStr for Collisions {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Normal" => Ok(Collisions::Normal),
            "Reverse" => Ok(Collisions::Reverse),
            _ => Err(format!("Invalid value for Collisions: {}", s)),
        }
    }
}

impl fmt::Display for Collisions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Collisions::Normal => "Normal",
            Collisions::Reverse => "Reverse",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapStyle {
    // 0: 智能换行，行均匀断开
    SmartWrapping,
    // 1: 行尾单词换行，仅 \N 换行
    EndOfLineWordWrapping,
    // 2: 无自动换行，\n 和 \N 都换行
    NoWordWrapping,
    // 3: 同 0，但下行更宽
    SmartWrappingWideLower,
}

impl FromStr for WrapStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "0" => Ok(WrapStyle::SmartWrapping),
            "1" => Ok(WrapStyle::EndOfLineWordWrapping),
            "2" => Ok(WrapStyle::NoWordWrapping),
            "3" => Ok(WrapStyle::SmartWrappingWideLower),
            _ => Err(format!("Invalid value for WrapStyle: {}", s)),
        }
    }
}

impl fmt::Display for WrapStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            WrapStyle::SmartWrapping => "0",
            WrapStyle::EndOfLineWordWrapping => "1",
            WrapStyle::NoWordWrapping => "2",
            WrapStyle::SmartWrappingWideLower => "3",
        };
        write!(f, "{}", s)
    }
}

pub struct ScriptInfo {
    // 这一部分的 ; 行不需要读取，是注释
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
    // 相对于视频的开始时间
    pub synch_point: Option<String>,
    // 字幕更新者
    pub script_updated_by: Option<String>,
    // 更新描述
    pub update_details: Option<String>,
    // SSA 版本号
    pub script_type: Option<String>,
    // 空间不足时堆叠方式
    pub collisions: Option<Collisions>,
    // 字幕作者视频高度
    pub play_res_y: Option<u32>,
    // 字幕作者视频宽度
    pub play_res_x: Option<u32>,
    // 字幕作者视频颜色深度
    pub play_depth: Option<u32>,
    // 时轴速度（百分比表示，100.0000 表示精确的100%）
    pub timer: Option<f64>,
    // 换行样式
    pub wrap_style: Option<u8>,
    // 存储未知或自定义字段
    pub other_fields: std::collections::HashMap<String, String>,
}

pub enum StyleBool {
    True = -1,
    False,
}
impl FromStr for StyleBool {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "-1" => Ok(StyleBool::True),
            "0" => Ok(StyleBool::False),
            _ => Err(format!("Invalid value for StyleBool: {}", s)),
        }
    }
}
impl fmt::Display for StyleBool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            StyleBool::True => "-1",
            StyleBool::False => "0",
        };
        write!(f, "{}", s)
    }
}

pub enum BorderStyle {
    OutlineAndDropShadow = 1,
    OpaqueBox = 3,
}
impl FromStr for BorderStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1" => Ok(BorderStyle::OutlineAndDropShadow),
            "3" => Ok(BorderStyle::OpaqueBox),
            _ => Err(format!("Invalid value for BorderStyle: {}", s)),
        }
    }
}
impl fmt::Display for BorderStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BorderStyle::OutlineAndDropShadow => "1",
            BorderStyle::OpaqueBox => "3",
        };
        write!(f, "{}", s)
    }
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
impl FromStr for Alignment {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
            _ => Err(format!("Invalid value for Alignment: {}", s)),
        }
    }
}
impl fmt::Display for Alignment {
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

pub struct StyleWithoutName {
    pub fontname: String,
    pub fontsize: u32,
    // 默认显示的颜色 读取和写入需要解析类似 &H00FFFFFF
    pub primary_colour: u32,
    // 次要颜色
    pub secondary_colour: u32,
    // WARN: 官方定义中与实际不符
    // 轮廓的颜色和透明度
    pub outline_colour: u32,
    // 阴影的颜色
    pub back_colour: u32,
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
impl FromStr for AssTime {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(":").collect();
        if parts.len() != 4 {
            return Err(format!("Invalid time format: {s}"));
        }

        let hours = parts[0]
            .parse::<u8>()
            .map_err(|_| format!("Invalid hours in time: {s}"))?;
        let minutes = parts[1]
            .parse::<u8>()
            .map_err(|_| format!("Invalid minutes in time: {s}"))?;
        let seconds = parts[2]
            .parse::<u8>()
            .map_err(|_| format!("Invalid seconds in time: {s}"))?;
        let centiseconds = parts[3]
            .parse::<u8>()
            .map_err(|_| format!("Invalid centiseconds in time: {s}"))?;

        if hours > 9 || minutes > 59 || seconds > 59 || centiseconds > 99 {
            return Err(format!("Time value out of range: {s}"));
        }
        Ok(AssTime {
            hours,
            minutes,
            seconds,
            centiseconds,
        })
    }
}

pub struct EventMargin(u16);
impl FromStr for EventMargin {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim_start_matches("0");
        let value = if s.is_empty() {
            0
        } else {
            s.parse::<u16>()
                .map_err(|_| format!("Invalid value for margin: {s}"))?
        };
        Ok(EventMargin(value))
    }
}
impl fmt::Display for EventMargin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:0>4}", self.0)
    }
}

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
impl FromStr for Effect {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err("Effect string is empty.".to_string());
        }

        if s.eq("Karaoke") {
            return Ok(Effect::Karaoke);
        }

        if s.starts_with("Scroll up;") {
            // 解析 Scroll up 效果
            let params: Vec<&str> = s["Scroll up;".len()..].split(';').collect();
            if params.len() < 3 {
                return Err(format!("Invalid parameters for Scroll up effect: {}", s));
            }
            let y1 = params[0]
                .parse::<u16>()
                .map_err(|_| format!("Invalid y1 in effect: {}", s))?;
            let y2 = params[1]
                .parse::<u16>()
                .map_err(|_| format!("Invalid y2 in effect: {}", s))?;
            let delay = params[2]
                .parse::<u16>()
                .map_err(|_| format!("Invalid delay in effect: {}", s))?;
            let fadeaway_height = if params.len() > 3 {
                Some(
                    params[3]
                        .parse::<u16>()
                        .map_err(|_| format!("Invalid fadeaway_height in effect: {}", s))?,
                )
            } else {
                None
            };
            return Ok(Effect::ScrollUp {
                y1,
                y2,
                delay,
                fadeaway_height,
            });
        }

        if s.starts_with("Scroll down;") {
            // 类似处理 Scroll down
            let params: Vec<&str> = s["Scroll down;".len()..].split(';').collect();
            if params.len() < 3 {
                return Err(format!("Invalid parameters for Scroll down effect: {}", s));
            }
            let y1 = params[0]
                .parse::<u16>()
                .map_err(|_| format!("Invalid y1 in effect: {}", s))?;
            let y2 = params[1]
                .parse::<u16>()
                .map_err(|_| format!("Invalid y2 in effect: {}", s))?;
            let delay = params[2]
                .parse::<u16>()
                .map_err(|_| format!("Invalid delay in effect: {}", s))?;
            let fadeaway_height = if params.len() > 3 {
                Some(
                    params[3]
                        .parse::<u16>()
                        .map_err(|_| format!("Invalid fadeaway_height in effect: {}", s))?,
                )
            } else {
                None
            };
            return Ok(Effect::ScrollDown {
                y1,
                y2,
                delay,
                fadeaway_height,
            });
        }

        if s.starts_with("Banner;") {
            let params: Vec<&str> = s["Banner;".len()..].split(';').collect();
            if params.is_empty() {
                return Err(format!("Invalid parameters for Banner effect: {}", s));
            }
            let delay = params[0]
                .parse::<u16>()
                .map_err(|_| format!("Invalid delay in effect: {}", s))?;
            let left_to_right = if params.len() > 1 {
                Some(
                    params[1]
                        .parse::<u8>()
                        .map_err(|_| format!("Invalid lefttoright in effect: {}", s))?
                        != 0,
                )
            } else {
                None
            };
            let fadeaway_width = if params.len() > 2 {
                Some(
                    params[2]
                        .parse::<u16>()
                        .map_err(|_| format!("Invalid fadeaway_width in effect: {}", s))?,
                )
            } else {
                None
            };
            return Ok(Effect::Banner {
                delay,
                left_to_right,
                fadeaway_width,
            });
        }

        Err(format!("Invalid value for effect: {s}"))
    }
}
impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::Karaoke => write!(f, "Karaoke"),
            Effect::ScrollUp {
                y1,
                y2,
                delay,
                fadeaway_height,
            } => {
                write!(f, "Scroll up;{};{};{}", y1, y2, delay)?;
                if let Some(fh) = fadeaway_height {
                    write!(f, ";{}", fh)?;
                }
                Ok(())
            }
            Effect::ScrollDown {
                y1,
                y2,
                delay,
                fadeaway_height,
            } => {
                write!(f, "Scroll down;{};{};{}", y1, y2, delay)?;
                if let Some(fh) = fadeaway_height {
                    write!(f, ";{}", fh)?;
                }
                Ok(())
            }
            Effect::Banner {
                delay,
                left_to_right,
                fadeaway_width,
            } => {
                write!(f, "Banner;{}", delay)?;
                if let Some(ltr) = left_to_right {
                    write!(f, ";{}", if *ltr { 1 } else { 0 })?;
                }
                if let Some(fw) = fadeaway_width {
                    write!(f, ";{}", fw)?;
                }
                Ok(())
            }
        }
    }
}

pub struct Event {
    Layer: i32,
    Start: AssTime,
    End: AssTime,
    Style: String,
    Name: Option<String>,
    MarginL: EventMargin,
    MarginR: EventMargin,
    MarginV: EventMargin,
    Effect: Option<Effect>,
    Text: String,
}

pub struct ASSFile {
    pub script_info: ScriptInfo,
    pub styles: HashMap<String, StyleWithoutName>,
}
