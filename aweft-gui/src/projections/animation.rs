//! 动画投影：曲线编辑器（Graph Editor）/ 摄影表（Dope Sheet）派生层更新。
//!
//! 权威数据在编辑数据层（`GraphEditorState::channels_data`/`editing_keyframes`/UI 态）
//! 与 model（`Project.library.animation_library.actions`，松开时写回）。
//! reducer 只改数据层 + 打精确 tag；各 op 按 TagKind 路由，把数据层同步到
//! Slint 绑定模型（channels/keyframe_models/segments/handles/connections/summary/
//! selected_display）+ 标量。
//!
//! 依赖关系：关键帧/选中/通道操作都在结构变化（装载/重置/搜索）之后才有意义
//! （keyframe_models/过滤视图依赖 apply_structure 重建），因此 depends_on 结构操作。

use crate::depsgraph::Tag;
use crate::store::AppStore;

/// 动画：结构变化（装载/重置/搜索）→ 派生层全量重建 + 过滤视图重新绑定 Slint。
pub fn op_animation_structure(store: &mut AppStore, tags: &[Tag]) {
    let structure: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::AnimationStructureChanged))
        .cloned()
        .collect();
    if structure.is_empty() {
        return;
    }
    store.ui_models.graph_editor.apply_tags(&structure);
    store.rebind_animation_filters();
}

/// 动画：关键帧值变化（拖拽/插值/手柄）→ 嵌套模型同步 + 段/手柄/连接线/汇总增量重建。
pub fn op_animation_keyframes(store: &mut AppStore, tags: &[Tag]) {
    let keyframes: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::AnimationKeyframesChanged { .. }))
        .cloned()
        .collect();
    if keyframes.is_empty() {
        return;
    }
    store.ui_models.graph_editor.apply_tags(&keyframes);
}

/// 动画：选中变化 → 显示掩码/嵌套模型 selected/手柄选中/汇总掩码同步。
pub fn op_animation_selection(store: &mut AppStore, tags: &[Tag]) {
    let selection: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::AnimationSelectionChanged { .. }))
        .cloned()
        .collect();
    if selection.is_empty() {
        return;
    }
    store.ui_models.graph_editor.apply_tags(&selection);
}

/// 动画：通道属性变化 → 通道行同步 + 连接线/汇总重建。
pub fn op_animation_channel(store: &mut AppStore, tags: &[Tag]) {
    let channel: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::AnimationChannelChanged { .. }))
        .cloned()
        .collect();
    if channel.is_empty() {
        return;
    }
    store.ui_models.graph_editor.apply_tags(&channel);
}

/// 动画：拖拽结束整理（预览恢复/重排）→ 恢复编辑不变量。
/// 与选中变化分离：其他组件可单独依赖本整理（如 NLA 需要有序数据）。
pub fn op_animation_drag_ended(store: &mut AppStore, tags: &[Tag]) {
    let drag_ended: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::AnimationDragEnded))
        .cloned()
        .collect();
    if drag_ended.is_empty() {
        return;
    }
    store.ui_models.graph_editor.apply_tags(&drag_ended);
}

/// 动画：标量变化（播放头/当前通道/选中数/插值聚合）→ 同步到 Slint 全局。
/// 声明处理全部动画 TagKind（标量读数据层，与各 op 无执行顺序约束）。
pub fn op_animation_scalars(store: &mut AppStore, _tags: &[Tag]) {
    store.sync_animation_scalars();
}
