use nom::branch::alt;
use nom::bytes::complete::take_until;
use nom::combinator::{map, rest};
use nom::multi::many0;
use nom::IResult;

use crate::parse_style_override::parse_multiple_overrides;

use super::definitions::*;
use super::parse_drawing_command::parse_drawing_commands;
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
            "v4.00" => {
                eprintln!("Now is only implementation ASS");
                Ok(ScriptType::SSAv4)
            }
            "v4.00+" => Ok(ScriptType::ASS),
            "v4.00++" => {
                eprintln!("Now is only implementation ASS");
                Ok(ScriptType::ASS2)
            }
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
            "script_type" | "ScriptType" => {
                self.script_type = ScriptType::from_str(value)?;
                Ok(())
            }
            "ScaledBorderAndShadow" => {
                // 不应修改
                self.scaled_border_and_shadow = true;
                Ok(())
            }
            "ycbcr_matrix" | "YCbCr Matrix" => {
                self.ycbcr_matrix = YCbCrMatrix::from_str(value)?;
                Ok(())
            }
            "layout_res_x" | "LayoutResX" | "PlayResX" => match value.parse::<u32>() {
                Ok(v) => {
                    self.layout_res_x = v;
                    Ok(())
                }
                Err(_) => Err(format!("Invalid PlayResX value: {}", value)),
            },
            "layout_res_y" | "LayoutResY" | "PlayResY" => match value.parse::<u32>() {
                Ok(v) => {
                    self.layout_res_y = v;
                    Ok(())
                }
                Err(_) => Err(format!("Invalid PlayResY value: {}", value)),
            },
            "wrap_style" | "WrapStyle" => {
                self.wrap_style = WrapStyle::from_str(value)?;
                Ok(())
            }

            // 选填部分
            "title" | "Title" => {
                self.title = Some(value.to_string());
                Ok(())
            }
            "original_script" | "Original Script" => {
                self.original_script = Some(value.to_string());
                Ok(())
            }
            "original_translation" | "Original Translation" => {
                self.original_translation = Some(value.to_string());
                Ok(())
            }
            "original_editing" | "Original Editing" => {
                self.original_editing = Some(value.to_string());
                Ok(())
            }
            "original_timing" | "Original Timing" => {
                self.original_timing = Some(value.to_string());
                Ok(())
            }
            "script_updated_by" | "Script Updated By" => {
                self.script_updated_by = Some(value.to_string());
                Ok(())
            }
            "update_details" | "Update Details" => {
                self.update_details = Some(value.to_string());
                Ok(())
            }
            _ => Err(format!("Invalid key for ScriptInfo: {}", key)),
        }
    }
    pub fn add_or_update_other_fields(&mut self, key: String, value: String) -> Result<(), String> {
        self.other_fields
            .entry(key)
            .and_modify(|e| *e = value.clone())
            .or_insert(value);
        Ok(())
    }
}

impl FromStr for ASSColor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(u32_color) = s.strip_prefix("&H") {
            u32::from_str_radix(u32_color, 16)
                .map(ASSColor)
                .map_err(|_| format!("Invalid value for ASSColor: {}", s))
        } else {
            Err(format!("Invalid format for ASSColor: {}", s))
        }
    }
}
impl fmt::Display for ASSColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "&H{:08X}", self.0)
    }
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
impl fmt::Display for BoldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoldType::Bold => write!(f, "-1"),
            BoldType::NonBold => write!(f, "0"),
            BoldType::Weight(num) => write!(f, "{}", num),
        }
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
impl StyleWithoutName {
    // https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass.c#L224-L242
    fn new() -> Self {
        StyleWithoutName {
            // NOTE: length <= 31
            fontname: "Arial".to_string(),
            // NOTE: length <= 511
            fontsize: 18.0,
            primary_color: ASSColor(0xFFFFFF00),
            secondary_color: ASSColor(0x00FFFF00),
            outline_color: ASSColor(0x00000000),
            back_color: ASSColor(0x00000080),
            bold: BoldType::Weight(200),
            italic: StyleBool::False,
            underline: StyleBool::False,
            strikeout: StyleBool::False,
            scale_x: 1.0,
            scale_y: 1.0,
            spacing: 0.0,
            angle: 0.0,
            border_style: BorderStyle::OutlineAndDropShadow,
            outline: 2.0,
            shadow: 3.0,
            alignment: Alignment::BottomCenter,
            margin_l: 20,
            margin_r: 20,
            margin_v: 20,
            encoding: 1,
        }
    }
}

impl FromStr for ASSTime {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(":").collect();
        if parts.len() != 4 {
            return Err(format!("Invalid time format: {s}"));
        }

        let hours = parts[0]
            .parse::<u16>()
            .map_err(|_| format!("Invalid hours in time: {s}"))?;
        let minutes = parts[1]
            .parse::<u8>()
            .map_err(|_| format!("Invalid minutes in time: {s}"))?;

        let sec_parts: Vec<&str> = parts[2].split(".").collect();
        if sec_parts.len() != 2 {
            return Err(format!("Invalid seconds and centiseconds in time: {s}"));
        }

        let seconds = sec_parts[0]
            .parse::<u8>()
            .map_err(|_| format!("Invalid seconds in time: {s}"))?;
        let centiseconds = sec_parts[1]
            .parse::<u8>()
            .map_err(|_| format!("Invalid centiseconds in time: {s}"))?;

        if hours > 595 || minutes > 59 || seconds > 59 || centiseconds > 99 {
            return Err(format!("Time value out of range: {s}"));
        }
        Ok(ASSTime {
            hours,
            minutes,
            seconds,
            centiseconds,
        })
    }
}

impl fmt::Display for ASSTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{:02}:{:02}.{:02}",
            self.hours, self.minutes, self.seconds, self.centiseconds
        )
    }
}
impl ASSTime {
    pub fn new() -> Self {
        Self {
            hours: 0,
            minutes: 0,
            seconds: 0,
            centiseconds: 0,
        }
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

// WARN: Review this
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
            return Ok(Effect::ScrollUp { y1, y2, delay });
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
            return Ok(Effect::ScrollDown { y1, y2, delay });
        }

        if s.starts_with("Banner;") {
            let params: Vec<&str> = s["Banner;".len()..].split(';').collect();
            if params.is_empty() {
                return Err(format!("Invalid parameters for Banner effect: {}", s));
            }
            let delay = params[0]
                .parse::<u8>()
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
            return Ok(Effect::Banner {
                delay,
                left_to_right,
            });
        }

        Err(format!("Invalid value for effect: {s}"))
    }
}
impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::Karaoke => write!(f, "Karaoke"),
            Effect::ScrollUp { y1, y2, delay } => {
                write!(f, "Scroll up;{};{};{}", y1, y2, delay)?;
                Ok(())
            }
            Effect::ScrollDown { y1, y2, delay } => {
                write!(f, "Scroll down;{};{};{}", y1, y2, delay)?;
                Ok(())
            }
            Effect::Banner {
                delay,
                left_to_right,
            } => {
                write!(f, "Banner;{}", delay)?;
                if let Some(ltr) = left_to_right {
                    write!(f, ";{}", if *ltr { 1 } else { 0 })?;
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
            _ => Err(format!("Invalid value for EventType: {}", s)),
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            EventType::Dialogue => "Dialogue",
            EventType::Comment => "Comment",
        };
        write!(f, "{}", s)
    }
}

impl TryFrom<StyleOverride> for StyleModifier {
    type Error = ();

    fn try_from(value: StyleOverride) -> Result<Self, Self::Error> {
        match value {
            // Font
            StyleOverride::FontSize(fs) => Ok(StyleModifier::FontSize(fs)),
            StyleOverride::FontSpacing(fsp) => Ok(StyleModifier::FontSpacing(fsp)),
            StyleOverride::PrimaryColor(c) => Ok(StyleModifier::PrimaryColor(c)),
            StyleOverride::SecondaryColor(c) => Ok(StyleModifier::SecondaryColor(c)),
            StyleOverride::OutlineColor(c) => Ok(StyleModifier::OutlineColor(c)),
            StyleOverride::BackColor(c) => Ok(StyleModifier::BackColor(c)),
            StyleOverride::AllAlpha(a) => Ok(StyleModifier::AllAlpha(a)),
            StyleOverride::PrimaryAlpha(a) => Ok(StyleModifier::PrimaryAlpha(a)),
            StyleOverride::SecondaryAlpha(a) => Ok(StyleModifier::SecondaryAlpha(a)),
            StyleOverride::OutlineAlpha(a) => Ok(StyleModifier::OutlineAlpha(a)),
            StyleOverride::BackAlpha(a) => Ok(StyleModifier::BackAlpha(a)),

            // Geometry
            StyleOverride::FontScaleX(fscx) => Ok(StyleModifier::FontScaleX(fscx)),
            StyleOverride::FontScaleY(fscy) => Ok(StyleModifier::FontScaleY(fscy)),
            StyleOverride::FontRotationX(frx) => Ok(StyleModifier::FontRotationX(frx)),
            StyleOverride::FontRotationY(fry) => Ok(StyleModifier::FontRotationY(fry)),
            StyleOverride::FontRotationZ(frz) => Ok(StyleModifier::FontRotationZ(frz)),
            StyleOverride::FactorX(fax) => Ok(StyleModifier::FactorX(fax)),
            StyleOverride::FactorY(fay) => Ok(StyleModifier::FactorY(fay)),

            // Other effects
            StyleOverride::Bord(bord) => Ok(StyleModifier::Bord(bord)),
            StyleOverride::XBord(xbord) => Ok(StyleModifier::XBord(xbord)),
            StyleOverride::YBord(ybord) => Ok(StyleModifier::YBord(ybord)),
            StyleOverride::Shadow(shad) => Ok(StyleModifier::Shadow(shad)),
            StyleOverride::XShadow(xshad) => Ok(StyleModifier::XShadow(xshad)),
            StyleOverride::YShadow(yshad) => Ok(StyleModifier::YShadow(yshad)),
            StyleOverride::ClipRectangle {
                x1,
                y1,
                x2,
                y2,
                is_display,
            } => Ok(StyleModifier::ClipRectangle {
                x1,
                y1,
                x2,
                y2,
                is_display,
            }),
            StyleOverride::ClipDrawing {
                scale,
                drawing_commands,
                is_display,
            } => Ok(StyleModifier::ClipDrawing {
                scale,
                drawing_commands,
                is_display,
            }),
            StyleOverride::BlurEdges(be) => Ok(StyleModifier::BlurEdges(be)),
            StyleOverride::Blur(blur) => Ok(StyleModifier::Blur(blur)),

            // 其他不支持的变体返回 Err(()), 直接过滤
            _ => Err(()),
        }
    }
}

pub struct TempStyledTextSegment {
    pub text: String,
    pub styles: HashMap<StyleOverride, String>,
}

impl PartialEq for StyleOverride {
    fn eq(&self, other: &Self) -> bool {
        // NOTE: 只比较枚举变体类型，忽略内部值
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

// 解析普通文本直到下一组 `{`
fn parse_plain_text(input: &str) -> IResult<&str, String> {
    alt((
        // 如果找到 '{'，在它之前的所有字符视为普通文本
        map(take_until("{"), |s: &str| s.to_string()),
        // 如果没有 '{' 或 take_until("{") 失败，则直接将之后的内容都当作文本
        map(rest, |s: &str| s.to_string()),
    ))(input)
}

fn parse_segment(input: &str) -> IResult<&str, StyledTextSegment> {
    // 先尝试解析样式覆盖
    if let Ok((next_input, overrides)) = parse_multiple_overrides(input) {
        // 检查是否包含非零 DrawingScale
        let has_nonzero_scale = overrides
            .iter()
            .any(|ov| matches!(ov, StyleOverride::DrawingScale(s) if *s != 0));

        if has_nonzero_scale {
            // 如果有非零的 DrawingScale，就解析绘图命令
            if let Ok((rest_input, drawing_cmds)) = parse_drawing_commands(next_input) {
                let segment = StyledTextSegment {
                    text: String::new(),
                    styles: Some(overrides),
                    drawing_commands: Some(drawing_cmds),
                };
                return Ok((rest_input, segment));
            }
        } else {
            // 否则，直接解析下一段普通文本
            if let Ok((rest_input, text)) = parse_plain_text(next_input) {
                let segment = StyledTextSegment {
                    text,
                    styles: Some(overrides),
                    drawing_commands: None,
                };
                return Ok((rest_input, segment));
            }
        }
    }
    // 不是以覆盖标签开头
    if let Ok((rest_input, text)) = parse_plain_text(input) {
        let segment = StyledTextSegment {
            text,
            styles: None,
            drawing_commands: None,
        };
        return Ok((rest_input, segment));
    }

    // 啥都没有
    Ok((
        input,
        StyledTextSegment {
            text: String::new(),
            styles: None,
            drawing_commands: None,
        },
    ))
}

impl Event {
    fn parse_from_raw_text(mut self) {
        if self.parsed_text.is_some() {
            return ();
        }
        let raw_text = self.raw_text.as_str();
        // TODO: 需要每次轮回继承前面的覆盖样式
        let (rest, parsed_val) = many0(parse_segment)(raw_text)
            .expect("Parsing error: somehow parse_segment failed even though it has a fallback.");

        self.parsed_text = Some(parsed_val);

        // 如果解析后还有剩余数据，则说明 parse_segment 没有正确消费所有输入
        if !rest.is_empty() {
            eprintln!("Warning: leftover data after parsing: {:?}", rest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_and_print_color_test() {
        let parse_color_1 = ASSColor::from_str("&HFFFFFF00").unwrap();
        let parse_color_2 = ASSColor::from_str("&Hffffff00").unwrap();
        assert_eq!("&HFFFFFF00", parse_color_1.to_string());
        assert_eq!("&HFFFFFF00", parse_color_2.to_string());
    }
}
