use crate::types::drawing::DrawingCommand;

use super::{tags::Tag, time::AssTime};
// 代表 [Events] 块中的一行。
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Dialogue(Dialogue),
    Comment(Comment),
}
#[derive(Debug, Clone, PartialEq)]
pub struct Dialogue {
    pub layer: u32,
    pub start: AssTime,
    pub end: AssTime,
    pub style: String,
    pub actor: Option<String>,
    pub margin_l: u32,
    pub margin_r: u32,
    pub margin_v: u32,
    pub effect: Option<Effect>,
    /// 对话文本，已解析为文本片段和样式标签的序列。
    pub content: Vec<TextSegment>,
}

/// 注释行，渲染时会被忽略。
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub is_marker: bool, // Aegisub 中的 "Marker" 注释
    pub layer: u32,
    pub start: AssTime,
    pub end: AssTime,
    pub style: String,
    pub actor: Option<String>,
    pub margin_l: u32,
    pub margin_r: u32,
    pub margin_v: u32,
    pub effect: Option<Effect>,
    /// 原始注释文本。
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ScrollUp {
        // y1 < y2
        y1: u16,
        y2: u16,
        delay: u16,
    },
    ScrollDown {
        y1: u16,
        y2: u16,
        delay: u16,
    },
    Banner {
        delay: u8, // [0,100]
        left_to_right: Option<bool>,
    },
    // WARN: 还有 Karaoke Effect 在覆盖样式当中
}

// 对话内容的组成部分。
#[derive(Debug, Clone, PartialEq)]
pub enum TextSegment {
    // 普通文本字符串。
    Text(String),
    // 换行符 (`\n` 或 `\N`)。
    Newline,
    // 硬空格 (`\h`)。
    HardSpace,
    // 样式覆盖标签块 (`{...}`)。
    Overrides(Vec<Tag>),
    // 绘图命令(开始绘画与结束绘画命令在样式覆盖当中)
    Drawing(Vec<DrawingCommand>),
}
