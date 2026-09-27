//! NLA 投影：派生层更新。
//!
//! 权威数据在编辑数据层（`NlaEditorState::track_view`/`editing_strips`/UI 态）
//! 与 model（`Project.library.animation_library.anim_datas`，松开时写回）。
//! reducer 只改数据层 + 打精确 tag；各 op 按 TagKind 路由，把数据层同步到
//! Slint 绑定模型（tracks/strip_models/tracks_filter）。
//!
//! 依赖关系：片断值操作（拖拽/选中）发生在装载之后（strip_models 依赖
//! apply_structure 重建），因此 depends_on 结构操作。

use crate::depsgraph::Tag;
use crate::store::AppStore;

/// NLA：结构变化（装载/切换编辑的动画片段/向下插入/搜索）→ 派生层全量重建 +
/// 过滤视图重新绑定 Slint + Header 下拉选中项同步。
pub fn op_nla_structure(store: &mut AppStore, tags: &[Tag]) {
    if !tags.iter().any(|t| matches!(t, Tag::NlaStructureChanged)) {
        return;
    }
    store.ui_models.animation.nla.apply_structure();
    store.rebind_nla_tracks();
    store.sync_nla_scalars();
}

/// NLA：片断选中变化→就地 set_row_data 选中列，不触碰位置/结构。
pub fn op_nla_selection(store: &mut AppStore, tags: &[Tag]) {
    let sel: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::NlaSelectionChanged { .. }))
        .cloned()
        .collect();
    if sel.is_empty() {
        return;
    }
    store.ui_models.animation.nla.apply_selection(&sel);
}

/// NLA：片断位置值变化（拖拽）→就地 set_row_data（保住 grab）。
pub fn op_nla_position(store: &mut AppStore, tags: &[Tag]) {
    let pos: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::NlaStripPositionChanged { .. }))
        .cloned()
        .collect();
    if pos.is_empty() {
        return;
    }
    store.ui_models.animation.nla.apply_position(&pos);
}

/// NLA：片断结构变化（跨轨移动/同轨重排/插入）→精确 remove+insert（row_added/removed，存量复用）。
pub fn op_nla_strip_structure(store: &mut AppStore, tags: &[Tag]) {
    let strukt: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::NlaStripStructureChanged { .. }))
        .cloned()
        .collect();
    if strukt.is_empty() {
        return;
    }
    store.ui_models.animation.nla.apply_strip_structure(&strukt);
}

/// NLA：轨道属性行更新（启用/锁定）→ 就地 set_row_data。
pub fn op_nla_track_properties(store: &mut AppStore, tags: &[Tag]) {
    let tracks: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::NlaTrackChanged { .. }))
        .cloned()
        .collect();
    if tracks.is_empty() {
        return;
    }
    store
        .ui_models
        .animation
        .nla
        .apply_track_properties(&tracks);
}

/// NLA：轨道结构新增→就地 row_added（追加轨道行与片断 inner，不重建存量）。
pub fn op_nla_track_added(store: &mut AppStore, tags: &[Tag]) {
    let tracks: Vec<Tag> = tags
        .iter()
        .filter(|t| matches!(t, Tag::NlaTrackAdded { .. }))
        .cloned()
        .collect();
    if tracks.is_empty() {
        return;
    }
    store.ui_models.animation.nla.apply_track_added(&tracks);
}
