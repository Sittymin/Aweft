use super::definitions::*;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

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

impl fmt::Display for WrapStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            WrapStyle::Smart => "0",
            WrapStyle::EndWord => "1",
            WrapStyle::NoWrap => "2",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for ScriptType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "v4.00" => Ok(ScriptType::SSAv4),
            "v4.00+" => Ok(ScriptType::ASS),
            "v4.00++" => Ok(ScriptType::ASS2),
            _ => Err(format!("Invalid value for ScriptType: {}", s)),
        }
    }
}
impl fmt::Display for ScriptType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ScriptType::SSAv4 => "v4.00",
            ScriptType::ASS => "v4.00+",
            ScriptType::ASS2 => "v4.00++",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for YCbCrMatrix {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(Self::None),
            "tv.601" => Ok(Self::BT601TV),
            "pc.601" => Ok(Self::BT601PC),
            "tv.709" => Ok(Self::BT709TV),
            "pc.709" => Ok(Self::BT709PC),
            "tv.240m" => Ok(Self::SMPTE240MTV),
            "pc.240m" => Ok(Self::SMPTE240MPC),
            "tv.fcc" => Ok(Self::FCCTV),
            "pc.fcc" => Ok(Self::FCCPC),
            "" => Ok(Self::Default),
            _ => Ok(Self::Unknown),
        }
    }
}

impl ScriptInfo {
    pub fn new() -> Self {
        Self {
            // 必填部分，使用指定的默认值
            script_type: ScriptType::ASS,
            scaled_border_and_shadow: true,
            ycbcr_matrix: YCbCrMatrix::Default,
            layout_res_x: 1920,
            layout_res_y: 1080,
            wrap_style: WrapStyle::Smart,

            // 选填部分
            title: None,
            original_script: None,
            original_translation: None,
            original_editing: None,
            original_timing: None,
            script_updated_by: None,
            update_details: None,

            // 初始化其他字段
            other_fields: HashMap::new(),
        }
    }
    pub fn set_field(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "script_type" => {
                self.script_type = ScriptType::from_str(value)?;
                Ok(())
            }
            "ycbcr_matrix" => {
                self.ycbcr_matrix = YCbCrMatrix::from_str(value)?;
                Ok(())
            }
            "layout_res_x" => match value.parse::<u32>() {
                Ok(v) => {
                    self.layout_res_x = v;
                    Ok(())
                }
                Err(_) => Err(format!("Invalid PlayResX value: {}", value)),
            },
            "layout_res_y" => match value.parse::<u32>() {
                Ok(v) => {
                    self.layout_res_y = v;
                    Ok(())
                }
                Err(_) => Err(format!("Invalid PlayResY value: {}", value)),
            },
            "wrap_style" => {
                self.wrap_style = WrapStyle::from_str(value)?;
                Ok(())
            }

            // 选填部分
            "title" => {
                self.title = Some(value.to_string());
                Ok(())
            }
            "original_script" => {
                self.original_script = Some(value.to_string());
                Ok(())
            }
            "original_translation" => {
                self.original_translation = Some(value.to_string());
                Ok(())
            }
            "original_editing" => {
                self.original_editing = Some(value.to_string());
                Ok(())
            }
            "original_timing" => {
                self.original_timing = Some(value.to_string());
                Ok(())
            }
            "script_updated_by" => {
                self.script_updated_by = Some(value.to_string());
                Ok(())
            }
            "update_details" => {
                self.update_details = Some(value.to_string());
                Ok(())
            }
            _ => Err(format!("Invalid key for ScriptInfo: {}", key)),
        }
    }
    pub fn add_or_update_other_fields(&mut self, key: String, value: String) {
        self.other_fields
            .entry(key)
            .and_modify(|e| *e = value.clone())
            .or_insert(value);
    }
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

impl FromStr for EventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Dialogue" => Ok(EventType::Dialogue),
            "Comment" => Ok(EventType::Comment),
            "Picture" => Ok(EventType::Picture),
            "Sound" => Ok(EventType::Sound),
            "Movie" => Ok(EventType::Movie),
            "Command" => Ok(EventType::Command),
            _ => Err(format!("Invalid value for EventType: {}", s)),
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            EventType::Dialogue => "Dialogue",
            EventType::Comment => "Comment",
            EventType::Picture => "Picture",
            EventType::Sound => "Sound",
            EventType::Movie => "Movie",
            EventType::Command => "Command",
        };
        write!(f, "{}", s)
    }
}
