use aweft_model::ids::ItemId;

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
    },
    DecrementTimecode {
        event_id: ItemId,
        is_start_time: bool,
    },
    DragTimecodePreview {
        event_id: ItemId,
        is_start_time: bool,
        delta_x: f32,
    },
    DragTimecodeCommit {
        event_id: ItemId,
        is_start_time: bool,
        delta_x: f32,
    },
    DragTimecodeCancel {
        event_id: ItemId,
        is_start_time: bool,
    },
}
