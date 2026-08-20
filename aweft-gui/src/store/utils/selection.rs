//! 选中修饰键语义：全局复用（点选、框选、通道/关键帧/手柄/NLA/时间线）。
//!
//! - `None`  = 替换（清空后单选）
//! - `Shift` = 加选（区间扩展/追加）
//! - `Ctrl`  = 切换（存在则取消）

use crate::bridge::types::SlintSelectionModifier;

pub fn is_replacing(m: SlintSelectionModifier) -> bool {
    m == SlintSelectionModifier::None
}
pub fn is_toggling(m: SlintSelectionModifier) -> bool {
    m == SlintSelectionModifier::Ctrl
}
pub fn is_additive(m: SlintSelectionModifier) -> bool {
    m == SlintSelectionModifier::Shift
}
