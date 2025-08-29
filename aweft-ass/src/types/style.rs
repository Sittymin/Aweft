use aweft_libass::{types::AssColorRgba, Alignment};

use super::BoldType;

#[derive(Debug, PartialEq, Clone)]
pub enum BorderStyle {
    OutlineAndDropShadow = 1,
    OpaqueBox = 3, // 矩形框
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
    // FIXME: 之前遗漏的字段
    pub blur: f64,
    // FIXME: 之前遗漏的字段(不明类型)
    pub justify: i32,
}
