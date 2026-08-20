//! NLA 编辑器操作 Action（动画域 NLA 子集）：交互入口。
//!
//! 走 main 标准管线 `Action::Animation(AnimationAction::Nla(...)) → dispatch_animation → tag → 投影`（统一动画域）：
//!  - reducer（`AppStore::dispatch_animation` 子路由 Nla）只改数据层 + model（`AnimData.tracks`）
//!    并返回精确 tag（NlaStructure/NlaStrips/NlaTrackProperties/NlaTrackAdded）
//!  - 投影（`projections/animation/nla.rs:op_nla_*`）把数据层同步到派生层（Slint 绑定模型）

use crate::bridge::types::SlintSelectionModifier;

/// NLA 编辑器交互操作。
#[derive(Debug, Clone)]
pub enum NlaAction {
    // —— 片断 ——
    /// 片断按下：none=单选，shift/ctrl=追加切换。同时记录拖拽起点。
    StripPressed {
        track: usize,
        strip: usize,
        modifier: SlintSelectionModifier,
    },
    /// 片断拖拽（delta 相对按下起点：delta-x 世界时间，delta-row 行数）。
    StripDragged {
        track: usize,
        strip: usize,
        delta_x: f32,
        delta_row: f32,
    },
    /// 片断松开（提交：数据层整理 + 写回 model）。
    StripReleased { track: usize, strip: usize },
    /// 画布空白点击：清除全部片断选中。
    StripDeselectAll,

    // —— 轨道 ——
    /// 轨道启用开关（enabled ↔ model NlaTrack.muted）。
    ToggleTrackEnabled { track: usize },
    /// 轨道锁定开关（纯 UI 态）。
    ToggleTrackLocked { track: usize },
    /// 全局 Action 轨"向下插入"：把当前编辑的 Action 作为片断加入 NLA。
    PushDown,
    /// 轨道列表搜索（空串 = 全部）。
    SearchChanged { text: String },
}
