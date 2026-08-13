use aweft_model::{
    ids::{ActorId, ItemId},
    structure::SegmentTiming,
};
use chrono::TimeDelta;

#[derive(Debug, Clone)]
pub enum LineEditorAction {
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
    SelectSingle {
        item_id: ItemId,
    },
    SelectRange {
        item_id: ItemId,
    },
    SelectToggle {
        item_id: ItemId,
    },
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
