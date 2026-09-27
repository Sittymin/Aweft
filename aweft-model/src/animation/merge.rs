use palette::Srgba;

use crate::animation::anim_data::{BlendMode, PropertyPath};
use crate::animation::sample::EvaluatedStrip;
use crate::node_graph::style::StyleResult;

/// Apply evaluated animation strips to a `StyleResult`, mutating it in place.
///
/// Each strip's `blend_mode` and `influence` determines how its values
/// compose with the style's current state.
pub fn apply_animation_to_style(style: &mut StyleResult, strips: &[EvaluatedStrip]) {
    for strip in strips {
        let influence = strip.influence.clamp(0.0, 1.0);
        for (path, value) in &strip.values {
            apply_scalar(style, path, *value, strip.blend_mode, influence);
        }
    }
}

fn apply_scalar(
    style: &mut StyleResult,
    path: &PropertyPath,
    value: f64,
    mode: BlendMode,
    influence: f64,
) {
    match path {
        PropertyPath::FontSize => {
            set_scalar(&mut style.font_size, value, mode, influence);
        }
        PropertyPath::Bold => {
            let v = value >= 0.5;
            set_bool(&mut style.bold, v, mode);
        }
        PropertyPath::Italic => {
            let v = value >= 0.5;
            set_bool(&mut style.italic, v, mode);
        }
        PropertyPath::Underline => {
            let v = value >= 0.5;
            set_bool(&mut style.underline, v, mode);
        }
        PropertyPath::Strikeout => {
            let v = value >= 0.5;
            set_bool(&mut style.strikeout, v, mode);
        }

        PropertyPath::ColorPrimaryR => color_component(
            &mut style.color_primary,
            |c| &mut c.red,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorPrimaryG => color_component(
            &mut style.color_primary,
            |c| &mut c.green,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorPrimaryB => color_component(
            &mut style.color_primary,
            |c| &mut c.blue,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorPrimaryA => color_component(
            &mut style.color_primary,
            |c| &mut c.alpha,
            value as f32,
            mode,
            influence,
        ),

        PropertyPath::ColorSecondaryR => color_component(
            &mut style.color_secondary,
            |c| &mut c.red,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorSecondaryG => color_component(
            &mut style.color_secondary,
            |c| &mut c.green,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorSecondaryB => color_component(
            &mut style.color_secondary,
            |c| &mut c.blue,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorSecondaryA => color_component(
            &mut style.color_secondary,
            |c| &mut c.alpha,
            value as f32,
            mode,
            influence,
        ),

        PropertyPath::ColorOutlineR => color_component(
            &mut style.color_outline,
            |c| &mut c.red,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorOutlineG => color_component(
            &mut style.color_outline,
            |c| &mut c.green,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorOutlineB => color_component(
            &mut style.color_outline,
            |c| &mut c.blue,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorOutlineA => color_component(
            &mut style.color_outline,
            |c| &mut c.alpha,
            value as f32,
            mode,
            influence,
        ),

        PropertyPath::ColorBackR => color_component(
            &mut style.color_back,
            |c| &mut c.red,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorBackG => color_component(
            &mut style.color_back,
            |c| &mut c.green,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorBackB => color_component(
            &mut style.color_back,
            |c| &mut c.blue,
            value as f32,
            mode,
            influence,
        ),
        PropertyPath::ColorBackA => color_component(
            &mut style.color_back,
            |c| &mut c.alpha,
            value as f32,
            mode,
            influence,
        ),

        PropertyPath::BorderX => vec2_component(&mut style.border, 0, value, mode, influence),
        PropertyPath::BorderY => vec2_component(&mut style.border, 1, value, mode, influence),
        PropertyPath::ShadowX => vec2_component(&mut style.shadow, 0, value, mode, influence),
        PropertyPath::ShadowY => vec2_component(&mut style.shadow, 1, value, mode, influence),
        PropertyPath::FontScaleX => {
            vec2_component(&mut style.font_scale, 0, value, mode, influence);
        }
        PropertyPath::FontScaleY => {
            vec2_component(&mut style.font_scale, 1, value, mode, influence);
        }
        PropertyPath::FontRotationX => {
            vec3_component(&mut style.font_rotation, 0, value, mode, influence);
        }
        PropertyPath::FontRotationY => {
            vec3_component(&mut style.font_rotation, 1, value, mode, influence);
        }
        PropertyPath::FontRotationZ => {
            vec3_component(&mut style.font_rotation, 2, value, mode, influence);
        }
        PropertyPath::FactorX => vec2_component(&mut style.factor, 0, value, mode, influence),
        PropertyPath::FactorY => vec2_component(&mut style.factor, 1, value, mode, influence),
        PropertyPath::Blur => {
            set_scalar(&mut style.blur, value, mode, influence);
        }
        PropertyPath::BlurEdges => {
            let v = value.round() as u32;
            set_scalar_u32(&mut style.blur_edges, v, mode, influence);
        }
        PropertyPath::PositionX => {
            vec2_component(&mut style.position, 0, value, mode, influence);
        }
        PropertyPath::PositionY => {
            vec2_component(&mut style.position, 1, value, mode, influence);
        }
        PropertyPath::PositionRatioX => {
            vec2_component(&mut style.position_ratio, 0, value, mode, influence);
        }
        PropertyPath::PositionRatioY => {
            vec2_component(&mut style.position_ratio, 1, value, mode, influence);
        }
        PropertyPath::OriginX => vec2_component(&mut style.origin, 0, value, mode, influence),
        PropertyPath::OriginY => vec2_component(&mut style.origin, 1, value, mode, influence),
        PropertyPath::ClipRectX1 => {
            clip_rect_component(&mut style.clip_rect, 0, value, mode, influence);
        }
        PropertyPath::ClipRectY1 => {
            clip_rect_component(&mut style.clip_rect, 1, value, mode, influence);
        }
        PropertyPath::ClipRectX2 => {
            clip_rect_component(&mut style.clip_rect, 2, value, mode, influence);
        }
        PropertyPath::ClipRectY2 => {
            clip_rect_component(&mut style.clip_rect, 3, value, mode, influence);
        }
    }
}

// ── Component helpers ─────────────────────────────────────────────────────

fn set_scalar(slot: &mut Option<f64>, value: f64, mode: BlendMode, influence: f64) {
    let current = slot.unwrap_or(0.0);
    *slot = Some(blend_f64(current, value, mode, influence));
}

fn blend_f64(base: f64, value: f64, mode: BlendMode, influence: f64) -> f64 {
    match mode {
        BlendMode::Replace => base + (value - base) * influence,
        BlendMode::Add => base + value * influence,
        BlendMode::Subtract => base - value * influence,
        BlendMode::Multiply => base * (1.0 + (value - 1.0) * influence),
        BlendMode::Combine => base * (1.0 - influence) + value * influence,
    }
}

fn set_scalar_u32(slot: &mut Option<u32>, value: u32, mode: BlendMode, influence: f64) {
    let cur = slot.unwrap_or(0) as f64;
    let v = blend_f64(cur, value as f64, mode, influence);
    *slot = Some(v.round() as u32);
}

fn set_bool(slot: &mut Option<bool>, value: bool, mode: BlendMode) {
    match mode {
        BlendMode::Replace => *slot = Some(value),
        BlendMode::Add | BlendMode::Subtract | BlendMode::Multiply | BlendMode::Combine => {}
    }
}

fn color_component(
    color: &mut Option<Srgba>,
    accessor: fn(&mut Srgba) -> &mut f32,
    value: f32,
    mode: BlendMode,
    influence: f64,
) {
    let c = color.get_or_insert_with(Srgba::default);
    let cur = *accessor(c) as f64;
    let next = blend_f64(cur, value as f64, mode, influence);
    *accessor(c) = next as f32;
}

fn vec2_component(
    v: &mut Option<crate::node_graph::Vec2>,
    idx: usize,
    value: f64,
    mode: BlendMode,
    influence: f64,
) {
    let v = v.get_or_insert_with(crate::node_graph::Vec2::default);
    let cur = if idx == 0 { v.x } else { v.y };
    let next = blend_f64(cur, value, mode, influence);
    if idx == 0 {
        v.x = next;
    } else {
        v.y = next;
    }
}

fn vec3_component(
    v: &mut Option<crate::node_graph::Vec3>,
    idx: usize,
    value: f64,
    mode: BlendMode,
    influence: f64,
) {
    let v = v.get_or_insert_with(crate::node_graph::Vec3::default);
    let cur = [v.x, v.y, v.z][idx];
    let next = blend_f64(cur, value, mode, influence);
    match idx {
        0 => v.x = next,
        1 => v.y = next,
        _ => v.z = next,
    }
}

fn clip_rect_component(
    r: &mut Option<crate::node_graph::ClipRect>,
    idx: usize,
    value: f64,
    mode: BlendMode,
    influence: f64,
) {
    let r = r.get_or_insert_with(crate::node_graph::ClipRect::default);
    let cur = [r.x1, r.y1, r.x2, r.y2][idx];
    let next = blend_f64(cur, value, mode, influence);
    match idx {
        0 => r.x1 = next,
        1 => r.y1 = next,
        2 => r.x2 = next,
        _ => r.y2 = next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn replace_mode_overrides() {
        let mut style = StyleResult::default();
        let strips = vec![EvaluatedStrip {
            blend_mode: BlendMode::Replace,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 24.0);
                m.insert(PropertyPath::Bold, 1.0);
                m
            },
        }];
        apply_animation_to_style(&mut style, &strips);
        assert_eq!(style.font_size, Some(24.0));
        assert_eq!(style.bold, Some(true));
    }

    #[test]
    fn add_mode_stacks_value() {
        let mut style = StyleResult {
            font_size: Some(12.0),
            ..StyleResult::default()
        };
        let strips = vec![EvaluatedStrip {
            blend_mode: BlendMode::Add,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 4.0);
                m
            },
        }];
        apply_animation_to_style(&mut style, &strips);
        assert_eq!(style.font_size, Some(16.0));
    }

    #[test]
    fn subtract_mode_reduces_value() {
        let mut style = StyleResult {
            font_size: Some(16.0),
            ..StyleResult::default()
        };
        let strips = vec![EvaluatedStrip {
            blend_mode: BlendMode::Subtract,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 4.0);
                m
            },
        }];
        apply_animation_to_style(&mut style, &strips);
        assert_eq!(style.font_size, Some(12.0));
    }

    #[test]
    fn multiply_mode_scales_value() {
        let mut style = StyleResult {
            font_size: Some(10.0),
            ..StyleResult::default()
        };
        let strips = vec![EvaluatedStrip {
            blend_mode: BlendMode::Multiply,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 2.0);
                m
            },
        }];
        apply_animation_to_style(&mut style, &strips);
        // base * (1 + (2-1)*1) = 10 * 2 = 20
        assert!((style.font_size.unwrap() - 20.0).abs() < 1e-9);
    }
}
