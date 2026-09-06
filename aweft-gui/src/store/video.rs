//! 视频域 UI 状态：每个视频会话派生到 Slint `VideoState` 的模型集合。
//! Rust 侧持有 VecModel（共享 inner 模型，变更自动通知 Slint）：
//! - `frames`/`widths`/`heights`：最新帧与尺寸（渲染线程逐帧 `set_row_data`）
//! - `positions`：视口世界坐标位置（级联错开/拖拽移动）
//! - `time_bars`：各面板控制时间条（普通 Action 由投影写，播放时钟逐帧增量写）
//!
//! 会话结构与普通状态变化由 `projections/video` 用 `push`/`set_row_data` 局部更新
//! （不重建既有行，避免拖拽/帧模型被打断）；时间条逐帧更新走投影提供的
//! 实时专用出口。音频模型见 `store/audio`，派生到 Slint `AudioState`。

use std::rc::Rc;

use slint::{Image, VecModel};

use crate::bridge::types::{SlintVideoPos, SlintVideoTimeBar};

#[derive(Default)]
pub struct VideoUiState {
    /// 每个会话的最新视频帧（下标 = 会话下标；首帧前为默认空图）
    pub frames: Rc<VecModel<Image>>,
    /// 每个会话的视频宽度（0 = 尚未收到首帧）
    pub widths: Rc<VecModel<i32>>,
    /// 每个会话的视频高度（0 = 尚未收到首帧）
    pub heights: Rc<VecModel<i32>>,
    /// 每个会话的视口世界坐标位置
    pub positions: Rc<VecModel<SlintVideoPos>>,
    /// 每个会话的控制时间条（彩虹色 + 选中/主视频标识）
    pub time_bars: Rc<VecModel<SlintVideoTimeBar>>,
}
