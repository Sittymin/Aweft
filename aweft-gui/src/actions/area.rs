use crate::bridge::types::SplitPreviewOverlayData;
use crate::screen::area::{AreaType, ScrAreaKey};
use crate::screen::primitives::Adjacency;

/// 区域相关 Action
#[derive(Debug, Clone)]
pub enum AreaAction {
    /// 切换区域类型
    SwitchType {
        area_key: ScrAreaKey,
        area_type: AreaType,
    },
    /// 分割区域
    Split {
        area_key: ScrAreaKey,
        preview_data: SplitPreviewOverlayData,
    },
    /// 合并区域
    Join {
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
        adjacency: Adjacency,
    },
}
