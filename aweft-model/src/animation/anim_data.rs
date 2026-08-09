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

// ── Edit source ───────────────────────────────────────────────────────────

/// 当前"编辑工作区"指向哪一份动画数据。
///
/// 曲线编辑器同一时刻只能编辑一份 Action（与 NLA 中选中的片断互斥显示），
/// 由 `AnimData::edit_source()` 派生：
/// - `Unique`：全局唯一编辑 Action，按绝对时间求值，叠加在所有 NLA 轨道之下；
/// - `NlaStrip`：NLA 中选中片断对应的 Action 临时编辑，按 `(track, strip)` 索引
///   定位（片断自身持有 `ActionId`，无需单独存储）。索引可能因增删失效，
///   求值时需做越界校验并回退为"无编辑层"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditSource {
    Unique(ActionId),
    NlaStrip { track: usize, strip: usize },
}

/// NLA 临时编辑指向的片断。
///
/// 与"全局唯一 Action"互斥显示，但它只是一个瞬态叠加层：**不覆盖
/// `unique_action`**，因此无论临时编辑多少次/切换多少片断，"取消临时编辑"
/// 都必然回到全局唯一 Action，不会被中间状态卡住。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempStripRef {
    pub track: usize,
    pub strip: usize,
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
    /// 全局唯一编辑 Action。独立于临时编辑存在，取消临时编辑必然恢复到它。
    #[serde(default)]
    pub unique_action: Option<ActionId>,
    /// 当前处于 NLA 临时编辑模式时指向的片断（`None` = 非临时编辑）。
    /// 仅影响显示与求值来源，不改动 `unique_action`。
    #[serde(default)]
    pub temporary_edit: Option<TempStripRef>,
}

impl AnimData {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: AnimDataId::new(),
            name: name.into(),
            tracks: Vec::new(),
            unique_action: None,
            temporary_edit: None,
        }
    }

    /// 当前有效的编辑工作区（互斥：临时编辑片断优先，否则全局唯一 Action）。
    pub fn edit_source(&self) -> Option<EditSource> {
        match self.temporary_edit {
            Some(TempStripRef { track, strip }) => {
                Some(EditSource::NlaStrip { track, strip })
            }
            None => self.unique_action.map(EditSource::Unique),
        }
    }

    /// 全局唯一编辑 Action（无论是否处于临时编辑模式都返回它）。
    pub fn unique_action(&self) -> Option<ActionId> {
        self.unique_action
    }

    /// 将全局唯一编辑 Action 设为 `action` 并退出临时编辑模式。
    pub fn set_editing_unique(&mut self, action: ActionId) {
        self.unique_action = Some(action);
        self.temporary_edit = None;
    }

    /// 进入 NLA 片断的临时编辑（再次调用则切换目标片断）。
    /// 不改变 `unique_action`。
    pub fn set_editing_strip(&mut self, track: usize, strip: usize) {
        self.temporary_edit = Some(TempStripRef { track, strip });
    }

    /// 取消 NLA 片断的临时编辑，恢复为全局唯一编辑 Action。
    pub fn cancel_temporary_edit(&mut self) {
        self.temporary_edit = None;
    }

    /// 清空编辑工作区。
    pub fn clear_editing(&mut self) {
        self.unique_action = None;
        self.temporary_edit = None;
    }
}
