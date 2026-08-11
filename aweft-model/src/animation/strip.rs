use serde::{Deserialize, Serialize};
use std::f64;

use crate::animation::BlendMode;
use crate::animation::fcurve::{FCurve, evaluate_fcurve};
use crate::animation::modifier::FModifierInstance;
use crate::ids::ActionId;

// ── Strip placement ───────────────────────────────────────────────────────

/// How a strip is positioned relative to the target event duration.
///
/// The "event time" runs from `0.0` to `target_duration`. Strip placement
/// maps this event time to a `[strip_start, strip_end]` interval that defines
/// where the strip lives on the event timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StripPlacement {
    /// The strip covers the full event duration [0, target_duration].
    Full,
    /// Fixed absolute start and end times (seconds from event start).
    /// 绝对时间，end 不是相对总时长的
    Absolute { start: f64, end: f64 },
    /// Start and end as fractions of target_duration.
    /// E.g. `Relative { start_frac: 0.1, end_frac: 0.9 }` means 10%–90%.
    Relative { start_frac: f64, end_frac: f64 },
    /// Fixed padding from start and end, middle stretches proportionally.
    /// E.g. `Padded { pad_start: 0.2, pad_end: 0.2 }` means the strip
    /// starts at 0.2s from event start and ends 0.2s before event end.
    Padded { pad_start: f64, pad_end: f64 },
}

impl Default for StripPlacement {
    fn default() -> Self {
        Self::Full
    }
}

impl StripPlacement {
    /// Compute `(strip_start, strip_end)` given a `target_duration`.
    pub fn resolve(&self, target_duration: f64) -> (f64, f64) {
        let dur = target_duration.max(1e-9);
        match *self {
            Self::Full => (0.0, dur),
            Self::Absolute { start, end } => (start, end),
            Self::Relative {
                start_frac,
                end_frac,
            } => (
                start_frac.clamp(0.0, 1.0) * dur,
                end_frac.clamp(0.0, 1.0) * dur,
            ),
            Self::Padded { pad_start, pad_end } => {
                let inner = dur - pad_start - pad_end;
                if inner <= 0.0 {
                    (pad_start, pad_start)
                } else {
                    (pad_start, dur - pad_end)
                }
            }
        }
    }
}

// ── Extrapolation mode ────────────────────────────────────────────────────

/// What happens when the event time falls outside a strip's [start, end] range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtendMode {
    /// Hold the first/last frame value. The strip continues contributing.
    Hold,
    /// Hold only the last frame value (after the strip ends). Before start = no contribution.
    HoldForward,
    /// No contribution outside the strip's range.
    Nothing,
}

impl Default for ExtendMode {
    fn default() -> Self {
        Self::Hold
    }
}

// ── Strip type ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StripType {
    /// References an Action and plays its FCurves.
    Action {
        action: ActionId,
        #[serde(default)]
        actstart: f64,
        #[serde(default)]
        actend: f64,
    },
    Transition,
    Meta {
        strips: Vec<NlaStrip>,
    },
}

impl Default for StripType {
    fn default() -> Self {
        Self::Action {
            action: ActionId::default(),
            actstart: 0.0,
            actend: 0.0,
        }
    }
}

// ── NlaStrip ──────────────────────────────────────────────────────────────

/// A single NLA strip: a positioned, timed, and blended animation segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NlaStrip {
    pub name: String,

    /// How this strip is positioned relative to event duration.
    #[serde(default)]
    pub placement: StripPlacement,

    /// Blend mode for how this strip combines with the current value stack.
    #[serde(default)]
    pub blend_mode: BlendMode,

    /// Extrapolation behavior outside the strip's [start, end] range.
    #[serde(default)]
    pub extend_mode: ExtendMode,

    /// If true, the strip is ignored entirely.
    #[serde(default)]
    pub muted: bool,

    /// If true, the clip_time is wrapped cyclically into [actstart, actend).
    #[serde(default)]
    pub use_time_cyclic: bool,

    /// If true, bypass all time mapping (scale/repeat/reverse/pingpong/actstart/actend).
    /// The clip_time is set directly to the raw evaluation time.
    #[serde(default)]
    pub no_time_map: bool,

    /// Control curves that dynamically override strip properties.
    /// When `Some`, the curve drives the property; when `None`, a sensible default is used
    /// (1.0 for influence, 0.0 for blend_in/blend_out, automatic time mapping for clip_time).
    #[serde(default)]
    pub control_curves: StripControlCurves,

    /// Modifier stack applied to this strip's action F-Curves.
    #[serde(default)]
    pub modifiers: Vec<FModifierInstance>,

    /// The type of strip (clip, transition, meta).
    #[serde(default)]
    pub strip_type: StripType,
}

impl NlaStrip {
    pub fn new_action(name: impl Into<String>, action: ActionId) -> Self {
        Self {
            name: name.into(),
            placement: StripPlacement::Full,
            blend_mode: BlendMode::default(),
            extend_mode: ExtendMode::Hold,
            muted: false,
            use_time_cyclic: false,
            no_time_map: false,
            control_curves: StripControlCurves::default(),
            modifiers: Vec::new(),
            strip_type: StripType::Action {
                action,
                actstart: 0.0,
                actend: 0.0,
            },
        }
    }

    /// Get the action ID referenced by an Action strip.
    pub fn action_id(&self) -> Option<ActionId> {
        match &self.strip_type {
            StripType::Action { action, .. } => Some(*action),
            _ => None,
        }
    }

    /// Get the action time range (actstart, actend).
    /// Returns `None` for non-Action strips.
    /// If actend <= actstart, defaults to actstart + 1.0.
    pub fn action_range(&self) -> Option<(f64, f64)> {
        match &self.strip_type {
            StripType::Action {
                actstart, actend, ..
            } => {
                let astart = *actstart;
                let aend = if *actend <= astart {
                    astart + 1.0
                } else {
                    *actend
                };
                Some((astart, aend))
            }
            _ => None,
        }
    }
}

// ── StripControlCurves ────────────────────────────────────────────────────

/// Strip-level F-Curves that dynamically override strip properties.
///
/// When `Some`, the curve is evaluated at the strip-local time to drive
/// the corresponding property. When `None`, the static field value is used.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StripControlCurves {
    pub influence: Option<FCurve>,
    // 需要增加一些预设曲线来方便用户（它是与Action时间的关系）
    // 重复（直角三角形锯齿），倒放（反向曲线），PingPong（锯齿）
    pub clip_time: Option<FCurve>,
}

// ── NlaTrack ──────────────────────────────────────────────────────────────

/// A single NLA track containing an ordered list of strips.
///
/// Strips within a track are ordered by their time placement (not necessarily
/// in index order — the evaluation engine resolves the active strip at each time).
// TODO: NOTE: 或许当中的时间、局部文字索引可以下拉菜单改为外部输入（多个时间对），这样可能就可以在应用图当中建立节点来实现逐字的卡拉OK动画（每个字的时间点都是播放一遍动画）
// 但是估计 converter 也需要依字拟合了
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NlaTrack {
    pub name: String,
    /// If true, the entire track is muted.
    #[serde(default)]
    pub muted: bool,
    /// If true, only this track contributes (solo).
    #[serde(default)]
    pub solo: bool,
    /// Strips in this track, ordered by time position.
    #[serde(default)]
    pub strips: Vec<NlaStrip>,
}

impl NlaTrack {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            muted: false,
            solo: false,
            strips: Vec::new(),
        }
    }
}

// ── Evaluation result types ───────────────────────────────────────────────

/// The result of finding the active strip within a track at a given time.
#[derive(Debug, Clone)]
pub struct ActiveStrip {
    /// The resolved strip.
    pub strip: NlaStrip,
    /// The time within the strip's action to sample at.
    pub clip_time: f64,
    /// Pre-computed influence (including blendin/blendout/control curves).
    pub influence: f64,
    /// The strip's absolute time bounds (after placement resolution).
    pub strip_start: f64,
    pub strip_end: f64,
}

// ── Compute influence ─────────────────────────────────────────────────────
pub fn compute_strip_influence(
    strip: &NlaStrip,
    evaltime: f64,
    strip_start: f64,
    strip_end: f64,
) -> f64 {
    if strip.muted {
        return 0.0;
    }

    let sdur = (strip_end - strip_start).max(1e-9);
    let ct = (evaltime - strip_start).clamp(0.0, sdur);

    strip
        .control_curves
        .influence
        .as_ref()
        .map(|curve| evaluate_fcurve(curve, ct))
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

// ── Strip finding ────────────────────────────────────────────────────────

/// Find the active strip in a track at the given event time.
///
/// Returns `None` if no strip is active (all are out of range and extendmode
/// doesn't cover the gap).
pub fn find_active_strip(
    track: &NlaTrack,
    evaltime: f64,
    target_duration: f64,
) -> Option<ActiveStrip> {
    if track.muted {
        return None;
    }

    // Resolve all strip placements
    let resolved: Vec<(&NlaStrip, f64, f64)> = track
        .strips
        .iter()
        .map(|s| {
            let (start, end) = s.placement.resolve(target_duration);
            (s, start, end)
        })
        .collect();

    // Sort by start time for gap detection
    let mut sorted: Vec<usize> = (0..resolved.len()).collect();
    sorted.sort_by(|&a, &b| {
        resolved[a]
            .1
            .partial_cmp(&resolved[b].1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Find the strip at evaltime
    for &idx in &sorted {
        let (strip, start, end) = resolved[idx];
        if strip.muted {
            continue;
        }
        if evaltime >= start && evaltime <= end {
            // We're inside this strip's range
            return Some(build_active_strip(strip, evaltime, start, end));
        }
    }

    // No overlapping strip found — check extendmode of adjacent strips
    // Before the first strip
    if let Some(&idx) = sorted.first() {
        let (strip, start, _end) = resolved[idx];
        if evaltime < start && strip.extend_mode == ExtendMode::Hold {
            return Some(build_active_strip_clamped(strip, start, start, start));
        }
    }

    // After the last strip
    if let Some(&idx) = sorted.last() {
        let (strip, start, end) = resolved[idx];
        if evaltime > end {
            return match strip.extend_mode {
                ExtendMode::Hold => Some(build_active_strip_clamped(strip, end, start, end)),
                ExtendMode::HoldForward => Some(build_active_strip_clamped(strip, end, start, end)),
                ExtendMode::Nothing => None,
            };
        }
    }

    // Between strips: look at the previous strip's extendmode
    for window in sorted.windows(2) {
        let (prev, _, prev_end) = resolved[window[0]];
        let next_start = resolved[window[1]].1;
        if evaltime > prev_end && evaltime < next_start {
            // Gap — check prev strip's extendmode
            return match prev.extend_mode {
                ExtendMode::Hold => Some(build_active_strip_clamped(
                    prev, prev_end, prev_end, prev_end,
                )),
                ExtendMode::HoldForward => Some(build_active_strip_clamped(
                    prev, prev_end, prev_end, prev_end,
                )),
                ExtendMode::Nothing => None,
            };
        }
    }

    None
}

fn build_active_strip(
    strip: &NlaStrip,
    evaltime: f64,
    strip_start: f64,
    strip_end: f64,
) -> ActiveStrip {
    build_active_strip_inner(strip, evaltime, strip_start, strip_end, false)
}

fn build_active_strip_clamped(
    strip: &NlaStrip,
    evaltime: f64,
    strip_start: f64,
    strip_end: f64,
) -> ActiveStrip {
    build_active_strip_inner(strip, evaltime, strip_start, strip_end, true)
}

fn build_active_strip_inner(
    strip: &NlaStrip,
    evaltime: f64,
    strip_start: f64,
    strip_end: f64,
    is_clamped_to_boundary: bool,
) -> ActiveStrip {
    let influence = compute_strip_influence(strip, evaltime, strip_start, strip_end);
    let (actstart, actend) = strip.action_range().unwrap_or((0.0, 1.0));
    let actlength = (actend - actstart).max(1e-9);
    let dur = (strip_end - strip_start).max(1e-9);

    let clip_time = if strip.no_time_map {
        evaltime
    } else if let Some(ref curve) = strip.control_curves.clip_time {
        let ct = (evaltime - strip_start).clamp(0.0, dur);
        let time = evaluate_fcurve(curve, ct);
        if matches!(strip.strip_type, StripType::Meta { .. }) {
            time.clamp(0.0, 1.0)
        } else {
            time
        }
    } else if matches!(strip.strip_type, StripType::Meta { .. }) {
        let t = ((evaltime - strip_start) / dur).clamp(0.0, 1.0);
        t
    } else if is_clamped_to_boundary && evaltime <= strip_start {
        actstart
    } else if is_clamped_to_boundary && evaltime >= strip_end {
        actend
    } else {
        actstart + ((evaltime - strip_start) / dur) * actlength
    };

    let clip_time = if !matches!(strip.strip_type, StripType::Meta { .. }) {
        apply_cyclic(clip_time, actstart, actlength, strip.use_time_cyclic)
    } else {
        clip_time
    };

    ActiveStrip {
        strip: strip.clone(),
        clip_time,
        influence,
        strip_start,
        strip_end,
    }
}

/// Apply cyclic wrapping. Wraps `time` into [actstart, actend) when enabled.
fn apply_cyclic(time: f64, actstart: f64, actlength: f64, cyclic: bool) -> f64 {
    if cyclic {
        ((time - actstart) % actlength + actlength) % actlength + actstart
    } else {
        time
    }
}

// ── Transition evaluation ─────────────────────────────────────────────────

/// Evaluate a transition, returning a normalized 0..1 factor.
///
/// Transition direction is encoded in the parent context: the caller
/// decides whether prev→next or next→prev by how prev/next are sampled.
pub fn evaluate_transition(
    _strip: &NlaStrip,
    evaltime: f64,
    strip_start: f64,
    strip_end: f64,
) -> f64 {
    let dur = (strip_end - strip_start).max(1e-9);
    ((evaltime - strip_start) / dur).clamp(0.0, 1.0)
}
