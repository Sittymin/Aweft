use std::collections::HashMap;

// 字段分块
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    ScriptInfo,
    V4PStyles,
    Events,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct EventMargin(pub u16);

#[derive(Debug, Clone, Copy)]
pub enum TimeAdjustTarget {
    Start,
    End,
    Both,
}
#[derive(Debug)]
pub struct ASSFile {
    pub script_info: ScriptInfo,
    pub styles: HashMap<String, StyleWithoutName>,
    pub events: Vec<Event>,
    // TODO: 嵌入字体
}
