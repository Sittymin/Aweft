use serde::{Deserialize, Serialize};

use crate::animation::keyframe::{BezierHandle, Extrapolation, Interpolation, Keyframe};
use crate::animation::modifier::{FModifierInstance, evaluate_modifiers};

/// A pure animation curve: keyframes + extrapolation + modifier stack.
///
/// Does not carry a property assignment — that lives in `ActionCurve`
/// for action F-Curves, or in `StripControlCurves` for strip controls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FCurve {
    /// Keyframes sorted by time. They are sorted at serialization; at runtime assume sorted.
    pub keyframes: Vec<Keyframe>,
    #[serde(default)]
    pub extrapolation: Extrapolation,
    /// Modifiers applied before value interpolation (time modifiers: Cycles, Stepped)
    /// and after interpolation (value modifiers: Noise, Envelope, Limits, Generator).
    #[serde(default)]
    pub modifiers: Vec<FModifierInstance>,
    /// If true, the output is rounded to the nearest integer after all processing.
    /// Corresponds to Blender's FCURVE_INT_VALUES flag.
    #[serde(default)]
    pub integer: bool,
}

impl Default for FCurve {
    fn default() -> Self {
        Self {
            keyframes: Vec::new(),
            extrapolation: Extrapolation::Constant,
            modifiers: Vec::new(),
            integer: false,
        }
    }
}

/// Sample a single FCurve at `eval_time`, producing a scalar value.
///
/// Pipeline:
///   1. Apply time-modifying modifiers in REVERSE order to get `devaltime`.
///   2. Sample base curve value at `devaltime` using interpolation/extrapolation.
///   3. Apply value-modifying modifiers in FORWARD order to get final value.
pub fn evaluate_fcurve(fcurve: &FCurve, eval_time: f64) -> f64 {
    // 将实际时间映射到动画区间当中
    let devaltime = evaluate_time_modifiers(&fcurve.modifiers, eval_time, fcurve);
    // 当前时间计算值
    let base_value = sample_normalized_curve(&fcurve.keyframes, fcurve.extrapolation, devaltime);
    let result = evaluate_value_modifiers(&fcurve.modifiers, devaltime, base_value, fcurve);
    if fcurve.integer {
        result.round()
    } else {
        result
    }
}

/// Apply time modifiers in reverse order (last→first on the stack).
/// 时间修改器是将外部的时间重新映射回动画时间区间
fn evaluate_time_modifiers(modifiers: &[FModifierInstance], evaltime: f64, fcurve: &FCurve) -> f64 {
    let mut time = evaltime;
    for m in modifiers.iter().rev() {
        time = evaluate_modifiers::evaluate_time(m, time, fcurve);
    }
    time
}

/// Apply value modifiers in forward order (first→last on the stack).
fn evaluate_value_modifiers(
    modifiers: &[FModifierInstance],
    evaltime: f64,
    mut value: f64,
    fcurve: &FCurve,
) -> f64 {
    for m in modifiers.iter() {
        value = evaluate_modifiers::evaluate_value(m, evaltime, value, fcurve);
    }
    value
}

// ── base curve sampling (interpolation + extrapolation) ────────────────────

fn sample_normalized_curve(keyframes: &[Keyframe], extrapolation: Extrapolation, time: f64) -> f64 {
    if keyframes.is_empty() {
        return 0.0;
    }

    let first = &keyframes[0];
    let last = &keyframes[keyframes.len() - 1];

    // Before first keyframe
    if time <= first.time {
        return match extrapolation {
            Extrapolation::Constant => first.value,
            Extrapolation::Linear => {
                if keyframes.len() >= 2 {
                    let second = &keyframes[1];
                    let slope = (second.value - first.value) / (second.time - first.time).max(1e-9);
                    first.value + slope * (time - first.time)
                } else {
                    first.value
                }
            }
        };
    }

    // After last keyframe
    if time >= last.time {
        return match extrapolation {
            Extrapolation::Constant => last.value,
            Extrapolation::Linear => {
                if keyframes.len() >= 2 {
                    let prev = &keyframes[keyframes.len() - 2];
                    let slope = (last.value - prev.value) / (last.time - prev.time).max(1e-9);
                    last.value + slope * (time - last.time)
                } else {
                    last.value
                }
            }
        };
    }

    // Within range: find bracketing keyframes
    let i = keyframes.partition_point(|k| k.time < time);
    // 当前时间的前后两个关键帧
    let k0 = &keyframes[i - 1];
    let k1 = &keyframes[i];

    match k0.interpolation {
        Interpolation::Hold => k0.value,
        Interpolation::Linear => {
            let t = (time - k0.time) / (k1.time - k0.time).max(1e-9);
            lerp(k0.value, k1.value, t.clamp(0.0, 1.0))
        }
        Interpolation::Bezier { handle_out, .. } => {
            let handle_in = match k1.interpolation {
                Interpolation::Bezier { handle_in, .. } => handle_in,
                _ => BezierHandle::default(),
            };
            let t = bezier_time_at_x(
                k0.time,
                k0.time + handle_out.time_offset,
                k1.time + handle_in.time_offset,
                k1.time,
                time,
            );
            cubic_bezier(
                k0.value,
                k0.value + handle_out.value_offset,
                k1.value + handle_in.value_offset,
                k1.value,
                t,
            )
        }
    }
}

// ── math helpers ───────────────────────────────────────────────────────────

// 按照 t 在 a ， b 当中插值
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub fn cubic_bezier(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

pub fn bezier_time_at_x(x0: f64, x1: f64, x2: f64, x3: f64, target_x: f64) -> f64 {
    let mut low = 0.0f64;
    let mut high = 1.0f64;
    for _ in 0..64 {
        let mid = (low + high) * 0.5;
        let x = cubic_bezier(x0, x1, x2, x3, mid);
        if x < target_x {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) * 0.5
}
