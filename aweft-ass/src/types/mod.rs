pub mod definitions;
mod drawing;
mod event;
mod implementations;
mod parse_drawing_command;
mod parse_style_override;
mod script;
mod style;
mod tags;
mod time;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointF64 {
    pub x: f64,
    pub y: f64,
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

#[derive(Debug, Clone, PartialEq)]
pub enum BoldType {
    // Same with Weight(700)
    Bold,
    // Same with Weight(400)
    NonBold,
    Weight(u32),
}
pub use definitions::*; // 将类型公开
