use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Interpolation {
    Bezier {
        handle_in: BezierHandle,
        handle_out: BezierHandle,
    },
    Linear,
    Hold,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Extrapolation {
    /// Clamp to first/last keyframe value outside the keyframe range.
    #[default]
    Constant,
    /// Extend the slope of first/last segment outside the keyframe range.
    /// 延续斜率
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct BezierHandle {
    pub time_offset: f64,
    pub value_offset: f64,
}

impl BezierHandle {
    pub const fn new(time_offset: f64, value_offset: f64) -> Self {
        Self {
            time_offset,
            value_offset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub time: f64,
    pub value: f64,
    pub interpolation: Interpolation,
}

impl Keyframe {
    pub fn linear(time: f64, value: f64) -> Self {
        Self {
            time,
            value,
            interpolation: Interpolation::Linear,
        }
    }

    pub fn hold(time: f64, value: f64) -> Self {
        Self {
            time,
            value,
            interpolation: Interpolation::Hold,
        }
    }

    pub fn bezier(
        time: f64,
        value: f64,
        handle_in: BezierHandle,
        handle_out: BezierHandle,
    ) -> Self {
        Self {
            time,
            value,
            interpolation: Interpolation::Bezier {
                handle_in,
                handle_out,
            },
        }
    }
}

/// Merge keyframe times from multiple keyframe slices into a sorted,
/// deduplicated list of unique time points.
pub fn merge_keyframe_times(all_kfs: &[&[Keyframe]]) -> Vec<f64> {
    let mut time_set = std::collections::BTreeSet::new();
    for kfs in all_kfs {
        for kf in *kfs {
            let rounded = (kf.time * 1e9).round() / 1e9;
            time_set.insert(F64Ord(rounded));
        }
    }

    time_set.into_iter().map(|t| t.0).collect()
}

#[derive(PartialEq, Clone, Copy)]
struct F64Ord(f64);

impl Eq for F64Ord {}

impl PartialOrd for F64Ord {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for F64Ord {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
