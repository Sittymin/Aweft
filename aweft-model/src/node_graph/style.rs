use palette::Srgba;
use serde::{Deserialize, Serialize};

use super::math::{ClipRect, Vec2, Vec3};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StyleResult {
    pub font_name: Option<String>,
    pub font_size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikeout: Option<bool>,
    pub color_primary: Option<Srgba>,
    pub color_secondary: Option<Srgba>,
    pub color_outline: Option<Srgba>,
    pub color_back: Option<Srgba>,
    pub border: Option<Vec2>,
    pub shadow: Option<Vec2>,
    pub font_scale: Option<Vec2>,
    pub font_rotation: Option<Vec3>,
    pub factor: Option<Vec2>,
    pub blur: Option<f64>,
    pub blur_edges: Option<u32>,
    /// 锚点的排版像素位置
    pub position: Option<Vec2>,
    /// 锚点相对排版尺寸的分数位置（0..1），与 `position` 相加
    pub position_ratio: Option<Vec2>,
    pub origin: Option<Vec2>,
    pub clip_rect: Option<ClipRect>,
}

impl StyleResult {
    pub fn merge_over(self, other: StyleResult) -> StyleResult {
        StyleResult {
            font_name: other.font_name.or(self.font_name),
            font_size: other.font_size.or(self.font_size),
            bold: other.bold.or(self.bold),
            italic: other.italic.or(self.italic),
            underline: other.underline.or(self.underline),
            strikeout: other.strikeout.or(self.strikeout),
            color_primary: other.color_primary.or(self.color_primary),
            color_secondary: other.color_secondary.or(self.color_secondary),
            color_outline: other.color_outline.or(self.color_outline),
            color_back: other.color_back.or(self.color_back),
            border: other.border.or(self.border),
            shadow: other.shadow.or(self.shadow),
            font_scale: other.font_scale.or(self.font_scale),
            font_rotation: other.font_rotation.or(self.font_rotation),
            factor: other.factor.or(self.factor),
            blur: other.blur.or(self.blur),
            blur_edges: other.blur_edges.or(self.blur_edges),
            position: other.position.or(self.position),
            position_ratio: other.position_ratio.or(self.position_ratio),
            origin: other.origin.or(self.origin),
            clip_rect: other.clip_rect.or(self.clip_rect),
        }
    }
}
