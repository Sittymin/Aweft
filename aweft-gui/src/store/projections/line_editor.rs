//! LineEditor 投影操作：按 TagKind 拆分为原子更新动作。

use aweft_model::ids::ItemId;
use chrono::TimeDelta;

use crate::depsgraph::Tag;
use crate::store::AppStore;

/// 行结构操作：处理行/分组增删、层级调整。
///
/// 所有结构变化都用精确锚点（row_added/row_removed）。`StructureChanged`
/// 不应被 reducer 产生——若出现说明有未精确化的结构操作，明确报错而非全量兜底。
pub fn op_line_structure(store: &mut AppStore, tags: &[Tag]) {
    // 结构变化 → 事件数缓存失效（Blender 式粗粒度：投影收到结构 tag
    // 即该数据域整体变了，缓存下次 select 时懒重建，无需精确到组）。
    let (_, ui) = store.split_mut();
    ui.line_editor_state.invalidate_event_counts();

    for tag in tags {
        match tag {
            Tag::StructureChanged => {
                log::error!(
                    "depsgraph: 收到未精确化的 StructureChanged——所有结构操作必须产生具体 tag（ItemsInserted/Removed/Pasted/Ungrouped）"
                );
            }
            Tag::ItemsInserted {
                item_id,
                new_item_id,
            } => {
                let (state, ui) = store.split_mut();
                if let Err(err) = ui.line_editor_state.handle_item_inserted(
                    &state.project,
                    *item_id,
                    *new_item_id,
                ) {
                    log::error!("LineEditor 插入行失败: {}", err);
                }
            }
            Tag::ItemsRemoved { item_ids } => {
                let (state, ui) = store.split_mut();
                ui.line_editor_state
                    .handle_items_removed(&state.project, item_ids);
            }
            Tag::ItemsPasted {
                new_root_ids,
                base_location,
            } => {
                let (state, ui) = store.split_mut();
                if let Err(err) = ui.line_editor_state.handle_subtrees_pasted(
                    &state.project,
                    *base_location,
                    new_root_ids,
                ) {
                    log::error!("LineEditor 粘贴子树失败: {}", err);
                }
            }
            Tag::GroupsUngrouped { group_id, new_ids } => {
                let (state, ui) = store.split_mut();
                if let Err(err) =
                    ui.line_editor_state
                        .handle_group_ungrouped(&state.project, *group_id, new_ids)
                {
                    log::error!("LineEditor 解散分组失败: {}", err);
                }
            }

            _ => {}
        }
    }
}

/// 行时间码操作：处理时间码值变化（单行/批量/预览/取消）。
pub fn op_line_timecode(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        match tag {
            Tag::EventTimecodeChanged {
                event_id,
                is_start_time,
                time,
                ancestors: _,
            } => {
                let (state, ui) = store.split_mut();
                // commit/cancel 后清除预览覆盖（提交已写 structure，cancel 恢复原值）
                ui.line_editor_state
                    .rows_model()
                    .clear_preview_timecode(*event_id);
                // update_row_time 内部重建该行 + 所有祖先分组头并通知（原子刷新）
                ui.line_editor_state.update_row_time(
                    &state.project,
                    *event_id,
                    *is_start_time,
                    *time,
                );
            }
            Tag::TimecodePreviewChanged {
                event_id,
                is_start_time,
                time,
                ancestors,
            } => {
                // 拖拽预览：写覆盖表（不修改 structure），UI 显示覆盖值
                let (_, ui) = store.split_mut();
                let rows = ui.line_editor_state.rows_model();
                rows.set_preview_timecode(*event_id, *is_start_time, *time);
                if let Some(idx) = rows.row_index_of(*event_id) {
                    rows.notify_row_changed(idx);
                }
                for anc in ancestors {
                    if let Some(idx) = rows.row_index_of(*anc) {
                        rows.notify_row_changed(idx);
                    }
                }
            }
            Tag::EventsTimecodeChanged {
                updates,
                ancestors: _,
                ..
            } => {
                let (state, ui) = store.split_mut();
                let rows = ui.line_editor_state.rows_model();
                // 批量 commit/cancel 后清除预览覆盖
                for (id, _, _) in updates {
                    rows.clear_preview_timecode(*id);
                }
                let ids: Vec<ItemId> = updates.iter().map(|(id, _, _)| *id).collect();
                let times: Vec<TimeDelta> = updates.iter().map(|(_, _, t)| *t).collect();
                let is_start = updates.first().map(|(_, s, _)| *s).unwrap_or(false);
                // 内部重建所有受影响行 + 祖先分组头并通知（原子刷新）
                ui.line_editor_state.update_rows_batch_then_ancestors(
                    &state.project,
                    &ids,
                    is_start,
                    &times,
                );
            }
            _ => {}
        }
    }
}

/// 分组折叠操作。
pub fn op_line_collapse(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        if let Tag::GroupCollapsedToggled { group_id, .. } = tag {
            let (_, ui) = store.split_mut();
            ui.line_editor_state.toggle_group(&group_id.to_string());
        }
    }
}

/// 选中操作：处理选中状态变化（按选择类型路由，避免 toggle/range 被降级为单选）。
pub fn op_line_selection(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        if let Tag::SelectionChanged {
            item_id,
            kind,
            affected,
        } = tag
        {
            let (state, ui) = store.split_mut();
            match kind {
                crate::depsgraph::SelectionKind::Single => {
                    ui.line_editor_state.select_single(&state.project, *item_id);
                }
                crate::depsgraph::SelectionKind::Range => {
                    ui.line_editor_state.select_range(&state.project, *item_id);
                }
                crate::depsgraph::SelectionKind::Toggle => {
                    ui.line_editor_state.select_toggle(&state.project, *item_id);
                }
            }
            // 聚合传播：更新所有受影响的祖先分组头选中态
            for id in affected {
                if let Some(idx) = ui.line_editor_state.rows_model().row_index_of(*id) {
                    ui.line_editor_state.rows_model().notify_row_changed(idx);
                }
            }
        }
    }
}
