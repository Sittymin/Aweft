use super::NlaStripData;
use crate::bridge::types::{SlintNlaStrip, SlintStripType};
use aweft_model::{
    animation::strip::{NlaStrip, StripType},
    ids::ActionId,
};

impl NlaStripData {
    /// 从 model 片断构造编辑数据（`target_duration` 用于解析相对/填充 placement）。
    pub(crate) fn from_model(id: super::StripId, strip: &NlaStrip, target_duration: f32) -> Self {
        let (start, end) = strip.placement.resolve(target_duration.max(1e-9) as f64);
        let strip_type = match &strip.strip_type {
            StripType::Action { .. } => SlintStripType::Action,
            StripType::Transition => SlintStripType::Transition,
            StripType::Meta { .. } => SlintStripType::Meta,
        };
        Self {
            id,
            view: SlintNlaStrip {
                name: strip.name.clone().into(),
                strip_type,
                start: start as f32,
                end: end as f32,
                muted: strip.muted,
                selected: false,
                dragging: false,
            },
            model: strip.clone(),
        }
    }

    /// 构造一个 Action 型片断（"向下插入"用）。
    pub(crate) fn new_action(
        id: super::StripId,
        name: impl Into<String>,
        action: ActionId,
        start: f32,
        end: f32,
    ) -> Self {
        let name = name.into();
        Self {
            id,
            view: SlintNlaStrip {
                name: name.clone().into(),
                strip_type: SlintStripType::Action,
                start,
                end,
                muted: false,
                selected: false,
                dragging: false,
            },
            model: NlaStrip::new_action(name, action),
        }
    }
}
