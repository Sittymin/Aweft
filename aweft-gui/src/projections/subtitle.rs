//! 字幕脚本投影：把「序列 + 样式层 + 动画」这份真相编成一份 ASS 脚本，交给字幕通路。
//!
//! 编脚本要遍历全部行、并对每条动画跑一次 DP 分段，是整条链里唯一的重活，因此它不在
//! 这里跑：本投影只把工程快照提交给 `store::subtitle_fit` 的后台线程，跑完由那条路上
//! 的 pump 把脚本换进通路，期间视口右上角显示一块进度浮层，不挡操作。逐帧那一侧只做 libass 出图与
//! 合成（见 `render::subtitle::pipeline`），所以 `handle_before_rendering` 照常出画面。

use crate::bridge::types::SlintSubtitleBox;
use crate::depsgraph::Tag;
use crate::store::AppStore;
use slint::Model;

/// 行结构、行时间与文本、动画与样式图、主视频（帧尺寸来源）的变化都要重编脚本。
/// 订阅哪几个 `TagKind` 在 `projections/mod.rs` 的注册表里声明，这里不重复。
pub fn op_subtitle_script(store: &mut AppStore, _tags: &[Tag]) {
    subtitle_rebuild_script(store);
}

/// 提交一次脚本重编。
///
/// 帧尺寸取预览值：有主视频用它的显示像素尺寸，没有则按 1080P 兜底，因此空视口也
/// 能出字幕。唯一的门禁是渲染器还没给出 GPU 句柄（字幕通路不存在）——这种情形由
/// `handle_rendering_setup` 建好通路后再调用本函数补上，两条路共用这一个入口。
pub(crate) fn subtitle_rebuild_script(store: &mut AppStore) {
    crate::store::subtitle_fit::request_rebuild(store);
}

/// 把最近一次出帧还原的行框发布到 `SubtitleState.boxes`：视口照它画包围框，
/// 命中检测与拖动也都按它的下标定身份（见 `SubtitleCallbacks`）。
///
/// 与视频帧、播放时钟同属实时例外，不经 reducer/depsgraph。**只写变了的那几行**：
/// `VecModel::set_row_data` 一律发通知，无条件逐帧重写会让 Slint 重绘 → 又一次
/// `before_rendering` → 再同步，转成停不下来的重绘循环。
pub(crate) fn ui_sync_subtitle_boxes(store: &mut AppStore) {
    let AppStore {
        state, ui_models, ..
    } = store;

    // 提交过的那次拖动：后台脚本一落地（既不在飞也不排队），预览偏移就该撤了，
    // 框的位置从此由 libass 实际画出来的几何决定。
    if state.subtitle_line_drag.is_some_and(|drag| drag.committed)
        && !state.fit_in_flight
        && !state.fit_pending
    {
        state.subtitle_line_drag = None;
    }
    let drag = state.subtitle_line_drag;
    let Some(pipeline) = state.subtitle.as_ref() else {
        return;
    };

    let rows: Vec<SlintSubtitleBox> = pipeline
        .line_boxes()
        .iter()
        .map(|line| {
            // 拖动中的那一行把预览偏移压上去，其余行原样。偏移 = 按下时已提交的量 +
            // 本次按下以来的位移，所以提交后立刻再按住同一行，框不会跳回旧位置。
            let (dx, dy) = match drag {
                Some(drag) if drag.item_id == line.item_id => {
                    (drag.pending_x + drag.drag_x, drag.pending_y + drag.drag_y)
                }
                _ => (0.0, 0.0),
            };
            SlintSubtitleBox {
                item_id: line.item_id.to_string().into(),
                x: (line.x + dx) as f32,
                y: (line.y + dy) as f32,
                width: line.w as f32,
                height: line.h as f32,
                selected: ui_models
                    .sequence_state
                    .rows_model()
                    .is_selected(line.item_id),
            }
        })
        .collect();

    let model = &ui_models.subtitle.boxes;
    for index in (rows.len()..model.row_count()).rev() {
        model.remove(index);
    }
    for (index, row) in rows.iter().enumerate() {
        match model.row_data(index) {
            Some(old) if &old == row => {}
            Some(_) => model.set_row_data(index, row.clone()),
            None => model.push(row.clone()),
        }
    }
}
