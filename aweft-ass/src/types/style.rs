use crate::error::{BorderStyleParseError, ParseError, Result, StyleFieldParseError};
use crate::types::event::{Event, Events};
use aweft_libass::types::{Alignment, AssColorRgba};
use log::warn;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use super::BoldType;

#[derive(Debug, PartialEq, Clone)]
pub enum BorderStyle {
    // 建议使用 1
    OutlineAndDropShadow = 1,
    OpaqueBox = 3, // 矩形框
}
impl FromStr for BorderStyle {
    type Err = BorderStyleParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "1" => Ok(BorderStyle::OutlineAndDropShadow),
            "3" => Ok(BorderStyle::OpaqueBox),
            _ => Err(BorderStyleParseError::InvalidValue {
                value: s.to_string(),
            }),
        }
    }
}
impl Display for BorderStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BorderStyle::OutlineAndDropShadow => "1",
            BorderStyle::OpaqueBox => "3",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
pub struct Styles {
    map: HashMap<String, StyleWithoutName>,
}

impl Deref for Styles {
    type Target = HashMap<String, StyleWithoutName>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl DerefMut for Styles {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

impl From<HashMap<String, StyleWithoutName>> for Styles {
    fn from(map: HashMap<String, StyleWithoutName>) -> Self {
        Self { map }
    }
}

impl From<Styles> for HashMap<String, StyleWithoutName> {
    fn from(styles: Styles) -> Self {
        styles.map
    }
}

impl Extend<(String, StyleWithoutName)> for Styles {
    fn extend<T: IntoIterator<Item = (String, StyleWithoutName)>>(&mut self, iter: T) {
        self.map.extend(iter);
    }
}

impl FromIterator<(String, StyleWithoutName)> for Styles {
    fn from_iter<T: IntoIterator<Item = (String, StyleWithoutName)>>(iter: T) -> Self {
        Self {
            map: HashMap::from_iter(iter),
        }
    }
}

impl IntoIterator for Styles {
    type Item = (String, StyleWithoutName);
    type IntoIter = std::collections::hash_map::IntoIter<String, StyleWithoutName>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.into_iter()
    }
}

impl<'a> IntoIterator for &'a Styles {
    type Item = (&'a String, &'a StyleWithoutName);
    type IntoIter = std::collections::hash_map::Iter<'a, String, StyleWithoutName>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

impl<'a> IntoIterator for &'a mut Styles {
    type Item = (&'a String, &'a mut StyleWithoutName);
    type IntoIter = std::collections::hash_map::IterMut<'a, String, StyleWithoutName>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.iter_mut()
    }
}

impl Styles {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
        }
    }

    pub fn rename(
        &mut self,
        old_name: &str,
        new_name: impl Into<String>,
        events: Events,
    ) -> Result<()> {
        let new_name = new_name.into();
        if let Some(style_without_name) = self.remove(old_name) {
            self.insert(new_name.clone(), style_without_name);

            for event in events {
                match event {
                    Event::Dialogue(mut d) => {
                        if d.style == old_name {
                            d.style = new_name.clone()
                        }
                    }
                    Event::Comment(mut c) => {
                        if c.style == old_name {
                            c.style = new_name.clone();
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn duplicate(&mut self, source: &str, new_name: impl Into<String>) -> Result<()> {
        let value = self
            .get(source)
            .ok_or_else(|| ParseError::Format {
                message: format!("源样式 '{}' 不存在", source),
            })?
            .clone();

        self.insert(new_name.into(), value.clone());
        Ok(())
    }
}

// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_types.h#L105-L133
#[derive(Debug, PartialEq, Clone)]
pub struct StyleWithoutName {
    pub fontname: String,
    pub fontsize: f64,
    // 默认显示的颜色 读取和写入需要解析类似 &H00FFFFFF
    pub primary_color: AssColorRgba,
    // 次要颜色
    pub secondary_color: AssColorRgba,
    // 轮廓的颜色和透明度
    pub outline_color: AssColorRgba,
    // 阴影的颜色
    pub back_color: AssColorRgba,
    // 粗体
    pub bold: BoldType,
    // 斜体
    // WARN: 注意规范当中布尔类型 -1 是 True
    pub italic: bool,
    // 下划线
    pub underline: bool,
    // 删除线
    pub strikeout: bool,
    // 字体宽度伸缩率 [百分比]
    pub scale_x: f64,
    // 字体的高度伸缩率 [百分比]
    pub scale_y: f64,
    // 字符间距
    pub spacing: f64,
    // 角度 [度]
    pub angle: f64,
    pub border_style: BorderStyle,
    // 轮廓宽度 [像素]
    pub outline: f64,
    // 阴影的深度为像素数
    pub shadow: f64,
    pub alignment: Alignment,
    // 距离边界距离
    pub margin_l: i32,
    pub margin_r: i32,
    // 对于字幕来说是距离底部的距离
    pub margin_v: i32,
    // 必须是 1 (解析时无视文件内定义)
    pub encoding: i32,
}

impl Default for StyleWithoutName {
    fn default() -> Self {
        StyleWithoutName {
            // NOTE: length <= 31
            fontname: "Arial".to_string(),
            // NOTE: length <= 511
            fontsize: 18.0,
            primary_color: AssColorRgba::new(255, 255, 255, 255),
            secondary_color: AssColorRgba::new(0, 255, 255, 255),
            outline_color: AssColorRgba::new(0, 0, 0, 0),
            back_color: AssColorRgba::new(0, 0, 0, 128),
            bold: BoldType::Weight(200),
            italic: false,
            underline: false,
            strikeout: false,
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

impl Display for StyleWithoutName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.fontname,
            self.fontsize,
            [
                "&H",
                &format!("{:08X}", self.primary_color.to_ass_style_u32())
            ]
            .concat(),
            [
                "&H",
                &format!("{:08X}", self.secondary_color.to_ass_style_u32())
            ]
            .concat(),
            [
                "&H",
                &format!("{:08X}", self.outline_color.to_ass_style_u32())
            ]
            .concat(),
            ["&H", &format!("{:08X}", self.back_color.to_ass_style_u32())].concat(),
            self.bold,
            self.italic,
            self.underline,
            self.strikeout,
            self.scale_x,
            self.scale_y,
            self.spacing,
            self.angle,
            self.border_style,
            self.outline,
            self.shadow,
            self.alignment,
            self.margin_l,
            self.margin_r,
            self.margin_v,
            self.encoding
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleField {
    Name,
    Fontname,
    Fontsize,
    PrimaryColour,
    SecondaryColour,
    OutlineColour,
    BackColour,
    Bold,
    Italic,
    Underline,
    StrikeOut,
    ScaleX,
    ScaleY,
    Spacing,
    Angle,
    BorderStyle,
    Outline,
    Shadow,
    Alignment,
    MarginL,
    MarginR,
    MarginV,
    Encoding,
}
impl FromStr for StyleField {
    type Err = StyleFieldParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "Name" => Ok(StyleField::Name),
            "Fontname" => Ok(StyleField::Fontname),
            "Fontsize" => Ok(StyleField::Fontsize),
            "PrimaryColour" => Ok(StyleField::PrimaryColour),
            "SecondaryColour" => Ok(StyleField::SecondaryColour),
            "OutlineColour" => Ok(StyleField::OutlineColour),
            "BackColour" => Ok(StyleField::BackColour),
            "Bold" => Ok(StyleField::Bold),
            "Italic" => Ok(StyleField::Italic),
            "Underline" => Ok(StyleField::Underline),
            "StrikeOut" => Ok(StyleField::StrikeOut),
            "ScaleX" => Ok(StyleField::ScaleX),
            "ScaleY" => Ok(StyleField::ScaleY),
            "Spacing" => Ok(StyleField::Spacing),
            "Angle" => Ok(StyleField::Angle),
            "BorderStyle" => Ok(StyleField::BorderStyle),
            "Outline" => Ok(StyleField::Outline),
            "Shadow" => Ok(StyleField::Shadow),
            "Alignment" => Ok(StyleField::Alignment),
            "MarginL" => Ok(StyleField::MarginL),
            "MarginR" => Ok(StyleField::MarginR),
            "MarginV" => Ok(StyleField::MarginV),
            "Encoding" => Ok(StyleField::Encoding),
            _ => Err(StyleFieldParseError::UnknownField {
                field: s.to_string(),
            }),
        }
    }
}
impl fmt::Display for StyleField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // match 表达式直接返回 &str，然后 write! 宏将其写入格式化器
        write!(
            f,
            "{}",
            match self {
                StyleField::Name => "Name",
                StyleField::Fontname => "Fontname",
                StyleField::Fontsize => "Fontsize",
                StyleField::PrimaryColour => "PrimaryColour",
                StyleField::SecondaryColour => "SecondaryColour",
                StyleField::OutlineColour => "OutlineColour",
                StyleField::BackColour => "BackColour",
                StyleField::Bold => "Bold",
                StyleField::Italic => "Italic",
                StyleField::Underline => "Underline",
                StyleField::StrikeOut => "StrikeOut",
                StyleField::ScaleX => "ScaleX",
                StyleField::ScaleY => "ScaleY",
                StyleField::Spacing => "Spacing",
                StyleField::Angle => "Angle",
                StyleField::BorderStyle => "BorderStyle",
                StyleField::Outline => "Outline",
                StyleField::Shadow => "Shadow",
                StyleField::Alignment => "Alignment",
                StyleField::MarginL => "MarginL",
                StyleField::MarginR => "MarginR",
                StyleField::MarginV => "MarginV",
                StyleField::Encoding => "Encoding",
            }
        )
    }
}

// 默认的 Style Format 顺序（与 libass 一致）
const DEFAULT_STYLE_FORMAT: &[StyleField] = &[
    StyleField::Name,
    StyleField::Fontname,
    StyleField::Fontsize,
    StyleField::PrimaryColour,
    StyleField::SecondaryColour,
    StyleField::OutlineColour,
    StyleField::BackColour,
    StyleField::Bold,
    StyleField::Italic,
    StyleField::Underline,
    StyleField::StrikeOut,
    StyleField::ScaleX,
    StyleField::ScaleY,
    StyleField::Spacing,
    StyleField::Angle,
    StyleField::BorderStyle,
    StyleField::Outline,
    StyleField::Shadow,
    StyleField::Alignment,
    StyleField::MarginL,
    StyleField::MarginR,
    StyleField::MarginV,
    StyleField::Encoding,
];

#[derive(Default)]
pub struct StyleParser {
    format_map: HashMap<StyleField, usize>,
}

impl StyleParser {
    pub fn learn_format(&mut self, format_data: &str) -> Result<()> {
        // 允许空 Format 行或包含空标记的行：过滤空白后若无字段，则保持空映射
        let mut fields: Vec<StyleField> = format_data
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<StyleField>().map_err(ParseError::from))
            .collect::<Result<Vec<_>>>()?;

        if fields.is_empty() {
            // 使用默认顺序
            fields = DEFAULT_STYLE_FORMAT.to_vec();
        }

        self.format_map = fields
            .into_iter()
            .enumerate()
            .map(|(i, field)| (field, i))
            .collect();
        Ok(())
    }

    pub fn parse_style_line(&self, style_data: &str) -> Result<(String, StyleWithoutName)> {
        if self.format_map.is_empty() {
            return Err(ParseError::Format {
                message: "Cannot parse 'Style:' line because 'Format:' line has not been seen yet"
                    .to_string(),
            });
        }

        let style_values = style_data
            .strip_prefix("Style:")
            .ok_or_else(|| ParseError::Format {
                message: format!("不存在 Style 前缀: {}", style_data),
            })?
            .trim();

        let values: Vec<&str> = style_values.splitn(self.format_map.len(), ',').collect();

        if values.len() != self.format_map.len() {
            return Err(ParseError::Format {
                message: format!(
                    "Style line has mismatched field count: expected {}, found {}",
                    self.format_map.len(),
                    values.len()
                ),
            });
        }

        let p = FieldParser {
            format_map: &self.format_map,
            values: &values,
        };

        // Name 字段：允许缺失或为空，缺失/为空时使用 "Default"
        let name = p
            .get_str(&StyleField::Name)
            .map(|s| {
                let s2 = s.trim();
                if s2.is_empty() {
                    warn!("Style 不存在 Name 字段，默认填充 Default");
                    "Default"
                } else {
                    s2
                }
            })
            .unwrap_or("Default")
            .to_string();

        let style = StyleWithoutName {
            fontname: p
                .get_str(&StyleField::Fontname)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("Arial")
                .to_string(),
            fontsize: p.parse_num(&StyleField::Fontsize, 18.0)?,
            primary_color: p.parse_color(
                &StyleField::PrimaryColour,
                AssColorRgba::new(255, 255, 255, 255),
            )?,
            secondary_color: p.parse_color(
                &StyleField::SecondaryColour,
                AssColorRgba::new(0, 255, 255, 255),
            )?,
            outline_color: p
                .parse_color(&StyleField::OutlineColour, AssColorRgba::new(0, 0, 0, 0))?,
            back_color: p.parse_color(&StyleField::BackColour, AssColorRgba::new(0, 0, 0, 128))?,
            bold: p.parse_ass_bold(&StyleField::Bold, BoldType::NonBold)?,
            italic: p.parse_ass_bool(&StyleField::Italic, false)?,
            underline: p.parse_ass_bool(&StyleField::Underline, false)?,
            strikeout: p.parse_ass_bool(&StyleField::StrikeOut, false)?,
            scale_x: p.parse_num(&StyleField::ScaleX, 100.0)?,
            scale_y: p.parse_num(&StyleField::ScaleY, 100.0)?,
            spacing: p.parse_num(&StyleField::Spacing, 0.0)?,
            angle: p.parse_num(&StyleField::Angle, 0.0)?,
            border_style: p.parse_ass_border_style(
                &StyleField::BorderStyle,
                BorderStyle::OutlineAndDropShadow,
            )?,
            outline: p.parse_num(&StyleField::Outline, 2.0)?,
            shadow: p.parse_num(&StyleField::Shadow, 2.0)?,
            alignment: p.parse_ass_alignment(&StyleField::Alignment, Alignment::BottomCenter)?,
            margin_l: p.parse_num(&StyleField::MarginL, 10)?,
            margin_r: p.parse_num(&StyleField::MarginR, 10)?,
            margin_v: p.parse_num(&StyleField::MarginV, 10)?,
            encoding: p.parse_num(&StyleField::Encoding, 1)?,
        };

        Ok((name, style))
    }
}

/// 内部辅助结构体，用于简化单行内字段的查找和解析。
struct FieldParser<'a> {
    format_map: &'a HashMap<StyleField, usize>,
    values: &'a [&'a str],
}

impl<'a> FieldParser<'a> {
    fn get_str(&self, name: &StyleField) -> Option<&'a str> {
        self.format_map
            .get(name)
            .and_then(|&idx| self.values.get(idx))
            .copied()
    }

    fn parse_num<T: std::str::FromStr>(&self, name: &StyleField, default: T) -> Result<T>
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

    fn parse_ass_bold(&self, name: &StyleField, default: BoldType) -> Result<BoldType> {
        match self.get_str(name) {
            Some(val) => {
                let parsed =
                    i32::from_str(val.trim()).map_err(|e| ParseError::InvalidFieldValue {
                        field: name.to_string(),
                        value: val.to_string(),
                        reason: e.to_string(),
                    })?;
                // INFO: 在 Style 部分当中不可以定义权重
                Ok(if parsed != 0 {
                    BoldType::Bold
                } else {
                    BoldType::NonBold
                })
            }
            None => Ok(default),
        }
    }

    fn parse_ass_border_style(
        &self,
        name: &StyleField,
        default: BorderStyle,
    ) -> Result<BorderStyle> {
        match self.get_str(name) {
            Some(val) => BorderStyle::from_str(val).map_err(|e| ParseError::InvalidFieldValue {
                field: name.to_string(),
                value: val.to_string(),
                reason: e.to_string(),
            }),
            None => Ok(default),
        }
    }

    fn parse_ass_alignment(&self, name: &StyleField, default: Alignment) -> Result<Alignment> {
        match self.get_str(name) {
            Some(val) => {
                // aweft-libass 的 AlignmentParseError 会自动通过 #[from] 转换为 ParseError
                Ok(Alignment::from_str(val)?)
            }
            None => Ok(default),
        }
    }

    fn parse_ass_bool(&self, name: &StyleField, default: bool) -> Result<bool> {
        self.parse_num(name, if default { -1 } else { 0 })
            .map(|v: i32| v != 0)
    }

    fn parse_color(&self, name: &StyleField, default: AssColorRgba) -> Result<AssColorRgba> {
        match self.get_str(name) {
            Some(val) => val
                .strip_prefix("&H")
                .and_then(|hex_str| u32::from_str_radix(hex_str, 16).ok())
                .map(AssColorRgba::from_ass_style_u32)
                .ok_or_else(|| ParseError::InvalidFieldValue {
                    field: name.to_string(),
                    value: val.to_string(),
                    reason: "Invalid color format: expected '&H<AABBGGRR>'".to_string(),
                }),
            None => Ok(default),
        }
    }
}
