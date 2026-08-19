//! Rust ↔ Slint 边界转换：model 的 animation 类型 ↔ Slint 视图类型。

use aweft_model::animation::{
    BezierHandle, Interpolation as ModelInterpolation, Keyframe as ModelKeyframe,
};
use slint::SharedString;

use crate::bridge::types::{ContextMenuItem, SlintBezierHandle, SlintInterpolation, SlintKeyframe};

/// model `Keyframe` → `SlintKeyframe`（f64→f32；`Interpolation` → `SlintInterpolation`）。
pub fn model_key_to_slint(k: &ModelKeyframe) -> SlintKeyframe {
    let (handle_in, handle_out, interpolation) = match k.interpolation {
        ModelInterpolation::Bezier {
            handle_in,
            handle_out,
        } => (
            SlintBezierHandle {
                time_offset: handle_in.time_offset as f32,
                value_offset: handle_in.value_offset as f32,
            },
            SlintBezierHandle {
                time_offset: handle_out.time_offset as f32,
                value_offset: handle_out.value_offset as f32,
            },
            SlintInterpolation::Bezier,
        ),
        ModelInterpolation::Linear => (
            SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            SlintInterpolation::Linear,
        ),
        ModelInterpolation::Hold => (
            SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            SlintInterpolation::Hold,
        ),
    };
    SlintKeyframe {
        time: k.time as f32,
        value: k.value as f32,
        interpolation,
        handle_in,
        handle_out,
        selected: false,
    }
}

/// `SlintKeyframe` → model `Keyframe`（f32→f64；UI 选中态丢弃）。
pub fn slint_key_to_model(k: &SlintKeyframe) -> ModelKeyframe {
    let interpolation = match k.interpolation {
        SlintInterpolation::Bezier => ModelInterpolation::Bezier {
            handle_in: BezierHandle::new(
                k.handle_in.time_offset as f64,
                k.handle_in.value_offset as f64,
            ),
            handle_out: BezierHandle::new(
                k.handle_out.time_offset as f64,
                k.handle_out.value_offset as f64,
            ),
        },
        SlintInterpolation::Linear => ModelInterpolation::Linear,
        SlintInterpolation::Hold => ModelInterpolation::Hold,
    };
    ModelKeyframe {
        time: k.time as f64,
        value: k.value as f64,
        interpolation,
    }
}

/// 由 Action 名称列表派生下拉菜单项（无前导色点/无分隔线）。
pub fn build_action_menu_items(names: &[SharedString]) -> Vec<ContextMenuItem> {
    names
        .iter()
        .map(|name| ContextMenuItem {
            text: name.clone(),
            enabled: true,
            dot_color: slint::Color::from_argb_u8(0, 0, 0, 0),
            separator_before: false,
        })
        .collect()
}
