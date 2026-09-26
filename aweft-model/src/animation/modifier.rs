use serde::{Deserialize, Serialize};

use crate::animation::fcurve::FCurve;
use crate::animation::noise::perlin_fbm;

// ── Modifier envelope ─────────────────────────────────────────────────────

/// A single modifier instance, wrapping type-specific data with shared controls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FModifierInstance {
    pub data: FModifier,
    /// Whether to apply `influence` scaling.
    #[serde(default)]
    pub use_influence: bool,
    /// Blend weight 0..1.
    #[serde(default = "default_influence")]
    pub influence: f64,
    /// Restricted time range start (inclusive).
    #[serde(default)]
    pub range_start: Option<f64>,
    /// Restricted time range end (inclusive).
    #[serde(default)]
    pub range_end: Option<f64>,
    /// Fade-in duration from `range_start`.
    #[serde(default)]
    pub blend_in: f64,
    /// Fade-out duration before `range_end`.
    #[serde(default)]
    pub blend_out: f64,
    /// If true, modifier is skipped entirely.
    #[serde(default)]
    pub muted: bool,
}

fn default_influence() -> f64 {
    1.0
}

impl Default for FModifierInstance {
    fn default() -> Self {
        Self {
            data: FModifier::Cycles(FModCycles::default()),
            use_influence: false,
            influence: 1.0,
            range_start: None,
            range_end: None,
            blend_in: 0.0,
            blend_out: 0.0,
            muted: false,
        }
    }
}

impl FModifierInstance {
    /// Compute the effective influence at `time`, factoring in range restrictions
    /// and fade-in/fade-out.
    pub fn effective_influence(&self, time: f64) -> f64 {
        if self.muted {
            return 0.0;
        }

        let base = if self.use_influence {
            self.influence.clamp(0.0, 1.0)
        } else {
            1.0
        };

        if let (Some(start), Some(end)) = (self.range_start, self.range_end) {
            if time < start || time > end {
                return 0.0;
            }
            if start < end {
                if time < start + self.blend_in {
                    if self.blend_in > 0.0 {
                        return base * (time - start) / self.blend_in;
                    }
                }
                if time > end - self.blend_out {
                    if self.blend_out > 0.0 {
                        return base * (end - time) / self.blend_out;
                    }
                }
            }
        }

        base
    }
}

// ── Modifier types ────────────────────────────────────────────────────────

/// Complete set of F-Curve modifier types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FModifier {
    /// Cycle/repeat/mirror the keyframe range.
    Cycles(FModCycles),
    /// Add Perlin FBM noise to the value.
    Noise(FModNoise),
    /// Snap time to discrete steps.
    Stepped(FModStepped),
    /// Map values through envelope control points.
    Envelope(FModEnvelope),
    /// Clamp time and/or value.
    Limits(FModLimits),
    /// Generate output via polynomial.
    Generator(FModGenerator),
}

// ── Cycles ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CycleMode {
    #[default]
    None,
    Repeat,
    RepeatOffset,
    Mirror,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FModCycles {
    #[serde(default)]
    pub before_mode: CycleMode,
    #[serde(default)]
    pub after_mode: CycleMode,
    /// Max repeats before the first keyframe (0 = infinite).
    #[serde(default)]
    pub before_cycles: u16,
    /// Max repeats after the last keyframe (0 = infinite).
    #[serde(default)]
    pub after_cycles: u16,
}

impl Default for FModCycles {
    fn default() -> Self {
        Self {
            before_mode: CycleMode::None,
            after_mode: CycleMode::None,
            before_cycles: 0,
            after_cycles: 0,
        }
    }
}

// ── Noise ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseMod {
    Replace,
    #[default]
    Add,
    Subtract,
    Multiply,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FModNoise {
    /// Scale of the noise (larger = slower). 1.0 default.
    #[serde(default = "one")]
    pub size: f64,
    /// Amplitude. 1.0 default.
    #[serde(default = "one")]
    pub strength: f64,
    /// Phase offset. 0.0 default.
    #[serde(default)]
    pub phase: f64,
    /// Time offset for the noise. 0.0 default.
    #[serde(default)]
    pub offset: f64,
    /// Number of octaves. 0-4. Default 0 = single octave.
    #[serde(default)]
    pub depth: u8,
    /// How amplitude decreases per octave. Default 0.5.
    #[serde(default = "half")]
    pub roughness: f64,
    /// How frequency increases per octave. Default 2.0.
    #[serde(default = "two")]
    pub lacunarity: f64,
    /// How noise combines with the signal value.
    #[serde(default)]
    pub modification: NoiseMod,
}

fn half() -> f64 {
    0.5
}
fn one() -> f64 {
    1.0
}
fn two() -> f64 {
    2.0
}

impl Default for FModNoise {
    fn default() -> Self {
        Self {
            size: 1.0,
            strength: 1.0,
            phase: 0.0,
            offset: 0.0,
            depth: 0,
            roughness: 0.5,
            lacunarity: 2.0,
            modification: NoiseMod::Add,
        }
    }
}

// ── Stepped ───────────────────────────────────────────────────────────────

/// Flag for stepped modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteppedFlags {
    /// Don't apply before `start_frame`.
    #[serde(default)]
    pub no_before: bool,
    /// Don't apply after `end_frame`.
    #[serde(default)]
    pub no_after: bool,
}

impl Default for SteppedFlags {
    fn default() -> Self {
        Self {
            no_before: false,
            no_after: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FModStepped {
    /// Time interval between steps. Default 2.0.
    #[serde(default = "two")]
    pub step_size: f64,
    /// Phase offset. Default 0.0.
    #[serde(default)]
    pub offset: f64,
    /// Range start.
    #[serde(default)]
    pub start_frame: f64,
    /// Range end.
    #[serde(default)]
    pub end_frame: f64,
    #[serde(default)]
    pub flags: SteppedFlags,
}

impl Default for FModStepped {
    fn default() -> Self {
        Self {
            step_size: 2.0,
            offset: 0.0,
            start_frame: 0.0,
            end_frame: 0.0,
            flags: SteppedFlags::default(),
        }
    }
}

// ── Envelope ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnvelopePoint {
    pub time: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FModEnvelope {
    pub control_points: Vec<EnvelopePoint>,
    #[serde(default)]
    pub midval: f64,
    #[serde(default)]
    pub min: f64,
    #[serde(default)]
    pub max: f64,
}

impl Default for FModEnvelope {
    fn default() -> Self {
        Self {
            control_points: Vec::new(),
            midval: 0.0,
            min: 0.0,
            max: 1.0,
        }
    }
}

// ── Limits ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FModLimits {
    /// Min eval time.
    #[serde(default)]
    pub xmin: Option<f64>,
    /// Max eval time.
    #[serde(default)]
    pub xmax: Option<f64>,
    /// Min output value.
    #[serde(default)]
    pub ymin: Option<f64>,
    /// Max output value.
    #[serde(default)]
    pub ymax: Option<f64>,
}

impl Default for FModLimits {
    fn default() -> Self {
        Self {
            xmin: None,
            xmax: None,
            ymin: None,
            ymax: None,
        }
    }
}

// ── Generator ─────────────────────────────────────────────────────────────

/// Polynomial generator mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeneratorMode {
    /// `sum(coefficients[i] * t^i)`.
    #[default]
    Polynomial,
    /// `product(coefficients[2i] * t + coefficients[2i+1])`.
    Factorised,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FModGenerator {
    pub coefficients: Vec<f64>,
    #[serde(default)]
    pub mode: GeneratorMode,
    /// If true, add to the existing value; otherwise replace.
    #[serde(default)]
    pub additive: bool,
}

impl Default for FModGenerator {
    fn default() -> Self {
        Self {
            coefficients: Vec::new(),
            mode: GeneratorMode::Polynomial,
            additive: false,
        }
    }
}

// ── Modifier evaluation ───────────────────────────────────────────────────

pub mod evaluate_modifiers {
    use super::*;

    /// Apply a time modifier. Returns the new evaluation time.
    /// Called in REVERSE stack order.
    pub fn evaluate_time(m: &FModifierInstance, evaltime: f64, fcurve: &FCurve) -> f64 {
        if matches!(
            m.data,
            FModifier::Cycles(_) | FModifier::Stepped(_) | FModifier::Limits(_)
        ) {
            let inf = m.effective_influence(evaltime);
            if inf <= 1e-12 {
                return evaltime;
            }
            let new_time = match &m.data {
                FModifier::Cycles(c) => eval_cycles_time(c, evaltime, fcurve),
                FModifier::Stepped(s) => eval_stepped_time(s, evaltime),
                FModifier::Limits(l) => eval_limits_time(l, evaltime),
                _ => evaltime,
            };
            crate::animation::fcurve::lerp(evaltime, new_time, inf)
        } else {
            evaltime
        }
    }

    /// Apply a value modifier. Returns the new output value.
    /// Called in FORWARD stack order.
    pub fn evaluate_value(
        m: &FModifierInstance,
        evaltime: f64,
        cvalue: f64,
        fcurve: &FCurve,
    ) -> f64 {
        if matches!(
            m.data,
            FModifier::Noise(_)
                | FModifier::Envelope(_)
                | FModifier::Limits(_)
                | FModifier::Generator(_)
                | FModifier::Cycles(_)
        ) {
            let inf = m.effective_influence(evaltime);
            if inf <= 1e-12 {
                return cvalue;
            }
            let new_val = match &m.data {
                FModifier::Noise(n) => eval_noise(n, evaltime, cvalue),
                FModifier::Envelope(e) => eval_envelope(e, evaltime, cvalue),
                FModifier::Limits(l) => eval_limits_value(l, cvalue),
                FModifier::Generator(g) => eval_generator(g, evaltime, cvalue),
                FModifier::Cycles(c) => eval_cycles_value_offset(c, evaltime, cvalue, fcurve),
                _ => cvalue,
            };
            crate::animation::fcurve::lerp(cvalue, new_val, inf)
        } else {
            cvalue
        }
    }

    // ── Cycles value offset (RepeatOffset only) ─────────────────────────

    fn eval_cycles_value_offset(
        cycles: &FModCycles,
        evaltime: f64,
        cvalue: f64,
        fcurve: &FCurve,
    ) -> f64 {
        if fcurve.keyframes.len() < 2 {
            return cvalue;
        }
        let first = fcurve.keyframes[0].time;
        let last = fcurve.keyframes[fcurve.keyframes.len() - 1].time;
        let length = last - first;
        if length.abs() < 1e-9 {
            return cvalue;
        }

        let (mode, max_cycles) = if evaltime < first {
            (cycles.before_mode, cycles.before_cycles)
        } else if evaltime > last {
            (cycles.after_mode, cycles.after_cycles)
        } else {
            return cvalue;
        };

        if !matches!(mode, CycleMode::RepeatOffset) {
            return cvalue;
        }

        let offset = ((evaltime - first) / length).floor();
        let bounded = if max_cycles > 0 {
            offset.clamp(-(max_cycles as f64), max_cycles as f64)
        } else {
            offset
        };

        let first_val = fcurve.keyframes[0].value;
        let last_val = fcurve.keyframes[fcurve.keyframes.len() - 1].value;
        cvalue + bounded * (last_val - first_val)
    }

    // ── Cycles time ───────────────────────────────────────────────────────

    fn eval_cycles_time(cycles: &FModCycles, evaltime: f64, fcurve: &FCurve) -> f64 {
        if fcurve.keyframes.len() < 2 {
            return evaltime;
        }
        let first = fcurve.keyframes[0].time;
        let last = fcurve.keyframes[fcurve.keyframes.len() - 1].time;
        let length = last - first;

        if length.abs() < 1e-9 {
            return evaltime;
        }

        if evaltime < first {
            eval_cycles_side(
                cycles.before_mode,
                cycles.before_cycles,
                first,
                length,
                evaltime,
                -1,
            )
        } else if evaltime > last {
            eval_cycles_side(
                cycles.after_mode,
                cycles.after_cycles,
                first,
                length,
                evaltime,
                1,
            )
        } else {
            evaltime
        }
    }

    fn eval_cycles_side(
        mode: CycleMode,
        max_cycles: u16,
        range_start: f64,
        length: f64,
        evaltime: f64,
        _side: i32,
    ) -> f64 {
        match mode {
            CycleMode::None => evaltime,
            CycleMode::Repeat => {
                let offset = ((evaltime - range_start) / length).floor();
                let bounded = if max_cycles > 0 {
                    offset.clamp(-(max_cycles as f64), max_cycles as f64)
                } else {
                    offset
                };
                evaltime - bounded * length
            }
            CycleMode::RepeatOffset => {
                CycleMode::Repeat.apply(range_start, length, evaltime, max_cycles)
            }
            // RepeatOffset uses same time wrapping (value offset applied separately)
            CycleMode::Mirror => {
                let raw_offset = (evaltime - range_start) / length;
                let cycle = raw_offset.floor();
                let bounded_cycle = if max_cycles > 0 {
                    cycle.clamp(-(max_cycles as f64), max_cycles as f64)
                } else {
                    cycle
                };
                let frac = raw_offset - cycle;
                let is_forward = (bounded_cycle as i64).rem_euclid(2) == 0;
                if is_forward {
                    range_start + frac * length
                } else {
                    range_start + (1.0 - frac) * length
                }
            }
        }
    }

    impl CycleMode {
        fn apply(&self, range_start: f64, length: f64, evaltime: f64, max_cycles: u16) -> f64 {
            let offset = ((evaltime - range_start) / length).floor();
            let bounded = if max_cycles > 0 {
                offset.clamp(-(max_cycles as f64), max_cycles as f64)
            } else {
                offset
            };
            evaltime - bounded * length
        }
    }

    // ── Stepped time ──────────────────────────────────────────────────────

    fn eval_stepped_time(stepped: &FModStepped, evaltime: f64) -> f64 {
        if stepped.flags.no_before && evaltime < stepped.start_frame {
            return evaltime;
        }
        if stepped.flags.no_after && evaltime > stepped.end_frame {
            return evaltime;
        }
        if stepped.step_size.abs() < 1e-9 {
            return evaltime;
        }
        let snap = ((evaltime - stepped.offset) / stepped.step_size).floor();
        snap * stepped.step_size + stepped.offset
    }

    // ── Limits time ───────────────────────────────────────────────────────

    fn eval_limits_time(limits: &FModLimits, evaltime: f64) -> f64 {
        let mut t = evaltime;
        if let Some(xmin) = limits.xmin {
            t = t.max(xmin);
        }
        if let Some(xmax) = limits.xmax {
            t = t.min(xmax);
        }
        t
    }

    // ── Limits value ──────────────────────────────────────────────────────

    fn eval_limits_value(limits: &FModLimits, value: f64) -> f64 {
        let mut v = value;
        if let Some(ymin) = limits.ymin {
            v = v.max(ymin);
        }
        if let Some(ymax) = limits.ymax {
            v = v.min(ymax);
        }
        v
    }

    // ── Noise value ───────────────────────────────────────────────────────

    fn eval_noise(noise: &FModNoise, evaltime: f64, cvalue: f64) -> f64 {
        let scale = if noise.size.abs() < 1e-9 {
            0.0
        } else {
            1.0 / noise.size
        };
        let noise_val = perlin_fbm(
            (evaltime - noise.offset) * scale + 0.618034,
            noise.phase,
            noise.depth.max(1),
            noise.roughness,
            noise.lacunarity,
            true,
        );
        // noise_val ≈ [0, 1], center around 0
        let n = (noise_val - 0.5) * 2.0 * noise.strength;
        match noise.modification {
            NoiseMod::Replace => n,
            NoiseMod::Add => cvalue + n,
            NoiseMod::Subtract => cvalue - n,
            NoiseMod::Multiply => cvalue * (1.0 + n * 0.5),
        }
    }

    // ── Envelope value ────────────────────────────────────────────────────

    fn eval_envelope(env: &FModEnvelope, evaltime: f64, cvalue: f64) -> f64 {
        let pts = &env.control_points;
        if pts.is_empty() {
            return cvalue;
        }

        // Find bracketing control points
        let (min_val, max_val) = if evaltime <= pts[0].time {
            (pts[0].min, pts[0].max)
        } else if evaltime >= pts[pts.len() - 1].time {
            let last = &pts[pts.len() - 1];
            (last.min, last.max)
        } else {
            let i = pts.partition_point(|p| p.time < evaltime);
            let p0 = &pts[i - 1];
            let p1 = &pts[i];
            let span = p1.time - p0.time;
            let t = if span.abs() < 1e-9 {
                0.0
            } else {
                ((evaltime - p0.time) / span).clamp(0.0, 1.0)
            };
            let min_v = p0.min + (p1.min - p0.min) * t;
            let max_v = p0.max + (p1.max - p0.max) * t;
            (min_v, max_v)
        };

        let range = env.max - env.min;
        if range.abs() < 1e-9 {
            return cvalue;
        }
        let fac = ((cvalue - env.midval) - env.min) / range;
        let clipped = fac.clamp(0.0, 1.0);
        min_val + (max_val - min_val) * clipped
    }

    // ── Generator value ───────────────────────────────────────────────────

    fn eval_generator(generator: &FModGenerator, evaltime: f64, cvalue: f64) -> f64 {
        let generated = match generator.mode {
            GeneratorMode::Polynomial => {
                let mut result = 0.0;
                let mut power = 1.0f64;
                for &coeff in &generator.coefficients {
                    result += coeff * power;
                    power *= evaltime;
                }
                result
            }
            GeneratorMode::Factorised => {
                let mut result = 1.0;
                let coeffs = &generator.coefficients;
                let mut i = 0;
                while i + 1 < coeffs.len() {
                    result *= coeffs[i] * evaltime + coeffs[i + 1];
                    i += 2;
                }
                if i < coeffs.len() {
                    result *= coeffs[i];
                }
                result
            }
        };
        if generator.additive {
            cvalue + generated
        } else {
            generated
        }
    }
}
