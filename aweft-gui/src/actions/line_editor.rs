use aweft_model::ids::ItemId;

#[derive(Debug, Clone)]
pub enum LineEditorAction {
    ToggleGroupCollapsed {
        group_id: ItemId,
    },
    InputTimecode {
        event_id: ItemId,
        text: String,
        is_start_time: bool,
    },
}
