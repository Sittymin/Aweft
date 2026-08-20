//! 纯数学/构建函数：曲线段、控制柄标记、贝塞尔运算。
//! 不依赖 GraphEditorState，可在任何上下文调用。

use crate::bridge::types::{
    SlintBezierHandle, SlintCurveSegment, SlintHandleMarker, SlintInterpolation, SlintKeyframe,
    SlintSegmentKind,
};
use slint::Color;

/// 三次贝塞尔上参数 t 处的点。
pub(super) fn cubic_point(
    x0: f32,
    y0: f32,
    cx1: f32,
    cy1: f32,
    cx2: f32,
    cy2: f32,
    x1: f32,
    y1: f32,
    t: f32,
) -> (f32, f32) {
    let mt = 1.0 - t;
    let x = mt * mt * mt * x0 + 3.0 * mt * mt * t * cx1 + 3.0 * mt * t * t * cx2 + t * t * t * x1;
    let y = mt * mt * mt * y0 + 3.0 * mt * mt * t * cy1 + 3.0 * mt * t * t * cy2 + t * t * t * y1;
    (x, y)
}

/// 直线段：控制点取直线上 1/3、2/3 位置。
pub(super) fn straight(
    kind: SlintSegmentKind,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) -> (SlintSegmentKind, f32, f32, f32, f32, f32, f32) {
    (
        kind,
        x0 + (x1 - x0) / 3.0,
        y0 + (y1 - y0) / 3.0,
        x0 + (x1 - x0) * 2.0 / 3.0,
        y0 + (y1 - y0) * 2.0 / 3.0,
        x1,
        y1,
    )
}

/// 构建一个曲线段（相邻两个关键帧之间）。
pub(super) fn make_segment(
    curve: usize,
    color: Color,
    k0: &SlintKeyframe,
    k1: &SlintKeyframe,
) -> SlintCurveSegment {
    let (kind, mut cx1, cy1, mut cx2, cy2, x1, y1) = match k0.interpolation {
        SlintInterpolation::Bezier => {
            let c1 = (
                k0.time + k0.handle_out.time_offset,
                k0.value + k0.handle_out.value_offset,
            );
            let c2 = (
                k1.time + k1.handle_in.time_offset,
                k1.value + k1.handle_in.value_offset,
            );
            (
                SlintSegmentKind::Bezier,
                c1.0,
                c1.1,
                c2.0,
                c2.1,
                k1.time,
                k1.value,
            )
        }
        SlintInterpolation::Linear => straight(
            SlintSegmentKind::Linear,
            k0.time,
            k0.value,
            k1.time,
            k1.value,
        ),
        SlintInterpolation::Hold => {
            straight(SlintSegmentKind::Hold, k0.time, k0.value, k1.time, k0.value)
        }
    };
    let step_value = if k0.interpolation == SlintInterpolation::Hold {
        k1.value
    } else {
        0.0
    };
    if k0.time <= k1.time {
        cx1 = cx1.clamp(k0.time, k1.time);
        cx2 = cx2.clamp(k0.time, k1.time);
    }
    SlintCurveSegment {
        curve_index: curve as i32,
        kind,
        color,
        x0: k0.time,
        y0: k0.value,
        cx1,
        cy1,
        cx2,
        cy2,
        x1,
        y1,
        step_value,
    }
}

/// 构建一个控制柄标记。
pub(super) fn mk_marker(
    curve: usize,
    key: usize,
    color: Color,
    kf: &SlintKeyframe,
    handle_time: f32,
    handle_value: f32,
    is_in: bool,
    preview: bool,
    selected: bool,
) -> SlintHandleMarker {
    SlintHandleMarker {
        curve_index: curve as i32,
        keyframe_index: key as i32,
        is_in,
        color,
        key_time: kf.time,
        key_value: kf.value,
        handle_time,
        handle_value,
        preview,
        selected,
    }
}

/// 硬钳制：把控制柄偏移限制在「自己关键帧」一侧（不越过相邻关键帧）。
pub(super) fn clamp_handle_own_side(kf: &mut SlintKeyframe, is_in: bool) {
    if is_in {
        // 入柄：手柄时间 ≤ 关键帧时间（偏移 ≤ 0）
        kf.handle_in.time_offset = kf.handle_in.time_offset.min(0.0);
    } else {
        // 出柄：手柄时间 ≥ 关键帧时间（偏移 ≥ 0）
        kf.handle_out.time_offset = kf.handle_out.time_offset.max(0.0);
    }
}

/// 按模式约束对手柄（对齐/原点对称/自由）。
pub(super) fn apply_handle_mode(
    kf: &mut SlintKeyframe,
    adjusted_side: bool,
    mode: crate::bridge::types::SlintHandleDragMode,
) {
    let (t, v) = if adjusted_side {
        (kf.handle_in.time_offset, kf.handle_in.value_offset)
    } else {
        (kf.handle_out.time_offset, kf.handle_out.value_offset)
    };
    match mode {
        crate::bridge::types::SlintHandleDragMode::Aligned => {
            let nl = (t * t + v * v).sqrt();
            if nl > 1e-9 {
                let len = if adjusted_side {
                    (kf.handle_out.time_offset.powi(2) + kf.handle_out.value_offset.powi(2)).sqrt()
                } else {
                    (kf.handle_in.time_offset.powi(2) + kf.handle_in.value_offset.powi(2)).sqrt()
                };
                let new = SlintBezierHandle {
                    time_offset: -t / nl * len,
                    value_offset: -v / nl * len,
                };
                if adjusted_side {
                    kf.handle_out = new;
                } else {
                    kf.handle_in = new;
                }
            }
        }
        crate::bridge::types::SlintHandleDragMode::Symmetric => {
            let new = SlintBezierHandle {
                time_offset: -t,
                value_offset: -v,
            };
            if adjusted_side {
                kf.handle_out = new;
            } else {
                kf.handle_in = new;
            }
        }
        crate::bridge::types::SlintHandleDragMode::Free => {}
    }
}

/// 曲线段与框选矩形是否相交。
pub(super) fn segment_intersects_rect(
    seg: &SlintCurveSegment,
    t_lo: f32,
    t_hi: f32,
    v_lo: f32,
    v_hi: f32,
) -> bool {
    let x_min = seg.x0.max(t_lo);
    let x_max = seg.x1.min(t_hi);
    if x_max <= x_min {
        return false;
    }
    const SAMPLES: usize = 30;
    let y_at = |x: f32| -> f32 {
        let mut lo = 0.0f32;
        let mut hi = 1.0f32;
        for _ in 0..40 {
            let mid = (lo + hi) * 0.5;
            let (px, _) = cubic_point(
                seg.x0, seg.y0, seg.cx1, seg.cy1, seg.cx2, seg.cy2, seg.x1, seg.y1, mid,
            );
            if px < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (_, py) = cubic_point(
            seg.x0,
            seg.y0,
            seg.cx1,
            seg.cy1,
            seg.cx2,
            seg.cy2,
            seg.x1,
            seg.y1,
            (lo + hi) * 0.5,
        );
        py
    };
    let zone = |y: f32| -> i8 {
        if y < v_lo {
            -1
        } else if y <= v_hi {
            0
        } else {
            1
        }
    };
    let mut prev = zone(y_at(x_min));
    if prev == 0 {
        return true;
    }
    for i in 1..=SAMPLES {
        let x = x_min + (x_max - x_min) * (i as f32 / SAMPLES as f32);
        let z = zone(y_at(x));
        if z == 0 || z != prev {
            return true;
        }
        prev = z;
    }
    false
}
