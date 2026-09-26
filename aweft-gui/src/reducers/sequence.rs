use std::collections::HashSet;

use anyhow::Result;
use aweft_model::ids::ItemId;
use aweft_model::sequence::ItemContent;
use aweft_project::Origin;
use chrono::TimeDelta;
use slint::Model;

use crate::actions::sequence::SequenceAction;
use crate::clipboard::ItemsClip;
use crate::depsgraph::Tag;
use crate::store::utils::selection::{is_additive, is_toggling};

/// Sequence 域 reducer（`impl AppStore`）：只改 Domain（`project.sequence`/选中）
/// 并返回精确 tag。UI 投影更新交给 depsgraph 投影重算（Sequence 投影依赖
/// `Sequence` / `Selection` 数据域）。
impl crate::store::AppStore {
    /// 可变访问字幕结构（model 纯值语义，直接 &mut）。
    fn sequence_mut(&mut self) -> &mut aweft_model::sequence::Sequence {
        self.state.project.sequence_mut()
    }

    /// 计算某个 item 的祖先分组链（叶子到根）。用于聚合传播。
    pub(crate) fn ancestor_chain(&self, item_id: ItemId) -> Vec<ItemId> {
        let mut chain = Vec::new();
        let mut cur = Some(item_id);
        while let Some(id) = cur {
            let sequence = self.state.project.sequence();
            let Some(item) = sequence.get(id) else {
                break;
            };
            if item.parent.is_some() {
                chain.push(id);
            }
            cur = item.parent;
        }
        chain
    }

    /// 构造单个时间码变化的 tag（带祖先链聚合传播）。
    pub(crate) fn event_timecode_tag(
        &self,
        event_id: ItemId,
        is_start_time: bool,
        time: TimeDelta,
    ) -> Tag {
        Tag::EventTimecodeChanged {
            event_id,
            is_start_time,
            time,
            ancestors: self.ancestor_chain(event_id),
        }
    }

    /// 构造批量时间码变化的 tag（updates 与祖先链去重收集）。
    fn events_timecode_tag(&self, ids: &[ItemId], is_start_time: bool, times: &[TimeDelta]) -> Tag {
        Tag::EventsTimecodeChanged {
            updates: ids
                .iter()
                .copied()
                .zip(std::iter::repeat(is_start_time))
                .zip(times.iter().copied())
                .map(|((id, s), t)| (id, s, t))
                .collect(),
            ancestors: ids.iter().flat_map(|&id| self.ancestor_chain(id)).collect(),
        }
    }

    /// 构造单个时间码拖拽预览的 tag（带祖先链聚合传播）。
    fn timecode_preview_tag(&self, event_id: ItemId, is_start_time: bool, time: TimeDelta) -> Tag {
        Tag::TimecodePreviewChanged {
            event_id,
            is_start_time,
            time,
            ancestors: self.ancestor_chain(event_id),
        }
    }

    /// 直接选中事件集 ∪ 当前事件。字幕行编辑器 Shift 批量调整与时间线
    /// 多选整体移动共用的目标收集：对 `event_id` 的操作按选中集扩散。
    fn selected_events_with(&self, event_id: ItemId) -> HashSet<ItemId> {
        let mut event_ids = self
            .ui_models
            .sequence_state
            .get_selected_events(self.state.project.sequence());
        // 确保当前 event_id 也在列表中（同时也避免重复）
        event_ids.insert(event_id);
        event_ids
    }

    /// 时间线片段整体拖拽的目标集：按住项在直接选中集内（事件或组头）时，
    /// 移动全部直接选中事件——若选中集含组头，「组头直接选中蕴含子树全选」
    /// （select_single 展开 + 计数派生维持），其子树事件已天然在内，整体一同移动；
    /// 按住未选中项时只动它自己。
    fn block_drag_targets(&self, item_id: ItemId) -> Vec<ItemId> {
        if !self
            .ui_models
            .sequence_state
            .rows_model()
            .directly_selected_items()
            .contains(&item_id)
        {
            return vec![item_id];
        }
        let mut ids: Vec<ItemId> = self
            .ui_models
            .sequence_state
            .get_selected_events(self.state.project.sequence())
            .into_iter()
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    /// 组头行当前聚合显示界 `(start, end)`。来自锚点快照——组块位置本就是
    /// 构建时烘焙的聚合值，拖拽跟随只需对其整体加 delta（内部偏移全部一致），
    /// 无需重新聚合。
    fn group_display_bounds(&self, gid: ItemId) -> Option<(TimeDelta, TimeDelta)> {
        let rows = self.ui_models.sequence_state.rows_model();
        let data = rows.row_data(rows.row_index_of(gid)?)?;
        let to_td = |sec: f32| TimeDelta::milliseconds((sec * 1000.0).round() as i64);
        Some((to_td(data.content.start_sec), to_td(data.content.end_sec)))
    }

    /// 时间线片段整体平移的目标时间（只读预览）：start/end 同加 delta，时长保持不变，
    /// 不做钳制（开始时间允许为负）。非事件项返回 None。
    fn preview_block_move(
        &self,
        event_id: ItemId,
        delta: TimeDelta,
    ) -> Option<(TimeDelta, TimeDelta)> {
        let item = self.state.project.sequence().get(event_id)?;
        let ItemContent::Event(e) = &item.content else {
            return None;
        };
        Some((e.start_time + delta, e.end_time + delta))
    }

    /// 提交整体平移（写 Domain）：语义同 [`Self::preview_block_move`]。
    /// 经 `ProjectStore` 的不钳制平移入口，回执进写日志投给插件。
    fn apply_block_move(
        &mut self,
        event_id: ItemId,
        delta: TimeDelta,
    ) -> Option<(TimeDelta, TimeDelta)> {
        self.state
            .project
            .shift_event_times(Origin::LocalGui, event_id, delta)
            .ok()
    }

    /// 构造选中变化的 tag（affected = 祖先链 + 自身）。
    fn selection_tag(&self, item_id: ItemId, kind: crate::depsgraph::SelectionKind) -> Tag {
        let mut affected = self.ancestor_chain(item_id);
        affected.push(item_id);
        Tag::SelectionChanged {
            item_id,
            kind,
            affected,
        }
    }

    /// 批量时间码调整的公共逻辑：收集选中事件 + 当前项，逐个调整并打批量 tag。
    ///
    /// `adjust` 决定调整方式（增量、预览、提交等），返回该次调整后的新时间；
    /// 失败的事件跳过。返回 `Some(tag)` 当有事件成功调整，否则 `None`。
    fn batch_timecode_tag<F>(
        &mut self,
        event_id: ItemId,
        is_start_time: bool,
        mut adjust: F,
    ) -> Option<Tag>
    where
        F: FnMut(&mut Self, ItemId) -> Option<TimeDelta>,
    {
        let event_ids: HashSet<ItemId> = self.selected_events_with(event_id);

        let mut updated_ids: Vec<ItemId> = Vec::new();
        let mut updated_times: Vec<TimeDelta> = Vec::new();
        for id in event_ids {
            if let Some(time) = adjust(self, id) {
                updated_ids.push(id);
                updated_times.push(time);
            }
        }
        if !updated_ids.is_empty() {
            Some(self.events_timecode_tag(&updated_ids, is_start_time, &updated_times))
        } else {
            None
        }
    }

    /// GUI 侧时间增减的唯一入口：经 `ProjectStore` 写入，回执进写日志投给插件。
    fn adjust_time(
        &mut self,
        event_id: ItemId,
        is_start_time: bool,
        delta: TimeDelta,
    ) -> Result<TimeDelta> {
        self.state
            .project
            .adjust_event_time(Origin::LocalGui, event_id, is_start_time, delta)
    }

    /// 处理 Sequence 相关 Action。
    ///
    /// 设计目标：本函数只做 Domain 修改，并向依赖图打 tag；UI 投影更新交给
    /// `Depsgraph` 的投影重算（Sequence 投影依赖 `Sequence` / `Selection` 数据域）。
    /// 时间写入一律经 `ProjectStore` 的方法（写入即广播，`dispatch` 的批量作用域把
    /// 一次交互合成一次通知）；结构、层级与文本尚无 `MutationKind`，
    /// 仍直接改 `sequence_mut()`。
    pub(crate) fn dispatch_sequence(&mut self, action: SequenceAction) -> Result<Vec<Tag>> {
        match action {
            SequenceAction::ToggleGroupCollapsed { group_id } => {
                // Rust 数据层修改 (持久化状态)
                if let Some(item) = self.sequence_mut().get_mut(group_id) {
                    if let ItemContent::Group(g) = &mut item.content {
                        g.collapsed = !g.collapsed;
                    }
                }

                Ok(vec![Tag::SequenceCollapsedToggled {
                    group_id,
                    ancestors: self.ancestor_chain(group_id),
                }])
            }
            SequenceAction::InputTimecode {
                event_id,
                text,
                is_start_time,
            } => {
                let time_result = self.state.project.set_event_time_fuzzy(
                    Origin::LocalGui,
                    event_id,
                    is_start_time,
                    &text,
                );
                if let Ok(time) = time_result {
                    return Ok(vec![self.event_timecode_tag(event_id, is_start_time, time)]);
                }
                Ok(vec![])
            }
            SequenceAction::IncrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            } => {
                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.adjust_time(id, is_start_time, TimeDelta::seconds(1)).ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let time = self
                        .adjust_time(event_id, is_start_time, TimeDelta::seconds(1))
                        .ok();
                    Ok(time
                        .map(|t| vec![self.event_timecode_tag(event_id, is_start_time, t)])
                        .unwrap_or_default())
                }
            }
            SequenceAction::DecrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            } => {
                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.adjust_time(id, is_start_time, TimeDelta::seconds(-1))
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let time = self
                        .adjust_time(event_id, is_start_time, TimeDelta::seconds(-1))
                        .ok();
                    Ok(time
                        .map(|t| vec![self.event_timecode_tag(event_id, is_start_time, t)])
                        .unwrap_or_default())
                }
            }
            SequenceAction::DragTimecodePreview {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            } => {
                log::debug!("DragTimecodePreview delta_x: {}", delta_x);
                if !crate::store::utils::drag::is_valid_delta(delta_x) {
                    return Ok(vec![]);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    // 批量预览：不写 sequence，构造批量 preview tag
                    let event_ids = self.selected_events_with(event_id);

                    let mut tags = Vec::new();
                    let mut ancestors: Vec<ItemId> = Vec::new();
                    for id in event_ids {
                        if let Ok(time) = self.state.project.sequence().preview_adjust_event_time(
                            id,
                            is_start_time,
                            delta,
                        ) {
                            tags.push(Tag::TimecodePreviewChanged {
                                event_id: id,
                                is_start_time,
                                time,
                                ancestors: self.ancestor_chain(id),
                            });
                            ancestors.extend(self.ancestor_chain(id));
                        }
                    }
                    Ok(tags)
                } else {
                    // 单个预览：不写 sequence，仅打 preview tag 供 UI 显示覆盖值
                    let time = self
                        .state
                        .project
                        .sequence()
                        .preview_adjust_event_time(event_id, is_start_time, delta)
                        .ok();
                    Ok(time
                        .map(|t| {
                            vec![Tag::TimecodePreviewChanged {
                                event_id,
                                is_start_time,
                                time: t,
                                ancestors: self.ancestor_chain(event_id),
                            }]
                        })
                        .unwrap_or_default())
                }
            }
            SequenceAction::DragTimecodeCommit {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            } => {
                log::debug!("DragTimecodeCommit delta_x: {}", delta_x);
                if !crate::store::utils::drag::is_valid_delta(delta_x) {
                    return Ok(vec![]);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.adjust_time(id, is_start_time, delta).ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let result = self.adjust_time(event_id, is_start_time, delta);
                    match result {
                        Ok(time) => {
                            Ok(vec![self.event_timecode_tag(event_id, is_start_time, time)])
                        }
                        Err(e) => {
                            log::warn!("{e}");
                            Ok(vec![])
                        }
                    }
                }
            }
            SequenceAction::DragTimecodeCancel {
                event_id,
                is_start_time,
                is_batch,
            } => {
                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.state
                                .project
                                .sequence()
                                .event_time(id, is_start_time)
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    // 单个取消（恢复到 Domain 当前值）
                    match self
                        .state
                        .project
                        .sequence()
                        .event_time(event_id, is_start_time)
                    {
                        Ok(time) => {
                            Ok(vec![self.event_timecode_tag(event_id, is_start_time, time)])
                        }
                        Err(_) => Ok(vec![]),
                    }
                }
            }
            SequenceAction::TimelineBlockDragPreview { item_id, delta_x } => {
                if !crate::store::utils::drag::is_valid_delta(delta_x) {
                    return Ok(vec![]);
                }
                let delta = TimeDelta::milliseconds((delta_x * 1000.0).round() as i64);
                // 整体移动预览：不写 sequence，每个目标 start/end 各打一条 preview tag。
                // 行映射器把覆盖同时应用到时间码文本与 start_sec/end_sec，
                // 字幕行编辑器与时间线块由同一条 Tag 一并驱动。
                let mut tags = Vec::new();
                for id in self.block_drag_targets(item_id) {
                    match self.preview_block_move(id, delta) {
                        Some((start, end)) => {
                            tags.push(self.timecode_preview_tag(id, true, start));
                            tags.push(self.timecode_preview_tag(id, false, end));
                        }
                        None => log::warn!("时间线片段预览跳过非事件项 id={id:?}"),
                    }
                }
                // 选中集内的组头（直接选中蕴含子树全选）：打聚合界覆盖，
                // 组块随子项实时平移（内部偏移全部一致）
                let rows = self.ui_models.sequence_state.rows_model();
                for h in rows.directly_selected_items() {
                    if !rows.is_group_item(h) {
                        continue;
                    }
                    if let Some((start, end)) = self.group_display_bounds(h) {
                        tags.push(self.timecode_preview_tag(h, true, start + delta));
                        tags.push(self.timecode_preview_tag(h, false, end + delta));
                    }
                }
                Ok(tags)
            }
            SequenceAction::TimelineBlockDragCommit { item_id, delta_x } => {
                if !crate::store::utils::drag::is_valid_delta(delta_x) {
                    return Ok(vec![]);
                }
                let delta = TimeDelta::milliseconds((delta_x * 1000.0).round() as i64);
                // 选中集内的组头（直接选中蕴含子树全选）：需清预览覆盖并刷新聚合
                let headers: Vec<ItemId> = self
                    .ui_models
                    .sequence_state
                    .rows_model()
                    .directly_selected_items()
                    .into_iter()
                    .filter(|id| {
                        self.ui_models
                            .sequence_state
                            .rows_model()
                            .is_group_item(*id)
                    })
                    .collect();
                let targets = self.block_drag_targets(item_id);
                let mut updates = Vec::new();
                let mut ancestors: Vec<ItemId> = Vec::new();
                for id in targets {
                    match self.apply_block_move(id, delta) {
                        Some((start, end)) => {
                            updates.push((id, true, start));
                            updates.push((id, false, end));
                            ancestors.extend(self.ancestor_chain(id));
                        }
                        None => log::warn!("时间线片段提交跳过非事件项 id={id:?}"),
                    }
                }
                // 组头条目：清各自预览覆盖 + 触发聚合行刷新
                //（新界由投影从 Domain 重算，此处快照值仅作记录）
                for h in &headers {
                    if let Some((start, end)) = self.group_display_bounds(*h) {
                        updates.push((*h, true, start));
                        updates.push((*h, false, end));
                        ancestors.extend(self.ancestor_chain(*h));
                    }
                }
                if updates.is_empty() {
                    return Ok(vec![]);
                }
                ancestors.sort_by_key(|id| id.0);
                ancestors.dedup();
                Ok(vec![Tag::EventsTimecodeChanged { updates, ancestors }])
            }
            SequenceAction::TimelineBlockDragCancel { item_id } => {
                // 恢复显示 Domain 当前值（投影清预览覆盖 + 刷新行 + 同步根级块）
                let targets = self.block_drag_targets(item_id);
                let seq = self.state.project.sequence();
                let mut updates = Vec::new();
                let mut ancestors: Vec<ItemId> = Vec::new();
                for id in targets {
                    if let (Ok(start), Ok(end)) =
                        (seq.event_time(id, true), seq.event_time(id, false))
                    {
                        updates.push((id, true, start));
                        updates.push((id, false, end));
                        ancestors.extend(self.ancestor_chain(id));
                    }
                }
                // 选中集内组头条目：清各自预览覆盖 + 刷新聚合行（Domain 未变，快照即当前值）
                for h in self
                    .ui_models
                    .sequence_state
                    .rows_model()
                    .directly_selected_items()
                {
                    if !self.ui_models.sequence_state.rows_model().is_group_item(h) {
                        continue;
                    }
                    if let Some((start, end)) = self.group_display_bounds(h) {
                        updates.push((h, true, start));
                        updates.push((h, false, end));
                        ancestors.extend(self.ancestor_chain(h));
                    }
                }
                if updates.is_empty() {
                    return Ok(vec![]);
                }
                ancestors.sort_by_key(|id| id.0);
                ancestors.dedup();
                Ok(vec![Tag::EventsTimecodeChanged { updates, ancestors }])
            }
            SequenceAction::SelectRow { item_id, modifier } => {
                let kind = if is_additive(modifier) {
                    crate::depsgraph::SelectionKind::Range
                } else if is_toggling(modifier) {
                    crate::depsgraph::SelectionKind::Toggle
                } else {
                    crate::depsgraph::SelectionKind::Single
                };
                Ok(vec![self.selection_tag(item_id, kind)])
            }
            SequenceAction::MarqueeSelect {
                x_lo,
                x_hi,
                row_lo,
                row_hi,
                modifier,
            } => Ok(vec![Tag::SelectionMarquee {
                x_lo,
                x_hi,
                row_lo,
                row_hi,
                modifier,
            }]),
            SequenceAction::DeselectAll => Ok(vec![Tag::SelectionDeselectedAll]),
            SequenceAction::AddLine {
                item_id,
                end_time,
                actor,
                text,
                segment_timing,
                // FIXME: 之后需要更改为节点链
                style,
            } => {
                let new_item_id = self.sequence_mut().insert_event_after(
                    item_id,
                    end_time,
                    actor,
                    text,
                    segment_timing,
                    style,
                )?;

                Ok(vec![
                    Tag::ItemsInserted {
                        item_id,
                        new_item_id,
                    },
                    self.selection_tag(new_item_id, crate::depsgraph::SelectionKind::Single),
                ])
            }
            SequenceAction::AddGroup { item_id } => {
                let group_count = self
                    .state
                    .project
                    .sequence()
                    .items
                    .values()
                    .filter(|item| matches!(item.content, ItemContent::Group(_)))
                    .count();

                let name = format!("新分组 {}", group_count + 1);

                let new_group_item_id = self.sequence_mut().insert_group_after(item_id, name)?;
                Ok(vec![
                    Tag::ItemsInserted {
                        item_id,
                        new_item_id: new_group_item_id,
                    },
                    self.selection_tag(new_group_item_id, crate::depsgraph::SelectionKind::Single),
                ])
            }
            SequenceAction::DeleteItems {} => {
                let removed = self.delete_currently_selected();
                Ok(vec![Tag::ItemsRemoved { item_ids: removed }])
            }
            SequenceAction::CopyItems => {
                self.copy_currently_selected()?;
                Ok(vec![])
            }
            SequenceAction::CutItems => {
                self.copy_currently_selected()?;
                let removed = self.delete_currently_selected();
                Ok(vec![Tag::ItemsRemoved { item_ids: removed }])
            }
            SequenceAction::UnGroup { group_item_id } => {
                let new_ids = self.sequence_mut().ungroup(group_item_id).map_err(|e| {
                    log::error!("解散分组失败 {:?}: {}", group_item_id, e);
                    e
                })?;
                // UI 更新：移除旧分组行 + 插入提升的子项行（精确锚点）。
                // 选中联动：子项提升后选中新根（reducer 声明数据变化，投影按 tag 原子执行）。
                let mut tags = vec![Tag::GroupsUngrouped {
                    group_id: group_item_id,
                    new_ids: new_ids.clone(),
                }];
                if let (Some(&first), Some(&last)) = (new_ids.first(), new_ids.last()) {
                    tags.push(self.selection_tag(first, crate::depsgraph::SelectionKind::Single));
                    tags.push(self.selection_tag(last, crate::depsgraph::SelectionKind::Range));
                }
                Ok(tags)
            }
            SequenceAction::PasteItems { anchor } => {
                let Some(clip) = self.clipboard.paste_items()? else {
                    log::info!("剪贴板没有可粘贴的内容");
                    return Ok(vec![]);
                };
                let (new_root_ids, base_location) =
                    clip.instantiate(&mut self.sequence_mut(), anchor)?;
                // 结构变化 + 选中联动（粘贴后选中新内容首个根）。
                let mut tags = vec![Tag::ItemsPasted {
                    new_root_ids: new_root_ids.clone(),
                    base_location,
                }];
                if let Some(&first) = new_root_ids.first() {
                    tags.push(self.selection_tag(first, crate::depsgraph::SelectionKind::Single));
                }
                Ok(tags)
            }
        }
    }

    /// 从当前选中集合构造 `ItemsClip` 并写入剪贴板，返回 root 数量。
    /// 选中为空时返回 0 且不写入剪贴板（保留旧内容）
    // NOTE: 也许输入为空的时候清空当前剪贴板更好？
    fn copy_currently_selected(&mut self) -> Result<usize> {
        let selected = self.ui_models.sequence_state.directly_selected_items();
        if selected.is_empty() {
            return Ok(0);
        }
        let clip = ItemsClip::from_selection(&*self.state.project.sequence(), &selected);
        let count = clip.root_count();
        self.clipboard.copy_items(&clip)?;
        Ok(count)
    }

    /// 从 Domain 删除当前选中集合（过滤"祖先已选中"），返回实际被移除的 root id 列表。
    fn delete_currently_selected(&mut self) -> Vec<ItemId> {
        let selected_set = self.ui_models.sequence_state.directly_selected_items();
        let to_delete: Vec<ItemId> = selected_set
            .iter()
            .copied()
            .filter(|id| {
                !&self
                    .state
                    .project
                    .sequence()
                    .has_ancestor_in_set(id, &selected_set)
            })
            .collect();

        let mut removed: Vec<ItemId> = Vec::with_capacity(to_delete.len());
        for id in to_delete {
            match self.sequence_mut().remove_item(id) {
                Ok(()) => removed.push(id),
                Err(e) => log::warn!("删除 sequence 项失败 id={:?}: {}", id, e),
            }
        }
        removed
    }
}
