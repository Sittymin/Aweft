use serde::{Deserialize, Serialize};

use crate::animation::strip::NlaTrack;
use crate::ids::AnimDataId;

// ── Property path (unchanged from original) ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PropertyPath {
    FontSize,

    Bold,
    Italic,
    Underline,
    Strikeout,

    ColorPrimaryR,
    ColorPrimaryG,
    ColorPrimaryB,
    ColorPrimaryA,

    ColorSecondaryR,
    ColorSecondaryG,
    ColorSecondaryB,
    ColorSecondaryA,

    ColorOutlineR,
    ColorOutlineG,
    ColorOutlineB,
    ColorOutlineA,

    ColorBackR,
    ColorBackG,
    ColorBackB,
    ColorBackA,

    BorderX,
    BorderY,

    ShadowX,
    ShadowY,

    FontScaleX,
    FontScaleY,

    FontRotationX,
    FontRotationY,
    FontRotationZ,

    FactorX,
    FactorY,

    Blur,
    BlurEdges,

    PositionX,
    PositionY,

    OriginX,
    OriginY,

    ClipRectX1,
    ClipRectY1,
    ClipRectX2,
    ClipRectY2,
}

// ── Blend mode ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    /// Overwrite: lerp toward override value.
    Replace,
    /// Accumulate: add override * influence.
    Add,
    /// Subtract: subtract override * influence.
    Subtract,
    /// Multiply: scale by (1 + (override - 1) * influence).
    Multiply,
    /// Crossfade: lerp between base and override.
    Combine,
}

impl Default for BlendMode {
    fn default() -> Self {
        Self::Replace
    }
}

// ── Animation clip ────────────────────────────────────────────────────────

/// An animation clip is a named configuration of NLA tracks with strips.
///
/// It does NOT contain the animation data (F-Curves) directly — those live
/// in `Action` objects stored in the `AnimationLibrary`. Strips reference
/// actions by their `ActionId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimData {
    pub id: AnimDataId,
    pub name: String,
    /// NLA tracks, evaluated from first to last (bottom to top).
    #[serde(default)]
    pub tracks: Vec<NlaTrack>,
}

impl AnimData {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: AnimDataId::new(),
            name: name.into(),
            tracks: Vec::new(),
        }
    }
}
