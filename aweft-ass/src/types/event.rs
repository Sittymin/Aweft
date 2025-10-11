use crate::error::{EffectParseError, EventFieldParseError, ParseError, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{self, char},
    combinator::{all_consuming, map, opt},
    multi::many0,
    sequence::preceded,
    IResult, Parser,
};

use crate::types::drawing::DrawingCommand;
use rayon::iter::{FromParallelIterator, IntoParallelIterator};
use std::{
    borrow::{Borrow, BorrowMut},
    collections::HashMap,
    fmt,
    ops::DerefMut,
};
use std::{error::Error, fmt::Display};
use std::{ops::Deref, str::FromStr};

use super::{parse_style_override::parse_multiple_overrides, tags::StyleTag, time::AssTime};

#[derive(Debug, PartialEq, Clone, Default)]
pub struct Events {
    vec: Vec<Event>,
}

impl Deref for Events {
    type Target = Vec<Event>;
    fn deref(&self) -> &Self::Target {
        &self.vec
    }
}

impl DerefMut for Events {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.vec
    }
}

impl AsMut<Vec<Event>> for Events {
    fn as_mut(&mut self) -> &mut Vec<Event> {
        &mut self.vec
    }
}
impl AsRef<Vec<Event>> for Events {
    fn as_ref(&self) -> &Vec<Event> {
        &self.vec
    }
}
impl Borrow<Vec<Event>> for Events {
    fn borrow(&self) -> &Vec<Event> {
        &self.vec
    }
}

impl BorrowMut<Vec<Event>> for Events {
    fn borrow_mut(&mut self) -> &mut Vec<Event> {
        &mut self.vec
    }
}
impl IntoIterator for Events {
    type Item = Event;
    type IntoIter = std::vec::IntoIter<Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.vec.into_iter()
    }
}
impl From<Vec<Event>> for Events {
    fn from(vec: Vec<Event>) -> Self {
        Events { vec }
    }
}
impl FromIterator<Event> for Events {
    fn from_iter<T: IntoIterator<Item = Event>>(iter: T) -> Self {
        Events {
            vec: Vec::from_iter(iter),
        }
    }
}

impl FromParallelIterator<Event> for Events {
    fn from_par_iter<I>(par_iter: I) -> Self
    where
        I: IntoParallelIterator<Item = Event>,
    {
        Events {
            vec: Vec::from_par_iter(par_iter),
        }
    }
}

impl<'a> IntoIterator for &'a Events {
    type Item = &'a Event;
    type IntoIter = std::slice::Iter<'a, Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.vec.iter()
    }
}

impl<'a> IntoIterator for &'a mut Events {
    type Item = &'a mut Event;
    type IntoIter = std::slice::IterMut<'a, Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.vec.iter_mut()
    }
}

impl Events {
    pub fn new() -> Self {
        Self { vec: Vec::new() }
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            vec: Vec::with_capacity(capacity),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventField {
    Layer,
    Start,
    End,
    Style,
    Name,
    MarginL,
    MarginR,
    MarginV,
    Effect,
    Text,
}
impl FromStr for EventField {
    type Err = EventFieldParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "Layer" => Ok(EventField::Layer),
            "Start" => Ok(EventField::Start),
            "End" => Ok(EventField::End),
            "Style" => Ok(EventField::Style),
            "Name" => Ok(EventField::Name),
            "MarginL" => Ok(EventField::MarginL),
            "MarginR" => Ok(EventField::MarginR),
            "MarginV" => Ok(EventField::MarginV),
            "Effect" => Ok(EventField::Effect),
            "Text" => Ok(EventField::Text),
            _ => Err(EventFieldParseError::UnknownField {
                field: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for EventField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                EventField::Layer => "Layer",
                EventField::Start => "Start",
                EventField::End => "End",
                EventField::Style => "Style",
                EventField::Name => "Name",
                EventField::MarginL => "MarginL",
                EventField::MarginR => "MarginR",
                EventField::MarginV => "MarginV",
                EventField::Effect => "Effect",
                EventField::Text => "Text",
            }
        )
    }
}
#[derive(Default)]
pub struct EventParser {
    format_map: HashMap<EventField, usize>,
}

fn parse_event_line_prefix(line: &str) -> Option<(&str, bool)> {
    if let Some(data) = line
        .strip_prefix("Dialogue:")
        .map(|s| s.strip_prefix(" ").unwrap_or(""))
    {
        // NOTE: 后方 Text 应该保留手动添加的空白
        return Some((data, true));
    }

    if let Some(data) = line
        .strip_prefix("Comment:")
        .map(|s| s.strip_prefix(" ").unwrap_or(""))
    {
        return Some((data, false));
    }

    // 其他类型目前不实现
    None
}

impl EventParser {
    pub fn learn_format(&mut self, format_data: &str) -> Result<()> {
        self.format_map = format_data
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<EventField>().map_err(ParseError::from))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .enumerate()
            .map(|(i, field)| (field, i))
            .collect();
        Ok(())
    }

    pub fn parse_event_line(&self, event_data: &str) -> Result<Event> {
        if self.format_map.is_empty() {
            return Err(ParseError::Format {
                message: "Cannot parse 'Event:' line because 'Format:' line has not been seen yet"
                    .to_string(),
            });
        }

        let (event_value, is_dialogue) =
            parse_event_line_prefix(event_data).ok_or_else(|| ParseError::Format {
                message: format!("不存在 Event 前缀: {}", event_data),
            })?;

        let values: Vec<&str> = event_value.splitn(self.format_map.len(), ',').collect();

        if values.len() != self.format_map.len() {
            return Err(ParseError::Format {
                message: format!(
                    "Event line has mismatched field count: expected {}, found {}",
                    self.format_map.len(),
                    values.len()
                ),
            });
        }

        let p = FieldParser {
            format_map: &self.format_map,
            values: &values,
        };

        if is_dialogue {
            let dialogue = Dialogue {
                layer: p.parse_num(&EventField::Layer, 0i32)?,
                start: p.parse_ass_time(&EventField::Start, AssTime::from_centiseconds(0))?,
                end: p.parse_ass_time(&EventField::End, AssTime::from_centiseconds(0))?,
                style: p
                    .get_str(&EventField::Style)
                    .map(|s| s.trim())
                    .unwrap_or("Default")
                    .to_string(),
                name: p.get_str(&EventField::Name).and_then(|s| {
                    let val = s.trim();
                    if !val.is_empty() {
                        Some(val.to_string())
                    } else {
                        None
                    }
                }),
                margin_l: p.parse_num(&EventField::MarginL, 0i32)?,
                margin_r: p.parse_num(&EventField::MarginR, 0i32)?,
                margin_v: p.parse_num(&EventField::MarginV, 0i32)?,
                effect: p.parse_effect(&EventField::Effect, None)?,
                content: p
                    .parse_content(&EventField::Text, vec![TextSegment::Text("".to_string())])?,
            };
            Ok(Event::Dialogue(dialogue))
        } else {
            let comment = Comment {
                layer: p.parse_num(&EventField::Layer, 0i32)?,
                start: p.parse_ass_time(&EventField::Start, AssTime::from_centiseconds(0))?,
                end: p.parse_ass_time(&EventField::End, AssTime::from_centiseconds(0))?,
                style: p
                    .get_str(&EventField::Style)
                    .map(|s| s.trim())
                    .unwrap_or("Default")
                    .to_string(),
                name: p.get_str(&EventField::Name).and_then(|s| {
                    let val = s.trim();
                    if !val.is_empty() {
                        Some(val.to_string())
                    } else {
                        None
                    }
                }),
                margin_l: p.parse_num(&EventField::MarginL, 0i32)?,
                margin_r: p.parse_num(&EventField::MarginR, 0i32)?,
                margin_v: p.parse_num(&EventField::MarginV, 0i32)?,
                effect: p.parse_effect(&EventField::Effect, None)?,
                text: p.get_str(&EventField::Text).unwrap_or("").to_string(),
            };

            Ok(Event::Comment(comment))
        }
    }
}

/// 内部辅助结构体，用于简化单行内字段的查找和解析。
struct FieldParser<'a> {
    format_map: &'a HashMap<EventField, usize>,
    values: &'a [&'a str],
}

impl<'a> FieldParser<'a> {
    fn get_str(&self, name: &EventField) -> Option<&'a str> {
        self.format_map
            .get(name)
            .and_then(|&idx| self.values.get(idx))
            .copied()
    }

    fn parse_num<T: std::str::FromStr>(&self, name: &EventField, default: T) -> Result<T>
    where
        T::Err: Error + Send + Sync + 'static,
    {
        match self.get_str(name) {
            Some(val) => T::from_str(val).map_err(|e| ParseError::InvalidFieldValue {
                field: name.to_string(),
                value: val.to_string(),
                reason: e.to_string(),
            }),
            None => Ok(default),
        }
    }

    fn parse_ass_time(&self, name: &EventField, default: AssTime) -> Result<AssTime> {
        match self.get_str(name) {
            Some(val) => AssTime::from_str(val).map_err(|e| ParseError::InvalidFieldValue {
                field: name.to_string(),
                value: val.to_string(),
                reason: e.to_string(),
            }),
            None => Ok(default),
        }
    }

    fn parse_effect(&self, name: &EventField, default: Option<Effect>) -> Result<Option<Effect>> {
        match self.get_str(name) {
            Some(val) => {
                let trimmed = val.trim();
                if trimmed.is_empty() {
                    // 空字符串视为无效果
                    Ok(default)
                } else {
                    Effect::from_str(trimmed)
                        .map(Some)
                        .map_err(|e| ParseError::InvalidFieldValue {
                            field: name.to_string(),
                            value: val.to_string(),
                            reason: e.to_string(),
                        })
                }
            }
            None => Ok(default),
        }
    }

    fn parse_content(
        &self,
        name: &EventField,
        default: Vec<TextSegment>,
    ) -> Result<Vec<TextSegment>> {
        match self.get_str(name) {
            Some(val) => {
                // 直接调用独立的解析函数
                match parse_dialogue_line(val) {
                    Ok(("", segments)) => Ok(segments),
                    Ok((remaining, _)) => Err(ParseError::DialogueLine {
                        message: format!("未能完全解析内容，剩余部分: '{}'", remaining),
                    }),
                    Err(e) => Err(ParseError::DialogueLine {
                        message: format!("解析内容失败: {}", e),
                    }),
                }
            }
            None => Ok(default),
        }
    }
}

/// 解析对话行的文本内容
///
/// 解析包含普通文本和样式覆盖标签的混合内容。
/// 该函数被标记为 pub 是因为测试代码需要访问它来验证解析行为。
pub fn parse_dialogue_line(input: &str) -> IResult<&str, Vec<TextSegment>> {
    many0(alt((
        // 选项1: 解析样式覆盖块（仅当存在成对的花括号时）
        map(parse_multiple_overrides, |tags| {
            TextSegment::Overrides(tags)
        }),
        // 选项2: 单个左花括号视为普通文本
        map(tag("{"), |_| TextSegment::Text("{".to_string())),
        // 选项3: 解析普通文本，直到下一个 '{'
        map(take_while1(|c| c != '{'), |s: &str| {
            TextSegment::Text(s.to_string())
        }),
    )))
    .parse(input)
}

// 解析 "Scroll up;y1;y2;delay"
fn parse_scroll_up(input: &str) -> IResult<&str, Effect> {
    let (input, (_, y1, y2, delay)) = (
        tag("Scroll up"),
        preceded(char(';'), complete::i32),
        preceded(char(';'), complete::i32),
        preceded(char(';'), complete::i32),
    )
        .parse(input)?;

    Ok((input, Effect::ScrollUp { y1, y2, delay }))
}

// 解析 "Scroll down;y1;y2;delay"
fn parse_scroll_down(input: &str) -> IResult<&str, Effect> {
    let (input, (_, y1, y2, delay)) = (
        tag("Scroll down"),
        preceded(char(';'), complete::i32),
        preceded(char(';'), complete::i32),
        preceded(char(';'), complete::i32),
    )
        .parse(input)?;
    Ok((input, Effect::ScrollDown { y1, y2, delay }))
}

// 解析 "Banner;delay;direction?"
fn parse_banner(input: &str) -> IResult<&str, Effect> {
    let (input, (_, delay, direction_opt)) = (
        tag("Banner"),
        preceded(char(';'), complete::i32),
        opt(preceded(char(';'), complete::i32)),
    )
        .parse(input)?;

    Ok((
        input,
        Effect::Banner {
            delay,
            left_to_right: direction_opt.map(|d| d != 0),
        },
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ScrollUp {
        // WARN: 存储时自行处理大小顺序
        // y1 < y2
        y1: i32,
        y2: i32,
        // WARN: 最小钳制到1，存储时注意处理
        delay: i32,
    },
    ScrollDown {
        y1: i32,
        y2: i32,
        // WARN: 最小钳制到1，存储时注意处理
        delay: i32,
    },
    Banner {
        // WARN: 最小钳制到1，存储时注意处理
        delay: i32,
        left_to_right: Option<bool>,
    },
    // WARN: 还有 Karaoke Effect 在覆盖样式当中
}

impl FromStr for Effect {
    type Err = EffectParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let trimmed = s.trim();

        match all_consuming(alt((parse_scroll_up, parse_scroll_down, parse_banner))).parse(trimmed)
        {
            Ok((_, effect)) => Ok(effect),
            Err(_) => {
                // 尝试识别效果类型以提供更好的错误信息
                if trimmed.starts_with("Scroll up") {
                    Err(EffectParseError::InvalidScrollParameters)
                } else if trimmed.starts_with("Scroll down") {
                    Err(EffectParseError::InvalidScrollParameters)
                } else if trimmed.starts_with("Banner") {
                    Err(EffectParseError::InvalidBannerParameters)
                } else if trimmed.is_empty() {
                    // HACK: 空字符串不应该被解析为错误，应该在调用方处理
                    Err(EffectParseError::InvalidFormat {
                        input: trimmed.to_string(),
                    })
                } else {
                    Err(EffectParseError::UnknownEffect {
                        effect_type: trimmed.to_string(),
                    })
                }
            }
        }
    }
}
impl Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

// 代表 [Events] 块中的一行。
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Dialogue(Dialogue),
    Comment(Comment),
}
#[derive(Debug, Clone, PartialEq)]
pub struct Dialogue {
    pub layer: i32,
    pub start: AssTime,
    pub end: AssTime,
    pub style: String,
    pub name: Option<String>,
    pub margin_l: i32,
    pub margin_r: i32,
    pub margin_v: i32,
    pub effect: Option<Effect>,
    /// 对话文本，已解析为文本片段和样式标签的序列。
    pub content: Vec<TextSegment>,
}

impl Dialogue {
    pub fn plain_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|ts| {
                if let TextSegment::Text(t) = ts {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// 注释行，渲染时会被忽略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub layer: i32,
    pub start: AssTime,
    pub end: AssTime,
    pub style: String,
    pub name: Option<String>,
    pub margin_l: i32,
    pub margin_r: i32,
    pub margin_v: i32,
    pub effect: Option<Effect>,
    /// 原始注释文本。
    pub text: String,
}
// 对话内容的组成部分。
#[derive(Debug, Clone, PartialEq)]
pub enum TextSegment {
    // 普通文本字符串
    // INFO: 一些文本特殊字符
    // 不间断空格 (`\h`) 无效的 (中文无用)
    // \N - 强制换行
    // \n - 根据 wrap_style 决定是否强制换行
    Text(String),
    // 样式覆盖标签块 (`{...}`)
    Overrides(Vec<StyleTag>),
    // 绘图命令(开始绘画与结束绘画命令在样式覆盖当中)
    Drawing(Vec<DrawingCommand>),
}
impl Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dialogue(d) => {
                write!(
                    f,
                    "Dialogue:{},{},{},{},{},{},{},{},{},",
                    d.layer,
                    d.start,
                    d.end,
                    d.style,
                    d.name.as_deref().unwrap_or(""),
                    d.margin_l,
                    d.margin_r,
                    d.margin_v,
                    d.effect
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or(String::new())
                )?;
                for text_segment in &d.content {
                    match text_segment {
                        TextSegment::Text(s) => {
                            write!(f, "{}", s)?;
                        }
                        TextSegment::Overrides(style_tags) => {
                            for st in style_tags {
                                write!(f, "{}", st)?;
                            }
                        }
                        TextSegment::Drawing(drawing_commands) => {
                            for dc in drawing_commands {
                                write!(f, "{}", dc)?;
                            }
                        }
                    }
                }
                Ok(())
            }
            Self::Comment(c) => {
                write!(
                    f,
                    "Comment:{},{},{},{},{},{},{},{},{},{}",
                    c.layer,
                    c.start,
                    c.end,
                    c.style,
                    c.name.as_deref().unwrap_or(""),
                    c.margin_l,
                    c.margin_r,
                    c.margin_v,
                    c.effect
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or(String::new()),
                    c.text
                )?;
                Ok(())
            }
        }
    }
}
