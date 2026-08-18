//! 动画编辑器（曲线编辑器）操作 Action：交互入口。
//!
//! 走 main 标准管线 `Action → dispatch → tag → 投影`（对齐 node_graph）：
//!  - reducer（`AppStore::dispatch_animation`）只改数据层（channels_data/editing_keyframes/UI 态）
//!    并返回精确 tag（AnimationStructure/Keyframes/Selection/Channel/Scalars）
//!  - 投影（`op_animation_*`）把数据层同步到派生层（Slint 绑定模型 + 标量）

use crate::bridge::types::{
    SlintColorMode, SlintHandleDragMode, SlintInterpolation, SlintKeyframeDragMode,
    SlintSelectionModifier,
};

/// 曲线编辑器（Graph Editor）交互操作。
#[derive(Debug, Clone)]
pub enum AnimationAction {
    // —— 关键帧 ——
    /// 关键帧按下：none=单选，shift=区间扩展，ctrl=切换。
    KeyframePressed {
        curve: usize,
        key: usize,
        modifier: SlintSelectionModifier,
    },
    /// 关键帧拖拽（delta 相对按下起点，世界坐标）。
    KeyframeDragged {
        curve: usize,
        key: usize,
        delta_time: f32,
        delta_value: f32,
        mode: SlintKeyframeDragMode,
    },
    /// 关键帧松开。`moved=false`（单击）时折叠为单选。
    KeyframeReleased {
        curve: usize,
        key: usize,
        moved: bool,
    },
    /// 画布空白点击：清除关键帧选中。
    DeselectKeyframes,

    // —— 贝塞尔控制柄 ——
    /// 手柄按下：none=单选，shift=区间扩展，ctrl=切换。
    HandlePressed {
        curve: usize,
        key: usize,
        is_in: bool,
        modifier: SlintSelectionModifier,
    },
    /// 手柄拖拽：free=Ctrl 自由，aligned=默认对齐，symmetric=Shift 原点对称。
    HandleDragged {
        curve: usize,
        key: usize,
        is_in: bool,
        delta_time: f32,
        delta_value: f32,
        mode: SlintHandleDragMode,
    },
    /// 手柄松开。`moved=false`（单击）时折叠为单选该手柄。
    HandleReleased {
        curve: usize,
        key: usize,
        is_in: bool,
        moved: bool,
    },
    /// 画布空白框选（none=替换，shift=区间扩展，ctrl=切换）。
    GraphMarqueeSelect {
        x_lo: f32,
        x_hi: f32,
        y_lo: f32,
        y_hi: f32,
        modifier: SlintSelectionModifier,
    },

    // —— 通道 ——
    /// 通道/曲线按下：none=普通，shift=区间，ctrl=切换。
    ChannelPressed {
        curve: usize,
        modifier: SlintSelectionModifier,
    },
    /// 画布空白点击：清除激活通道与选中。
    ChannelDeselectAll,
    /// 曲线编辑器搜索框。
    GraphSearchChanged {
        text: String,
    },
    /// 摄影表搜索框。
    DopeSearchChanged {
        text: String,
    },
    /// 摄影表压缩视图行被点击（显示行 → Rust 映射回真实通道）。
    DopeChannelPressed {
        view_row: usize,
        modifier: SlintSelectionModifier,
    },
    /// 侧边栏：曲线颜色来源（0=自动彩虹，1=用户设置）。
    CurveColorModeChanged {
        curve: usize,
        mode: SlintColorMode,
    },
    /// 侧边栏：设置选中关键帧插值。
    KeyframeInterpolationChanged {
        interpolation: SlintInterpolation,
    },
    /// 通道列表开关。
    ChannelToggleVisible {
        curve: usize,
    },
    ChannelToggleModifier {
        curve: usize,
    },
    ChannelToggleActive {
        curve: usize,
    },
    ChannelToggleEditable {
        curve: usize,
    },

    // —— 共享 ——
    /// 当前时间（播放头）。
    SetCurrentTime {
        time: f32,
    },
    /// 视图缩放变化（更新命中检测容差）。
    ViewChanged {
        zoom_x: f32,
        zoom_y: f32,
    },
    /// Action 选择（Header 下拉菜单）。
    ActionSelected {
        index: i32,
    },
}
