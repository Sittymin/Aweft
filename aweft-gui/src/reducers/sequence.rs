use std::collections::HashSet;

use anyhow::Result;
use aweft_model::ids::ItemId;
use aweft_model::sequence::ItemContent;
use chrono::TimeDelta;

use crate::actions::sequence::SequenceAction;
use crate::clipboard::ItemsClip;
use crate::depsgraph::Tag;

/// Sequence 域 reducer（`impl AppStore`）：只改 Domain（`project.sequence`/选中）
/// 并返回精确 tag。UI 投影更新交给 depsgraph 投影重算（Sequence 投影依赖
/// `Sequence` / `Selection` 数据域）。
impl crate::store::AppStore {
    /// 可变访问字幕结构（model 纯值语义，直接 &mut）。
    fn sequence_mut(&mut self) -> &mut aweft_model::sequence::Sequence {
        self.state.project.sequence_mut()
    }

    /// 计算某个 item 的祖先分组链（叶子到根）。用于聚合传播。
    fn ancestor_chain(&self, item_id: ItemId) -> Vec<ItemId> {
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
    fn event_timecode_tag(&self, event_id: ItemId, is_start_time: bool, time: TimeDelta) -> Tag {
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
        let mut event_ids: HashSet<ItemId> = self
            .ui_models
            .sequence_state
            .get_selected_events(&self.state.project)
            .into_iter()
            .collect();
        // 确保当前 event_id 也在列表中（同时也避免重复）
        event_ids.insert(event_id);

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
    // 处理 Sequence 相关 Action。
    //
    // 设计目标：本函数只做 Domain（`project.sequence`）修改，并向依赖图打 tag；
    // UI 投影更新交给 `Depsgraph` 的投影重算（Sequence 投影依赖
    // `Sequence` / `Selection` 数据域）。
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
                let time_result =
                    self.sequence_mut()
                        .set_event_time_fuzzy(event_id, is_start_time, &text);
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
                            s.sequence_mut()
                                .adjust_event_time(id, is_start_time, TimeDelta::seconds(1))
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let time = self
                        .sequence_mut()
                        .adjust_event_time(event_id, is_start_time, TimeDelta::seconds(1))
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
                            s.sequence_mut()
                                .adjust_event_time(id, is_start_time, TimeDelta::seconds(-1))
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let time = self
                        .sequence_mut()
                        .adjust_event_time(event_id, is_start_time, TimeDelta::seconds(-1))
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

                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(vec![]);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    // 批量预览：不写 sequence，构造批量 preview tag
                    let mut event_ids: HashSet<ItemId> = self
                        .ui_models
                        .sequence_state
                        .get_selected_events(&self.state.project)
                        .into_iter()
                        .collect();
                    event_ids.insert(event_id);

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

                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(vec![]);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.sequence_mut()
                                .adjust_event_time(id, is_start_time, delta)
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let result =
                        self.sequence_mut()
                            .adjust_event_time(event_id, is_start_time, delta);
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
            SequenceAction::SelectRow { item_id, modifier } => {
                let kind = match modifier {
                    crate::bridge::types::SlintSelectionModifier::Shift => {
                        crate::depsgraph::SelectionKind::Range
                    }
                    crate::bridge::types::SlintSelectionModifier::Ctrl => {
                        crate::depsgraph::SelectionKind::Toggle
                    }
                    crate::bridge::types::SlintSelectionModifier::None => {
                        crate::depsgraph::SelectionKind::Single
                    }
                };
                Ok(vec![self.selection_tag(item_id, kind)])
            }
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
