use serde::{Deserialize, Serialize};

use crate::animation::anim_data::BlendMode;
use crate::animation::fcurve::FCurve;
use crate::animation::PropertyPath;
use crate::ids::ActionId;

/// Associates an F-Curve with a property path.
///
/// Multiple `ActionCurve` entries with the same `property` are allowed —
/// they are blended using their individual `blend_mode` to produce complex
/// combined effects (e.g. a walk cycle built from displacement + arm swing + bounce).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionCurve {
    pub property: PropertyPath,
    /// How this curve's value combines with the previous value for the same property.
    /// Defaults to `Replace` — each curve overwrites the value below it.
    #[serde(default)]
    pub blend_mode: BlendMode,
    pub curve: FCurve,
}

impl ActionCurve {
    pub fn new(property: PropertyPath, curve: FCurve) -> Self {
        Self {
            property,
            blend_mode: BlendMode::default(),
            curve,
        }
    }
}

/// A reusable animation data block containing F-Curves for a set of properties.
///
/// This is the animation "source material" — it does not know about timing,
/// placement, or external blending. NLA strips reference an `Action` to drive
/// their animation values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub name: String,
    #[serde(default)]
    pub duration: f64,
    /// All property curves in this action, evaluated in order.
    /// Multiple entries for the same property are blended using each entry's `blend_mode`.
    #[serde(default)]
    pub curves: Vec<ActionCurve>,
}

impl Action {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ActionId::new(),
            name: name.into(),
            duration: 0.0,
            curves: Vec::new(),
        }
    }

    pub fn find_fcurve(&self, property: &PropertyPath) -> Option<&FCurve> {
        self.curves.iter().find_map(|ac| {
            if &ac.property == property {
                Some(&ac.curve)
            } else {
                None
            }
        })
    }

    /// Sample all F-Curves at the given time, returning a map of property→value.
    /// Multiple curves for the same property are blended using each curve's `blend_mode`.
    pub fn sample(
        &self,
        time: f64,
    ) -> std::collections::HashMap<PropertyPath, f64> {
        use crate::animation::fcurve::evaluate_fcurve;
        use crate::animation::sample::blend_scalar;
        let mut result: std::collections::HashMap<PropertyPath, f64> =
            std::collections::HashMap::new();
        for ac in &self.curves {
            let val = evaluate_fcurve(&ac.curve, time);
            let entry = result.entry(ac.property.clone()).or_insert(0.0);
            *entry = blend_scalar(*entry, val, ac.blend_mode, 1.0);
        }
        result
    }
}
