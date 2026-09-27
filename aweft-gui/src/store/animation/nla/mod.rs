//! NLA 编辑器 Rust 侧：持有 NLA 轨道视图模型与全部编辑操作（动画域 NLA 子集）。
//!
//! # main 架构接线（依赖图结构）
//!
//! 交互统一走 main 标准管线：`AnimationCallbacks` → `AppStore::dispatch(AnimationAction::Nla(NlaAction))`
//! → `dispatch_animation` 子路由 Nla（reducer，只改数据层并返回精确 tag）→ depsgraph 单轮求值
//! → 投影 `projections/animation/nla.rs:op_nla_*`（把数据层同步到派生层 Slint 模型）。
//!
//! # 权威数据在 model（已接线）
//!
//! NLA 轨道/片断权威存储在 `Project.library.animation_library.anim_datas`
//! 中 `AnimData.tracks`（`aweft-model::animation::NlaTrack/NlaStrip`）。本 store
//! 只保存编辑数据并生成 Slint 的轨道/片断视图模型（投影），不再自持权威数据。
//!
//! 映射关系（轨道下标含第 0 轨虚拟"全局 Action 轨"，1:1 显示、只读）：
//!  - 视图轨道 `i`（>= 1）↔ model `AnimData.tracks[i-1]`
//!  - `SlintNlaStrip` ↔ `NlaStrip`（时间范围 + 类型）。UI 只编辑时间范围/名称，
//!    其余字段（blend_mode/extend_mode/modifiers/strip_type）保留在 `NlaStripData.model`
//!    克隆里，提交时合并回 model（placement 更新为 Absolute{start,end}）。
//!
//! 数据流：
//!  - **装载**：`load_from_model` 读 model AnimData → 重建数据层 + 派生层（全量）
//!  - **拖拽（高频）**：`strip_dragged` 只改数据层 + 返回 `NlaStripsChanged` tag，
//!    投影就地 `set_row_data`（或精确 remove+insert 跨轨），保住 Slint 拖拽抓取
//!  - **提交**：`strip_released` 后 reducer 调 `commit_to_model` 把数据层写回 model
//!  - UI 态（选中/搜索/锁定/拖拽状态）留在本 store，不入 model

use std::rc::Rc;

use aweft_model::{animation::strip::NlaStrip, ids::AnimDataId};
use slint::VecModel;

use crate::bridge::types::{ContextMenuItem, SlintNlaStrip, SlintNlaTrack};

pub mod drag;
pub mod load;
pub mod marquee;
pub mod projection;
pub mod selection;
pub mod strip;
pub mod track;

pub(crate) type StripId = u64;

/// 拖拽开始时固定的片段快照。
///
/// `track/index` 只是按下时的位置；拖拽过程中片段可能排序或跨轨，后续定位必须使用
/// `id`，不能再依赖下标。
#[derive(Debug, Clone, Copy)]
pub(crate) struct DragStripSnapshot {
    pub id: StripId,
    pub origin_track: usize,
    pub origin_index: usize,
    pub origin_start: f32,
    pub width: f32,
}

/// 片断拖拽状态。锚点片段与按下时位置统一通过稳定 ID 和快照查询；
/// UI 后续只需传递累计 delta，不需要重复传回片段坐标。
#[derive(Debug, Clone)]
pub(crate) struct StripDrag {
    anchor_id: StripId,
    /// 按下时所有选中片段的稳定快照，用于多选整体规划与取消恢复。
    selected_snapshot: Vec<DragStripSnapshot>,
    /// 多选拖拽最后一次成功应用的整体偏移与轨道偏移。
    last_offset: f32,
    last_row_shift: isize,
}

/// 一条片断的编辑数据：稳定的编辑器 ID、Slint 视图 + model 原始克隆。
#[derive(Debug, Clone)]
pub(crate) struct NlaStripData {
    pub id: StripId,
    pub view: SlintNlaStrip,
    /// model 中的原始 NlaStrip 克隆（提交时合并回 model，placement 更新为 Absolute）。
    pub model: NlaStrip,
}

/// 向下插入的数据层结果：目标轨道、片断插入下标，以及是否追加了新轨道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NlaPushDownResult {
    pub track: usize,
    pub strip: usize,
    pub new_track: bool,
}

/// NLA 编辑器状态
pub struct NlaEditorState {
    /// 当前编辑的 AnimData（model 权威数据所在；None = 尚未装载）。
    pub anim_data_id: Option<AnimDataId>,

    // —— 数据层（reducer 可写） ——
    /// Header 下拉的候选：库里全部 AnimData 的 id，顺序 = 显示顺序（按名排序，
    /// 与非线性动画引用节点的下拉同一份顺序，见 `AppStore::anim_data_entries`）。
    pub anim_order: Vec<AnimDataId>,
    /// Header 下拉的菜单项（由 `anim_order` 对应的名字派生；共享 inner，
    /// 初始化时绑定一次，之后 `set_vec` 自动通知 Slint）。
    pub anim_items: Rc<VecModel<ContextMenuItem>>,
    /// 轨道视图数据（权威 UI 态；`strips` 字段占位，真实片断在 `editing_strips`）。
    /// 第 0 轨为虚拟"全局 Action 轨"（只读，显示当前编辑的 Action）。
    pub(crate) track_view: Vec<SlintNlaTrack>,
    /// 片断编辑数据（轨道下标 → 片断列表，与 model tracks 同序、按 start 排序）。
    pub(crate) editing_strips: Vec<Vec<NlaStripData>>,
    /// 下一个编辑器内部片段 ID；model 没有稳定的 NLA 片段 ID，因此由 store 管理。
    pub(crate) next_strip_id: StripId,
    /// 搜索关键字（空 = 全部轨道）。
    search: String,
    /// 当前时间（播放头，与曲线编辑器共享）。
    pub current_time: f64,

    // —— 派生层（只由投影写） ——
    /// 权威轨道数据（含全部轨道；`strips` 指向 `strip_models` 中同一 inner 模型）。
    pub tracks: Rc<VecModel<SlintNlaTrack>>,
    /// 搜索过滤后的轨道投影（绑定 Slint；画布/列表都显示它，行号 = 过滤视图行，
    /// 经 `orig-index` 映射回权威轨道）。
    pub tracks_filter: Rc<VecModel<SlintNlaTrack>>,
    /// 片断 Slint 模型：权威轨道下标 → 稳定的 inner VecModel。
    pub strip_models: Vec<Rc<VecModel<SlintNlaStrip>>>,

    /// 片断拖拽状态（按下时记录；拖动中跨轨/重排会更新）。
    drag: Option<StripDrag>,
}

impl Default for NlaEditorState {
    fn default() -> Self {
        Self {
            anim_data_id: None,
            anim_order: Vec::new(),
            anim_items: Rc::new(VecModel::default()),
            track_view: Vec::new(),
            editing_strips: Vec::new(),
            next_strip_id: 1,
            search: String::new(),
            current_time: 0.0,
            tracks: Rc::new(VecModel::default()),
            tracks_filter: Rc::new(VecModel::default()),
            strip_models: Vec::new(),
            drag: None,
        }
    }
}

impl NlaEditorState {
    pub(crate) fn allocate_strip_id(&mut self) -> StripId {
        let id = self.next_strip_id;
        self.next_strip_id = self.next_strip_id.saturating_add(1).max(1);
        id
    }

    pub(crate) fn find_strip_location(&self, id: StripId) -> Option<(usize, usize)> {
        self.editing_strips
            .iter()
            .enumerate()
            .find_map(|(track, list)| {
                list.iter()
                    .position(|entry| entry.id == id)
                    .map(|index| (track, index))
            })
    }
}

#[cfg(test)]
mod tests;
