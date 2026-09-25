//! 字幕域 UI 状态：视口里每行字幕的包围框模型，与一次「按住某一行拖动」的会话。
//!
//! 框由 `render::subtitle::hits` 每次出帧从 libass 的字形块还原，单位是排版像素
//! （= 视口世界坐标）。模型行由 `projections::ui_sync_subtitle_boxes` 逐帧差异写入。
//!
//! 拖动期间**不往数据里写**：只在会话里攒着位移，把框画在指针底下，松手时一次性提交
//! 给 `SubtitleAction::ShiftLinePosition`。提交的是**增量**——样式链里那份位置是基线，
//! 位置动画在它之上相加，所以带 `\move` 的行照样拖得动，整条轨迹跟着平移。重编一份脚本
//! 要几百毫秒，逐帧提交会把 UI 拖死。提交之后脚本还是旧的，所以会话要留在原地继续压着
//! 画，直到后台那份脚本落地才清。

use std::rc::Rc;

use aweft_model::ids::ItemId;
use slint::VecModel;

use crate::bridge::types::SlintSubtitleBox;

#[derive(Default)]
pub struct SubtitleUiState {
    /// 此刻画面上每行字幕的包围框。顺序既是视口 `for lbox in SubtitleState.boxes` 画框的
    /// 顺序（后画的压在前面的上面），也是命中判定倒着扫的那一份列表（先扫到最上面的），
    /// 两边同源，所以点中的框与拖动的框必然是同一条。
    pub boxes: Rc<VecModel<SlintSubtitleBox>>,
}

/// 一次按住某一行字幕拖动的会话（`AppState::subtitle_line_drag`）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct LineDragSession {
    pub item_id: ItemId,
    /// 已经写进链、libass 却还没画出来的位移：上一次提交的脚本还在后台编时就是那次的量，
    /// 所以接着拖同一行不会让框跳回旧位置。
    pub pending_x: f64,
    pub pending_y: f64,
    /// 本次按下以来的位移（Slint 报来的世界坐标差），松手时冻结并按它提交增量。
    pub drag_x: f64,
    pub drag_y: f64,
    /// 已写进链、等后台脚本追上。为 `true` 期间框仍按 `pending + drag` 画；脚本一落地
    /// （既不在飞也不排队）就丢掉整个会话，框改由 libass 画出的位置决定。
    pub committed: bool,
}
