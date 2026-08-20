use std::collections::HashMap;

use crate::animation::anim_data::{AnimData, BlendMode, PropertyPath};
use crate::animation::strip::{
    compute_strip_influence, evaluate_transition, find_active_strip,
    ActiveStrip, ExtendMode, NlaStrip, StripType,
};
use crate::ids::ActionId;

/// Per-strip evaluation result before blending.
#[derive(Debug, Clone)]
pub struct EvaluatedStrip {
    pub blend_mode: BlendMode,
    pub influence: f64,
    pub values: HashMap<PropertyPath, f64>,
}

/// An action provider: given an ActionId, return the Action's F-Curves
/// for sampling. This is a function (or trait) to avoid tying the evaluation
/// engine to a specific storage sequence.
pub trait ActionProvider {
    /// Return Some(iter of (property, FCurve)) or None if action not found.
    /// The returned box exists to allow dynamic dispatch.
    fn get_fcurves(
        &self,
        action_id: ActionId,
    ) -> Option<Vec<(PropertyPath, crate::animation::fcurve::FCurve)>>;
}

/// Sample an entire `AnimData` at the given event time and duration.
///
/// Returns a flat list of evaluated strips (ordered by track, then by strip
/// position within track). Each strip carries its own blend_mode and influence.
///
/// This implementation supports:
/// - Action strips (sample action FCurves)
/// - Transition strips (interpolate between adjacent strip values)
/// - Meta strips (recursive evaluation)
/// - Dynamic influence/time via strip-level control curves
/// - Strip-level modifier stacks
pub fn sample_anim_data(
    anim_data: &AnimData,
    anim_time: f64,
    target_duration: f64,
    action_provider: &dyn ActionProvider,
) -> Vec<EvaluatedStrip> {
    let mut results = Vec::new();

    let has_solo = anim_data.tracks.iter().any(|t| t.solo);

    for (track_idx, track) in anim_data.tracks.iter().enumerate() {
        if track.muted || (has_solo && !track.solo) {
            continue;
        }

        // Find the active strip in this track
        if let Some(active) = find_active_strip(track, anim_time, target_duration) {
            let strip_vals = evaluate_strip(
                &active,
                anim_time,
                target_duration,
                action_provider,
                track_idx,
                &anim_data.tracks,
            );
            // Skip if no values or influence is zero
            if active.influence > 0.0 && !strip_vals.is_empty() {
                results.push(EvaluatedStrip {
                    blend_mode: active.strip.blend_mode,
                    influence: active.influence,
                    values: strip_vals,
                });
            }
        }
    }

    // 基础编辑 Action 叠加在所有 NLA 轨道之上（优先级最高），按绝对时间求值，
    // 保证用户编辑时总能看得到自己的改动。
    // 临时编辑期间不叠加——此时编辑的就是 NLA 里片断引用的 Action，改动已
    // 由 NLA 求值反映。
    if !anim_data.is_temporary_editing() {
        if let Some(base) = evaluate_base_action(anim_data, anim_time, action_provider) {
            results.push(base);
        }
    }

    results
}

/// 求值基础编辑 Action：按"绝对时间"采样。
///
/// 基础层以 `Replace` + 满 influence 混入，且叠加在所有 NLA 轨道**之上**
/// （最后混入），因此会覆盖 NLA 的求值结果——用户编辑时看到的就是
/// `base_action` 的当前值。仅当处于非临时编辑模式且有 `base_action` 时调用。
fn evaluate_base_action(
    anim_data: &AnimData,
    anim_time: f64,
    action_provider: &dyn ActionProvider,
) -> Option<EvaluatedStrip> {
    let action_id = anim_data.base_action()?;

    let fcurves = action_provider.get_fcurves(action_id)?;
    let mut values = HashMap::new();
    for (prop, fcurve) in &fcurves {
        let value = evaluate_fcurve_with_extra_modifiers(fcurve, &[], anim_time);
        values.insert(prop.clone(), value);
    }
    if values.is_empty() {
        return None;
    }
    Some(EvaluatedStrip {
        blend_mode: BlendMode::Replace,
        influence: 1.0,
        values,
    })
}

/// Evaluate a single active strip, returning sampled property→value.
fn evaluate_strip(
    active: &ActiveStrip,
    evaltime: f64,
    target_duration: f64,
    action_provider: &dyn ActionProvider,
    track_idx: usize,
    all_tracks: &[crate::animation::strip::NlaTrack],
) -> HashMap<PropertyPath, f64> {
    match &active.strip.strip_type {
StripType::Action { action, actstart: _, actend: _ } => {
            evaluate_action_strip(active, *action, action_provider)
        }
        StripType::Transition => {
            evaluate_transition_strip(active, evaltime, target_duration, action_provider, track_idx, all_tracks)
        }
        StripType::Meta { strips } => {
            evaluate_meta_strip(active, evaltime, strips, action_provider)
        }
    }
}

/// Evaluate an Action strip: sample the referenced action at `clip_time`.
fn evaluate_action_strip(
    active: &ActiveStrip,
    action_id: ActionId,
    action_provider: &dyn ActionProvider,
) -> HashMap<PropertyPath, f64> {
    let Some(fcurves) = action_provider.get_fcurves(action_id) else {
        return HashMap::new();
    };

    let clip_time = active.clip_time;

    let mut values = HashMap::new();
    for (prop, fcurve) in &fcurves {
        let value =
            evaluate_fcurve_with_extra_modifiers(fcurve, &active.strip.modifiers, clip_time);
        values.insert(prop.clone(), value);
    }
    values
}

/// Evaluate a Transition strip: interpolate between the two adjacent strips
/// in the same track using the transition's own blend_mode (Fix 15).
fn evaluate_transition_strip(
    active: &ActiveStrip,
    evaltime: f64,
    target_duration: f64,
    action_provider: &dyn ActionProvider,
    track_idx: usize,
    all_tracks: &[crate::animation::strip::NlaTrack],
) -> HashMap<PropertyPath, f64> {
    let track = &all_tracks[track_idx];
    // Sort by resolved start time for neighbor detection
    let mut indexed: Vec<(usize, &NlaStrip, f64, f64)> = track
        .strips
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (st, en) = s.placement.resolve(target_duration);
            (i, s, st, en)
        })
        .collect();
    indexed.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // Find the transition strip's index in the sorted list
    let tx_pos = indexed.iter().position(|(_, s, _, _)| {
        matches!(s.strip_type, StripType::Transition) && s.name == active.strip.name
    });

    let mut prev_vals = HashMap::new();
    let mut next_vals = HashMap::new();

    if let Some(tx_idx) = tx_pos {
        // Previous neighbor: skip other transitions, find nearest non-transition
        for prev in (0..tx_idx).rev() {
            let (_, prev_s, prev_st, prev_en) = &indexed[prev];
            if let Some(prev_action) = prev_s.action_id() {
                let prev_active = ActiveStrip {
                    strip: (*prev_s).clone(),
                    clip_time: prev_s.action_range().unwrap().1.max(prev_s.action_range().unwrap().0),
                    influence: 1.0,
                    strip_start: *prev_st,
                    strip_end: *prev_en,
                };
                prev_vals = evaluate_action_strip(&prev_active, prev_action, action_provider);
                break;
            }
        }

        // Next neighbor
        for next in (tx_idx + 1)..indexed.len() {
            let (_, next_s, next_st, next_en) = &indexed[next];
            if let Some(next_action) = next_s.action_id() {
                let next_active = ActiveStrip {
                    strip: (*next_s).clone(),
                    clip_time: next_s.action_range().unwrap().0,
                    influence: 1.0,
                    strip_start: *next_st,
                    strip_end: *next_en,
                };
                next_vals = evaluate_action_strip(&next_active, next_action, action_provider);
                break;
            }
        }
    }

    let t = evaluate_transition(
        &active.strip,
        evaltime,
        active.strip_start,
        active.strip_end,
    );

    let mut result = HashMap::new();
    let mut all_props: std::collections::HashSet<PropertyPath> = std::collections::HashSet::new();
    for k in prev_vals.keys().chain(next_vals.keys()) {
        all_props.insert(k.clone());
    }

    for prop in all_props {
        let prev_v = prev_vals.get(&prop).copied().unwrap_or(0.0);
        let next_v = next_vals.get(&prop).copied().unwrap_or(0.0);
        result.insert(prop, crate::animation::fcurve::lerp(prev_v, next_v, t));
    }

    result
}

/// Evaluate a Meta strip: map meta's normalized clip_time back to global time,
/// then recursively evaluate child strips with meta's modifier stack inherited.
fn evaluate_meta_strip(
    active: &ActiveStrip,
    _evaltime: f64,
    child_strips: &[NlaStrip],
    action_provider: &dyn ActionProvider,
) -> HashMap<PropertyPath, f64> {
    // Meta's clip_time is the normalized position [0, 1] within the meta strip.
    let meta_dur = (active.strip_end - active.strip_start).max(1e-9);
    let norm = active.clip_time;
    let meta_time = active.strip_start + norm * meta_dur;

    let mut result = HashMap::new();

    for child in child_strips {
        if child.muted {
            continue;
        }
        let (child_start, child_end) = child.placement.resolve(meta_dur);
        let child_evaltime = meta_time;

        if child_evaltime < child_start {
            match child.extend_mode {
                ExtendMode::Hold => {}
                ExtendMode::HoldForward => continue,
                ExtendMode::Nothing => continue,
            }
        } else if child_evaltime > child_end {
            match child.extend_mode {
                ExtendMode::Hold | ExtendMode::HoldForward => {}
                ExtendMode::Nothing => continue,
            }
        }

        let clamped_time = child_evaltime.clamp(child_start, child_end);
        let dur = (child_end - child_start).max(1e-9);
        let (actstart, actend) = child.action_range().unwrap_or((0.0, 1.0));
        let actlen = (actend - actstart).max(1e-9);

        let mut clip_time = if child.no_time_map {
            clamped_time
        } else if let Some(ref curve) = child.control_curves.clip_time {
            let ct = (clamped_time - child_start).clamp(0.0, dur);
            let time = crate::animation::fcurve::evaluate_fcurve(curve, ct);
            if matches!(child.strip_type, StripType::Meta { .. }) {
                time.clamp(0.0, 1.0)
            } else {
                time
            }
        } else if matches!(child.strip_type, StripType::Meta { .. }) {
            let t = ((clamped_time - child_start) / dur).clamp(0.0, 1.0);
            t
        } else {
            actstart + ((clamped_time - child_start) / dur) * actlen
        };
        if child.use_time_cyclic {
            clip_time = ((clip_time - actstart) % actlen + actlen) % actlen + actstart;
        }

        // Prepend meta's modifiers to child's (Fix 7)
        let mut child_with_meta_mods = child.clone();
        let mut combined_mods = active.strip.modifiers.clone();
        combined_mods.extend(child.modifiers.clone());
        child_with_meta_mods.modifiers = combined_mods;

        let child_active = ActiveStrip {
            strip: child_with_meta_mods,
            clip_time,
            influence: compute_strip_influence(child, clamped_time, child_start, child_end),
            strip_start: child_start,
            strip_end: child_end,
        };

        let child_vals = evaluate_strip_in_meta(
            &child_active, meta_dur, action_provider, child_strips,
        );
        for (prop, val) in child_vals {
            let entry = result.entry(prop).or_insert(0.0);
            let inf = child_active.influence;
            *entry = blend_scalar(*entry, val, child.blend_mode, inf);
        }
    }

    result
}

fn evaluate_strip_in_meta(
    active: &ActiveStrip,
    meta_dur: f64,
    action_provider: &dyn ActionProvider,
    sibling_strips: &[NlaStrip],
) -> HashMap<PropertyPath, f64> {
    match &active.strip.strip_type {
        StripType::Action { action, .. } => evaluate_action_strip(active, *action, action_provider),
        StripType::Meta { strips } => {
            evaluate_meta_strip(active, active.clip_time, strips, action_provider)
        }
        StripType::Transition => {
            evaluate_meta_transition(active, meta_dur, action_provider, sibling_strips)
        }
    }
}

/// Evaluate a Transition that lives inside a Meta strip.
///
/// Uses the same neighbor-finding strategy as `evaluate_transition_strip`:
/// resolve sibling placements, sort by start time, find prev/next
/// non-transition strips, sample them at boundary frames, interpolate.
fn evaluate_meta_transition(
    active: &ActiveStrip,
    meta_dur: f64,
    action_provider: &dyn ActionProvider,
    sibling_strips: &[NlaStrip],
) -> HashMap<PropertyPath, f64> {
    let meta_time = active.strip_start + active.clip_time * meta_dur;

    // Index siblings by resolved placement within the meta's duration
    let mut indexed: Vec<(usize, &NlaStrip, f64, f64)> = sibling_strips
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (st, en) = s.placement.resolve(meta_dur);
            (i, s, st, en)
        })
        .collect();
    indexed.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let tx_pos = indexed.iter().position(|(_, s, _, _)| {
        matches!(s.strip_type, StripType::Transition) && s.name == active.strip.name
    });

    let mut prev_vals = HashMap::new();
    let mut next_vals = HashMap::new();

        if let Some(tx_idx) = tx_pos {
        // Previous neighbor: walk backward, skip other transitions, find the
        // nearest non-transition strip and sample it at its end frame.
        for prev in (0..tx_idx).rev() {
            let (_, prev_s, prev_st, prev_en) = &indexed[prev];
            if let Some(prev_action) = prev_s.action_id() {
                let prev_active = ActiveStrip {
                    strip: (*prev_s).clone(),
                    clip_time: prev_s.action_range().unwrap().1.max(prev_s.action_range().unwrap().0),
                    influence: 1.0,
                    strip_start: *prev_st,
                    strip_end: *prev_en,
                };
                prev_vals = evaluate_action_strip(&prev_active, prev_action, action_provider);
                break;
            }
        }

        // Next neighbor: walk forward, skip transitions, find nearest
        // non-transition and sample it at its start frame.
        for next in (tx_idx + 1)..indexed.len() {
            let (_, next_s, next_st, next_en) = &indexed[next];
            if let Some(next_action) = next_s.action_id() {
                let next_active = ActiveStrip {
                    strip: (*next_s).clone(),
                    clip_time: next_s.action_range().unwrap().0,
                    influence: 1.0,
                    strip_start: *next_st,
                    strip_end: *next_en,
                };
                next_vals = evaluate_action_strip(&next_active, next_action, action_provider);
                break;
            }
        }
    }

    let t = evaluate_transition(
        &active.strip,
        meta_time,
        active.strip_start,
        active.strip_end,
    );

    let mut result = HashMap::new();
    let mut all_props: std::collections::HashSet<PropertyPath> = std::collections::HashSet::new();
    for k in prev_vals.keys().chain(next_vals.keys()) {
        all_props.insert(k.clone());
    }
    for prop in all_props {
        let prev_v = prev_vals.get(&prop).copied().unwrap_or(0.0);
        let next_v = next_vals.get(&prop).copied().unwrap_or(0.0);
        result.insert(prop, crate::animation::fcurve::lerp(prev_v, next_v, t));
    }
    result
}

// ── FCurve evaluation with extra modifiers ────────────────────────────────

fn evaluate_fcurve_with_extra_modifiers(
    fcurve: &crate::animation::fcurve::FCurve,
    extra_modifiers: &[crate::animation::modifier::FModifierInstance],
    clip_time: f64,
) -> f64 {
    // Apply both the FCurve's own modifiers and the strip's extra modifiers.
    // Strip modifiers are prepended (take effect first in the reverse time pass).
    if extra_modifiers.is_empty() {
        crate::animation::fcurve::evaluate_fcurve(fcurve, clip_time)
    } else {
        let mut combined = fcurve.clone();
        // Prepend strip modifiers (they act before the FCurve's own modifiers in reverse order)
        let mut all_mods = extra_modifiers.to_vec();
        all_mods.extend(combined.modifiers.clone());
        combined.modifiers = all_mods;
        crate::animation::fcurve::evaluate_fcurve(&combined, clip_time)
    }
}

// ── Blend scalar values ───────────────────────────────────────────────────

pub fn blend_scalar(base: f64, override_val: f64, mode: BlendMode, influence: f64) -> f64 {
    match mode {
        BlendMode::Replace => base + (override_val - base) * influence,
        BlendMode::Add => base + override_val * influence,
        BlendMode::Subtract => base - override_val * influence,
        BlendMode::Multiply => base * (1.0 + (override_val - 1.0) * influence),
        BlendMode::Combine => base * (1.0 - influence) + override_val * influence,
    }
}

/// Blend a list of evaluated strips into a single property→value map.
pub fn blend_strip_results(strips: &[EvaluatedStrip]) -> HashMap<PropertyPath, f64> {
    let mut result = HashMap::new();

    for strip in strips {
        for (path, &value) in &strip.values {
            let entry = result.entry(path.clone()).or_insert(0.0);
            *entry = blend_scalar(*entry, value, strip.blend_mode, strip.influence);
        }
    }

    result
}
