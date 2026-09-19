//! 字幕脚本投影：把「序列 + 样式层 + 动画」这份真相编成一份 ASS 脚本，交给字幕通路。
//!
//! 编脚本要遍历全部行、并对每条动画跑一次 DP 拟合，属于用户操作节奏的工作；
//! 逐帧那一侧只做 libass 出图与合成（见 `render::subtitle::pipeline`）。
//! 因此本投影负责写脚本，`handle_before_rendering` 负责出画面。

use crate::depsgraph::Tag;
use crate::store::AppStore;

/// 行结构、行时间与文本、动画与样式图、主视频（帧尺寸来源）的变化都要重编脚本。
/// 订阅哪几个 `TagKind` 在 `projections/mod.rs` 的注册表里声明，这里不重复。
pub fn op_subtitle_script(store: &mut AppStore, _tags: &[Tag]) {
    subtitle_rebuild_script(store);
}

/// 重编脚本并推给字幕通路。
///
/// 帧尺寸取预览值：有主视频用它的显示像素尺寸，没有则按 1080P 兜底，因此空视口也
/// 能出字幕。唯一的门禁是渲染器还没给出 GPU 句柄（字幕通路不存在）——这种情形由
/// `handle_rendering_setup` 建好通路后再调用本函数补上，两条路共用这一个入口。
pub(crate) fn subtitle_rebuild_script(store: &mut AppStore) {
    let state = store.state_mut();
    let frame_size = state.preview_frame_size();
    let Some(pipeline) = state.subtitle.as_mut() else {
        return;
    };

    let options = aweft_convert::ConvertOptions::default();
    let ass =
        match aweft_convert::sequence_to_ass(state.project.project_mut(), frame_size, &options) {
            Ok(ass) => ass,
            Err(error) => {
                log::warn!("字幕脚本生成失败: {error}");
                return;
            }
        };
    match ass.to_bytes() {
        Ok(bytes) => pipeline.set_script(bytes),
        Err(error) => log::warn!("字幕脚本写进内存失败: {error}"),
    }
}
