use crate::screen::area::{AreaType, ScrAreaKey};
use crate::screen::edge::ScrEdgeKey;
use std::rc::Rc;

/// 状态变更描述 - 精确描述哪些数据被修改了
#[derive(Debug, Clone)]
pub enum StateChange {
    /// 无变更
    None,

    /// 屏幕布局变更
    Screen(ScreenChange),

    /// 区域变更
    Areas(AreasChange),

    /// 拖拽状态变更
    DragOperation(DragOperationChange),

    LineEditor(LineEditorChange),

    /// 组合变更（多个同时发生）
    Multiple(Vec<StateChange>),
}

#[derive(Debug, Clone)]
pub enum ScreenChange {
    /// 布局初始化（全量更新）
    Initialized,
    /// 窗口大小改变（全量更新 areas + edges）
    Resized {
        old_size: (u32, u32),
        new_size: (u32, u32),
    },
    /// 缩放因子改变（全量更新 areas + edges）
    ScaleChanged { old_scale: f64, new_scale: f64 },
}

#[derive(Debug, Clone)]
pub enum AreasChange {
    /// 单个区域类型改变（只更新一个区域）
    TypeChanged {
        area_key: ScrAreaKey,
        old_type: AreaType,
        new_type: AreaType,
    },
    /// 区域分割（增加区域 + 更新边缘）
    Split {
        parent_key: ScrAreaKey,
        new_area_key: ScrAreaKey,
    },
    /// 区域合并（减少区域 + 更新边缘）
    Joined {
        removed_keys: Vec<ScrAreaKey>,
        merged_into: ScrAreaKey,
    },
}

#[derive(Debug, Clone)]
pub enum DragOperationChange {
    /// 更新拖拽预览（只更新覆盖层）
    SplitPreviewUpdated,
    JoinPreviewUpdated,
    PreviewCleared,
    EdgeUpdated {
        // NEED_CHECK: UI 局部更新时涉及到的 Edge 和 Area
        edge_keys_rc: Rc<Vec<ScrEdgeKey>>,
        area_keys_rc: Rc<Vec<ScrAreaKey>>,
    },
    /// 结束拖拽（隐藏覆盖层 + 可能的区域变更）
    Ended {
        areas_changed: bool,
    },
}

#[derive(Debug, Clone)]
pub enum LineEditorChange {
    GroupCollapsedToggled,
    TimecodeInputed,
    TimecodeIncremented,
    TimecodeDecremented,
    TimecodePreviewDraged,
    TimecodeCommitDraged,
    TimecodeCancelDraged,
    SelectionChanged,
    ItemsInserted,
}
