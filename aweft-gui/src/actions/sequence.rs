use aweft_model::{
    ids::{ActorId, ItemId},
    sequence::SegmentTiming,
};
use chrono::TimeDelta;

use crate::bridge::types::SlintSelectionModifier;

#[derive(Debug, Clone)]
pub enum SequenceAction {
    ToggleGroupCollapsed {
        group_id: ItemId,
    },
    InputTimecode {
        event_id: ItemId,
        is_start_time: bool,
        text: String,
    },
    IncrementTimecode {
        event_id: ItemId,
        is_start_time: bool,
        is_batch: bool,
    },
    DecrementTimecode {
        event_id: ItemId,
        is_start_time: bool,
        is_batch: bool,
    },
    DragTimecodePreview {
        event_id: ItemId,
        is_start_time: bool,
        delta_x: f32,
        is_batch: bool,
    },
    DragTimecodeCommit {
        event_id: ItemId,
        is_start_time: bool,
        delta_x: f32,
        is_batch: bool,
    },
    DragTimecodeCancel {
        event_id: ItemId,
        is_start_time: bool,
        is_batch: bool,
    },
    /// 时间线片段整体拖动预览（start/end 同加 delta，不写 sequence，仅 UI 覆盖显示）。
    /// 按住项可为事件或组头（组头=子树整体平移）；`delta_x` 为世界时间秒。
    TimelineBlockDragPreview {
        item_id: ItemId,
        delta_x: f32,
    },
    /// 时间线片段整体拖动提交（写 sequence）。按住项可为事件或组头。
    TimelineBlockDragCommit {
        item_id: ItemId,
        delta_x: f32,
    },
    /// 时间线片段拖动取消（清除预览覆盖，恢复显示 Domain 当前值）。
    TimelineBlockDragCancel {
        item_id: ItemId,
    },
    /// 行选择：none=普通单选，shift=区间扩展，ctrl=切换。
    SelectRow {
        item_id: ItemId,
        modifier: SlintSelectionModifier,
    },
    /// 时间线画布框选（时间范围 × 行区间；none=替换，shift=加选，ctrl=切换）。
    /// row_lo/row_hi 为可见行（FilterModel）下标，x 为世界时间（秒）。
    MarqueeSelect {
        x_lo: f32,
        x_hi: f32,
        row_lo: i32,
        row_hi: i32,
        modifier: SlintSelectionModifier,
    },
    /// 时间线画布空白点击：清除选中。
    DeselectAll,
    AddLine {
        item_id: ItemId,
        end_time: TimeDelta,
        actor: Option<ActorId>,
        text: String,
        segment_timing: Option<SegmentTiming>,
        // FIXME: 之后需要更改为节点链
        style: Option<String>,
    },
    AddGroup {
        item_id: ItemId,
    },
    DeleteItems,
    /// 把当前选中集合写入剪贴板（只读 Domain，不修改）
    ///
    CopyItems,
    /// 复制 + 删除当前选中集合
    ///
    CutItems,
    /// 把剪贴板内的 `ItemsClip` 实例化到 `anchor` 之后/之内
    ///
    PasteItems {
        anchor: ItemId,
    },
    UnGroup {
        group_item_id: ItemId,
    },
}
