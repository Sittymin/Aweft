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
use slotmap::{new_key_type, SlotMap};

use crate::types::drawing::DrawingCommand;
use rayon::iter::{FromParallelIterator, IntoParallelIterator, ParallelIterator};
use std::{collections::HashMap, fmt, ops::RangeBounds};
use std::{error::Error, fmt::Display};
use std::{slice, str::FromStr};

use super::{parse_style_override::parse_multiple_overrides, tags::StyleTag, time::AssTime};

new_key_type! {
    pub struct EventKey;
}

#[derive(Debug, Clone, Default)]
pub struct Events {
    map: SlotMap<EventKey, Event>,
    order: Vec<EventKey>,
}

pub struct EventsIterMut<'a> {
    map: &'a mut SlotMap<EventKey, Event>,
    order_iter: slice::Iter<'a, EventKey>,
}
impl<'a> Iterator for EventsIterMut<'a> {
    type Item = &'a mut Event;
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.order_iter.next()?;
        let event_ref_option = self.map.get_mut(*id);
        // 内部指向 Event 的指针肯定是不会重复的
        unsafe { event_ref_option.map(|e| &mut *(e as *mut Event)) }
    }
}
pub struct EventsDrain<'a> {
    events: &'a mut Events,
    drained_keys: std::vec::IntoIter<EventKey>,
}
impl<'a> Iterator for EventsDrain<'a> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        let key = self.drained_keys.next()?;
        self.events.map.remove(key)
    }
}
impl<'a> Drop for EventsDrain<'a> {
    fn drop(&mut self) {
        for &key in self.drained_keys.as_ref() {
            self.events.map.remove(key);
        }
    }
}

impl Events {
    pub fn new() -> Self {
        Self {
            map: SlotMap::with_key(),
            order: Vec::new(),
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.order.iter().filter_map(|key| self.map.get(*key))
    }
    pub fn iter_mut(&mut self) -> EventsIterMut<'_> {
        EventsIterMut {
            map: &mut self.map,
            order_iter: self.order.iter(),
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn insert(&mut self, index: usize, event: Event) {
        let key = self.map.insert(event);
        self.order.insert(index, key);
    }
    pub fn remove(&mut self, index: usize) -> Event {
        let key = self.order.remove(index);
        self.map.remove(key).unwrap()
    }

    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&Event) -> bool,
    {
        self.order.retain(|&key| {
            let event = self.map.get(key).unwrap();

            if f(event) {
                true
            } else {
                self.map.remove(key);
                false
            }
        });
    }
    pub fn retain_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut Event) -> bool,
    {
        self.order.retain(|&key| {
            let event = self.map.get_mut(key).unwrap();

            if f(event) {
                true
            } else {
                self.map.remove(key);
                false
            }
        });
    }

    pub fn push(&mut self, event: Event) {
        let key = self.map.insert(event);
        self.order.push(key);
    }
    pub fn pop(&mut self) -> Option<Event> {
        if let Some(key) = self.order.pop() {
            return self.map.remove(key);
        }
        None
    }
    pub fn pop_if(&mut self, predicate: impl FnOnce(&mut Event) -> bool) -> Option<Event> {
        if let Some(&last_key) = self.order.last() {
            let event = self.map.get_mut(last_key).unwrap();
            if predicate(event) {
                self.order.pop();
                return self.map.remove(last_key);
            }
        }
        None
    }
    pub fn append(&mut self, other: &mut Vec<Event>) {
        self.order.reserve(other.len());
        self.map.reserve(other.len());
        for event in other.drain(..) {
            let key = self.map.insert(event);
            self.order.push(key);
        }
    }

    pub fn drain<R>(&mut self, range: R) -> EventsDrain<'_>
    where
        R: RangeBounds<usize>,
    {
        // 直接被消耗了
        let drained_keys_vec: Vec<EventKey> = self.order.drain(range).collect();

        EventsDrain {
            events: self,
            drained_keys: drained_keys_vec.into_iter(),
        }
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.map.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

impl IntoIterator for Events {
    type Item = Event;
    type IntoIter = std::vec::IntoIter<Event>;

    fn into_iter(mut self) -> Self::IntoIter {
        let mut ordered_events = Vec::with_capacity(self.order.len());

        for key in self.order {
            if let Some(event) = self.map.remove(key) {
                ordered_events.push(event);
            }
        }

        ordered_events.into_iter()
    }
}

impl From<Vec<Event>> for Events {
    fn from(vec: Vec<Event>) -> Self {
        let capacity = vec.len();
        let mut map = SlotMap::with_capacity_and_key(capacity);
        let mut order = Vec::with_capacity(capacity);

        for event in vec {
            let key = map.insert(event);
            order.push(key);
        }

        Self { map, order }
    }
}
impl FromIterator<Event> for Events {
    fn from_iter<T: IntoIterator<Item = Event>>(iter: T) -> Self {
        let mut map = SlotMap::with_key();
        let mut order = Vec::new();

        for event in iter {
            let key = map.insert(event);
            order.push(key);
        }

        Self { map, order }
    }
}

impl FromParallelIterator<Event> for Events {
    fn from_par_iter<I>(par_iter: I) -> Self
    where
        I: IntoParallelIterator<Item = Event>,
    {
        let events_vec: Vec<Event> = par_iter.into_par_iter().collect();
        events_vec.into()
    }
}

impl PartialEq<Vec<Event>> for Events {
    fn eq(&self, other: &Vec<Event>) -> bool {
        if self.len() != other.len() {
            return false;
        }

        self.iter().zip(other.iter()).all(|(a, b)| a == b)
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
