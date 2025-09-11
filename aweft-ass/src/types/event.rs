use anyhow::{anyhow, bail, Context, Result};
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
use std::str::FromStr;
use std::{collections::HashMap, fmt};
use std::{error::Error, fmt::Display};

use super::{parse_style_override::parse_multiple_overrides, tags::StyleTag, time::AssTime};

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
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
            _ => bail!("未知的 Event 格式字段: '{}'", s),
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

    // It's neither, so return None.
    None
}

impl EventParser {
    pub fn learn_format(&mut self, format_data: &str) -> Result<()> {
        self.format_map = format_data
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<EventField>())
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .enumerate()
            .map(|(i, field)| (field, i))
            .collect();
        Ok(())
    }

    pub fn parse_event_line(&self, event_data: &str) -> Result<Event> {
        if self.format_map.is_empty() {
            bail!("Cannot parse 'Event:' line because 'Format:' line has not been seen yet");
        }

        let (event_value, is_dialogue) =
            parse_event_line_prefix(event_data).context(anyhow!("不存在 Event 前缀"))?;

        let values: Vec<&str> = event_value.splitn(self.format_map.len(), ',').collect();

        if values.len() != self.format_map.len() {
            bail!(
                "Style line has mismatched field count: expected {}, found {}",
                self.format_map.len(),
                values.len()
            );
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
            Some(val) => T::from_str(val).with_context(|| {
                format!(
                    "Failed to parse number for field '{}' from value '{}'",
                    name, val
                )
            }),
            None => Ok(default),
        }
    }

    fn parse_ass_time(&self, name: &EventField, default: AssTime) -> Result<AssTime> {
        match self.get_str(name) {
            Some(val) => AssTime::from_str(val)
                // HACK: 需要为 AssTime 实现错误类型
                .map_err(|e| anyhow!(e))
                .with_context(|| {
                    format!(
                        "Failed to parse ass_time for field '{}' from value '{}'",
                        name, val
                    )
                }),
            None => Ok(default),
        }
    }
    fn parse_effect(&self, name: &EventField, default: Option<Effect>) -> Result<Option<Effect>> {
        match self.get_str(name) {
            Some(val) => Effect::from_str(val)
                .map(Some)
                // HACK: 需要为 Effect 实现错误类型
                .map_err(|e| anyhow!(e))
                .with_context(|| {
                    format!(
                        "Failed to parse effect for field '{}' from value '{}'",
                        name, val
                    )
                }),
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
                    // Ok((remaining, segments)) 表示解析成功
                    // 我们通常期望整个字符串都被解析完，所以 remaining 应该是空的 ""
                    Ok(("", segments)) => Ok(segments),

                    // 如果 remaining 不是空的，说明有部分字符串没能解析，这可能是一个错误
                    Ok((remaining, _)) => {
                        Err(anyhow!("未能完全解析内容，剩余部分: '{}'", remaining))
                    }

                    // 如果解析直接失败
                    Err(e) => Err(anyhow!("解析内容失败: {}", e.to_string())),
                }
            }
            None => Ok(default),
        }
    }
}

// HACK: 可能这个不需要pub，但是测试解析期望 Text 的时候需要它
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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match all_consuming(alt((parse_scroll_up, parse_scroll_down, parse_banner))).parse(s.trim())
        {
            Ok((_, effect)) => Ok(effect),
            Err(e) => Err(e.to_string()),
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
