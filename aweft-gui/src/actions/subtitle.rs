//! 字幕行的视口操作：在视口里选中一行字幕、按住把它挪到别处。
//!
//! 落点送的是**排版像素增量**（与 `AppState::preview_frame_size` 同一基准）。为什么不是
//! 绝对坐标：样式链里那份位置是**基线**，位置动画在它之上相加，所以平移基线才能平移整条
//! `\move` 轨迹；送绝对坐标会把按下那一瞬动画恰好走到的偏移烤进基线里，一帧一帧拖下来
//! 位置就漂了。写进行为见 `store::subtitle_chains`：位置上只有样式链与应用链这一份真相，
//! 行上没有旁路字段。

use aweft_model::ids::ItemId;

#[derive(Debug, Clone)]
pub enum SubtitleAction {
    /// 把一行的位置基线平移这么多排版像素。基线落在这一行的视口样式链 `设位置` 端口上，
    /// 端口原本没有值就按 0 起算。
    ShiftLinePosition { item_id: ItemId, dx: f64, dy: f64 },
}
