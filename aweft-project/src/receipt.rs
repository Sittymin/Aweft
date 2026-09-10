use aweft_model::ids::ItemId;
use std::fmt;

/// 变更来源标记。写日志按它分发：来源方自己触发的写入不需要再回投给自己，
/// 否则插件写一次就会被自己再通知一次。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    LocalGui,
    LocalCli,
    Plugin(String),
    Collab(String),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::LocalGui => write!(f, "local-gui"),
            Origin::LocalCli => write!(f, "local-cli"),
            Origin::Plugin(id) => write!(f, "plugin:{id}"),
            Origin::Collab(id) => write!(f, "collab:{id}"),
        }
    }
}

/// 单行时间变更明细：哪一行、起止时间变成了什么（毫秒）。
#[derive(Debug, Clone)]
pub struct EventTimeChanged {
    pub item: ItemId,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// 变更种类。首版只含时间变更，后续追加文本/结构/布局变更。
#[derive(Debug, Clone)]
pub enum MutationKind {
    EventTimeChanged(EventTimeChanged),
}

/// 一次写入的回执：来源 + 变更明细。
/// 图形界面把插件等外部来源的回执映射为 `Tag` 刷新视图，
/// 插件运行时把时间变更回执转给订阅了该事件的 Guest。
#[derive(Debug, Clone)]
pub struct MutationReceipt {
    pub origin: Origin,
    pub kind: MutationKind,
}
