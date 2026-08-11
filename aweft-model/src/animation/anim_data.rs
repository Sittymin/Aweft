use serde::{Deserialize, Serialize};

use crate::animation::strip::NlaTrack;
use crate::ids::{ActionId, AnimDataId};

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
    /// 基础编辑 Action：主编辑工作区。
    ///
    /// 非临时编辑（`temporary_edit == false`）时按绝对时间求值，叠加在所有
    /// NLA 轨道之下，保证用户编辑时能看到自己的改动。
    #[serde(default)]
    pub base_action: Option<ActionId>,
    /// 是否正在临时编辑 NLA 片断。
    ///
    /// 临时编辑期间编辑的就是 NLA 轨道里片断引用的那个 Action，改动直接反映
    /// 在 NLA 求值中，因此 `base_action` 暂停求值，仅作为取消临时编辑后的
    /// 返回目标。
    #[serde(default)]
    pub temporary_edit: bool,
}

impl AnimData {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: AnimDataId::new(),
            name: name.into(),
            tracks: Vec::new(),
            base_action: None,
            temporary_edit: false,
        }
    }

    /// 基础编辑 Action（无论是否处于临时编辑模式都返回它）。
    pub fn base_action(&self) -> Option<ActionId> {
        self.base_action
    }

    /// 设置基础编辑 Action 并退出临时编辑模式。
    pub fn set_base_action(&mut self, action: ActionId) {
        self.base_action = Some(action);
        self.temporary_edit = false;
    }

    /// 进入 / 退出 NLA 片断临时编辑模式。不改变 `base_action`。
    pub fn set_temporary_edit(&mut self, active: bool) {
        self.temporary_edit = active;
    }

    /// 是否正在临时编辑 NLA 片断。
    pub fn is_temporary_editing(&self) -> bool {
        self.temporary_edit
    }

    /// 清空编辑工作区。
    pub fn clear_editing(&mut self) {
        self.base_action = None;
        self.temporary_edit = false;
    }
}
