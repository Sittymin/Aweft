use anyhow::{anyhow, bail, Context, Result};
use aweft_libass::types::{Alignment, AssColorRgba};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;

use super::BoldType;

#[derive(Debug, PartialEq, Clone)]
pub enum BorderStyle {
    // 建议使用 1
    OutlineAndDropShadow = 1,
    OpaqueBox = 3, // 矩形框
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
impl Display for BorderStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BorderStyle::OutlineAndDropShadow => "1",
            BorderStyle::OpaqueBox => "3",
        };
        write!(f, "{}", s)
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
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
            _ => bail!("Unknown or unsupported style format field: '{}'", s),
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

#[derive(Default)]
pub struct StyleParser {
    format_map: HashMap<StyleField, usize>,
}

impl StyleParser {
    pub fn learn_format(&mut self, format_data: &str) -> Result<()> {
        self.format_map = format_data
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.parse::<StyleField>())
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .enumerate()
            .map(|(i, field)| (field, i))
            .collect();
        Ok(())
    }

    pub fn parse_style_line(&self, style_data: &str) -> Result<(String, StyleWithoutName)> {
        if self.format_map.is_empty() {
            bail!("Cannot parse 'Style:' line because 'Format:' line has not been seen yet");
        }

        let style_values = style_data
            .strip_prefix("Style:")
            .context(anyhow!("不存在 Style 前缀"))?
            .trim();

        let values: Vec<&str> = style_values.splitn(self.format_map.len(), ',').collect();

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

        let name = p
            .get_str(&StyleField::Name)
            .context("Required field 'Name' not found in Format line")?
            .to_string();

        let style = StyleWithoutName {
            fontname: p
                .get_str(&StyleField::Fontname)
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
            Some(val) => T::from_str(val).with_context(|| {
                format!(
                    "Failed to parse number for field '{}' from value '{}'",
                    name, val
                )
            }),
            None => Ok(default),
        }
    }

    fn parse_ass_bold(&self, name: &StyleField, default: BoldType) -> Result<BoldType> {
        match self.get_str(name) {
            Some(val) => BoldType::from_str(val)
                // HACK: 需要为 BoldType 实现错误类型
                .map_err(|e| anyhow!(e))
                .with_context(|| {
                    format!(
                        "Failed to parse bold for field '{}' from value '{}'",
                        name, val
                    )
                }),
            None => Ok(default),
        }
    }
    fn parse_ass_border_style(
        &self,
        name: &StyleField,
        default: BorderStyle,
    ) -> Result<BorderStyle> {
        match self.get_str(name) {
            Some(val) => BorderStyle::from_str(val)
                // HACK: 需要为 BorderStyle 实现错误类型
                .map_err(|e| anyhow!(e))
                .with_context(|| {
                    format!(
                        "Failed to parse border style for field '{}' from value '{}'",
                        name, val
                    )
                }),
            None => Ok(default),
        }
    }
    fn parse_ass_alignment(&self, name: &StyleField, default: Alignment) -> Result<Alignment> {
        match self.get_str(name) {
            Some(val) => Alignment::from_str(val)
                // HACK: 需要为 Alignment 实现错误类型
                .map_err(|e| anyhow!(e))
                .with_context(|| {
                    format!(
                        "Failed to parse alignment for field '{}' from value '{}'",
                        name, val
                    )
                }),
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
                .ok_or_else(|| {
                    anyhow!(
                        "Invalid color format for field '{}': expected '&H<AABBGGRR>', got '{}'",
                        name,
                        val
                    )
                }),
            None => Ok(default),
        }
    }
}
