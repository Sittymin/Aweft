use std::collections::HashSet;

use crate::actions::Action;
use crate::actions::area::AreaAction;
use crate::actions::line_editor::LineEditorAction;
use crate::clipboard::{Clipboard, ItemsClip};
use crate::store::state::AppState;
use crate::store::state_change::{LineEditorChange, StateChange};
use crate::store::ui_models::UiModels;
use anyhow::Result;
use aweft_model::ids::ItemId;
use aweft_model::structure::{ItemContent, ItemLocation};
use chrono::TimeDelta;

/// 应用状态存储 - 单一数据源
pub struct AppStore {
    state: AppState,
    ui_models: UiModels,
    clipboard: Clipboard,
}

impl AppStore {
    /// 创建新的 Store（从 MainWindow 以访问主题系统）
    pub fn new(main_window: &crate::bridge::types::MainWindow) -> Self {
        use slint::ComponentHandle;

        let theme_settings = crate::bridge::types::ThemeSettings::from_main_window(main_window);
        Self {
            state: AppState::new(&main_window.window(), theme_settings),
            ui_models: UiModels::new(),
            clipboard: Clipboard::new(),
        }
    }

    /// 分发 Action，返回状态变更描述
    pub fn dispatch(&mut self, action: Action) -> Result<StateChange> {
        let change = match action {
            Action::Screen(screen_action) => self.dispatch_screen(screen_action),
            Action::Drag(drag_action) => self.dispatch_drag(drag_action),
            Action::Area(area_action) => self.dispatch_area(area_action),
            Action::Media(_media_action) => {
                // TODO: 实现媒体相关的 reducer
                Ok(StateChange::None)
            }
            Action::LineEditor(act) => self.dispatch_line_editor(act),
        };

        change
    }

    /// 获取当前状态（只读）
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// 获取可变状态（内部使用）
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// 获取 UI 模型（只读）
    pub fn ui_models(&self) -> &UiModels {
        &self.ui_models
    }

    /// 获取 UI 模型（可变）
    pub fn ui_models_mut(&mut self) -> &mut UiModels {
        &mut self.ui_models
    }

    /// 同时获取只读 `state` 与可变 `ui_models`。
    ///
    /// 用于需要“读取 Domain、写入 UI Projection”的场景（典型是 `UIUpdater` 应用 LineEditor 投影更新）。
    pub fn split_mut(&mut self) -> (&AppState, &mut UiModels) {
        (&self.state, &mut self.ui_models)
    }

    /// 重建 LineEditor 的行数据与过滤模型，返回过滤后的 ModelRc。
    pub fn rebuild_line_editor_models(
        &mut self,
    ) -> slint::ModelRc<crate::bridge::types::SlintSubtitleRow> {
        self.ui_models
            .line_editor_state
            .rebuild_all_rows(&self.state.project);
        self.ui_models.line_editor_state.init_filter_model()
    }

    // ========== 内部 dispatch 方法 ==========

    fn dispatch_screen(
        &mut self,
        action: crate::actions::screen::ScreenAction,
    ) -> Result<StateChange> {
        use crate::actions::screen::ScreenAction;

        match action {
            ScreenAction::WindowResized { width, height } => {
                crate::store::reducers::screen::handle_resize(&mut self.state, width, height)
            }
            ScreenAction::ScaleFactorChanged { scale_factor } => {
                crate::store::reducers::screen::handle_scale_change(&mut self.state, scale_factor)
            }
            ScreenAction::InitializeLayout { width, height } => {
                crate::store::reducers::screen::handle_initialize(&mut self.state, width, height)
            }
        }
    }

    fn dispatch_drag(&mut self, action: crate::actions::drag::DragAction) -> Result<StateChange> {
        use crate::actions::drag::DragAction;

        match action {
            DragAction::ActionZone(az_action) => self.dispatch_actionzone_drag(az_action),
            DragAction::Edge(edge_action) => self.dispatch_edge_drag(edge_action),
        }
    }

    fn dispatch_actionzone_drag(
        &mut self,
        action: crate::actions::drag::ActionZoneDrag,
    ) -> Result<StateChange> {
        use crate::actions::drag::ActionZoneDrag;

        match action {
            ActionZoneDrag::Update {
                area_key,
                corner,
                offset_x,
                offset_y,
            } => crate::store::reducers::drag::handle_actionzone_update(
                &mut self.state,
                area_key,
                corner,
                offset_x,
                offset_y,
            ),
            ActionZoneDrag::End {
                area_key,
                is_cancel,
            } => crate::store::reducers::drag::handle_actionzone_end(
                &mut self.state,
                area_key,
                is_cancel,
            ),
        }
    }

    fn dispatch_edge_drag(
        &mut self,
        action: crate::actions::drag::EdgeDrag,
    ) -> Result<StateChange> {
        use crate::actions::drag::EdgeDrag;

        match action {
            EdgeDrag::Update {
                vert_keys,
                offset_x,
                offset_y,
            } => crate::store::reducers::drag::handle_edge_update(
                &mut self.state,
                vert_keys,
                offset_x,
                offset_y,
            ),
            EdgeDrag::End { is_cancel } => {
                crate::store::reducers::drag::handle_edge_end(&mut self.state, is_cancel)
            }
        }
    }

    fn dispatch_area(&mut self, action: AreaAction) -> Result<StateChange> {
        use crate::actions::area::AreaAction;

        match action {
            AreaAction::SwitchType {
                area_key,
                area_type,
            } => crate::store::reducers::area::handle_switch_type(
                &mut self.state,
                area_key,
                area_type,
            ),
            AreaAction::Split {
                area_key,
                preview_data,
            } => {
                crate::store::reducers::area::handle_split(&mut self.state, area_key, preview_data)
            }
            AreaAction::Join {
                source_area_key,
                target_area_key,
                adjacency,
            } => crate::store::reducers::area::handle_join(
                &mut self.state,
                source_area_key,
                target_area_key,
                adjacency,
            ),
        }
    }
    // 处理 LineEditor 相关 Action。
    //
    // 设计目标：本函数只做 Domain（`project.structure`）修改，UI 投影更新交给
    // `UIUpdater`（通过返回的 `LineEditorChange`）。
    //
    // 当前已完成 Domain ↔ UI 拆分的分支：
    // - `ToggleGroupCollapsed`
    // - `InputTimecode`
    // - `IncrementTimecode` / `DecrementTimecode`（仅单条）
    // - `DragTimecodePreview` / `DragTimecodeCommit` / `DragTimecodeCancel`（仅单条）
    // - `SelectSingle` / `SelectRange` / `SelectToggle`
    // - `AddLine` / `DeleteItems`
    //
    // FIXME: 仍混合了 UI 投影更新（is_batch == true 的分支）：
    // - `IncrementTimecode { is_batch: true }`
    // - `DecrementTimecode { is_batch: true }`
    // - `DragTimecodePreview { is_batch: true }`
    // - `DragTimecodeCommit { is_batch: true }`
    // - `DragTimecodeCancel { is_batch: true }`
    //
    // 这些批量分支需要新设计 payload（建议引入
    // `LineEditorChange::TimecodesBatchUpdated { updates, with_ancestor_recalc }`），
    // 当前先保留以维持行为不变。
    fn dispatch_line_editor(&mut self, action: LineEditorAction) -> Result<StateChange> {
        match action {
            LineEditorAction::ToggleGroupCollapsed { group_id } => {
                // Rust 数据层修改 (持久化状态)
                if let Some(item) = self.state.project.structure.get_mut(group_id) {
                    if let ItemContent::Group(g) = &mut item.content {
                        g.collapsed = !g.collapsed;
                    }
                }

                Ok(StateChange::LineEditor(
                    LineEditorChange::GroupCollapsedToggled {
                        group_id: group_id.to_string(),
                    },
                ))
            }
            LineEditorAction::InputTimecode {
                event_id,
                text,
                is_start_time,
            } => {
                if let Ok(time) = self.state.project.structure.set_event_time_fuzzy(
                    event_id,
                    is_start_time,
                    &text,
                ) {
                    return Ok(StateChange::LineEditor(LineEditorChange::TimecodeInputed {
                        event_id,
                        is_start_time,
                        time,
                    }));
                }
                Ok(StateChange::None)
            }
            LineEditorAction::IncrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            } => {
                let delta = TimeDelta::seconds(1);

                if is_batch {
                    // 批量调整：获取所有选中的事件项
                    let mut event_ids: Vec<ItemId> = self
                        .ui_models
                        .line_editor_state
                        .get_selected_events(&self.state.project)
                        .into_iter()
                        .collect();

                    // 确保当前 event_id 也在列表中（同时也避免重复）
                    if !event_ids.contains(&event_id) {
                        event_ids.push(event_id);
                    }

                    // 批量调整
                    let mut updated_count = 0;
                    let total_count = event_ids.len();

                    for (index, &id) in event_ids.iter().enumerate() {
                        if let Ok(time) =
                            self.state
                                .project
                                .structure
                                .adjust_event_time(id, is_start_time, delta)
                        {
                            // 最后一个事件时更新分组头，其他事件只更新自身
                            let is_last = index == total_count - 1;

                            if is_last {
                                // 最后一个：使用 update_row_time 更新分组头
                                self.ui_models
                                    .line_editor_state
                                    .update_row_time_full_bounds_recursive(
                                        &self.state.project,
                                        id,
                                        is_start_time,
                                        time,
                                    );
                            } else {
                                // 其他事件：只更新自身，不更新分组头
                                self.ui_models
                                    .line_editor_state
                                    .update_row_time_without_ancestor_groups(
                                        id,
                                        is_start_time,
                                        time,
                                    );
                            }
                            updated_count += 1;
                        }
                    }

                    // FIXME: 批量分支尚未走 StateChange/UIUpdater 管道，
                    // 此处直接修改了 UI 投影；后续应引入 `TimecodesBatchUpdated` payload。
                    if updated_count > 0 {
                        return Ok(StateChange::None);
                    }
                } else {
                    // 单个调整
                    if let Ok(time) = self.state.project.structure.adjust_event_time(
                        event_id,
                        is_start_time,
                        delta,
                    ) {
                        return Ok(StateChange::LineEditor(
                            LineEditorChange::TimecodeIncremented {
                                event_id,
                                is_start_time,
                                time,
                            },
                        ));
                    }
                }
                Ok(StateChange::None)
            }
            LineEditorAction::DecrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            } => {
                let delta = TimeDelta::seconds(-1);

                if is_batch {
                    // 批量调整：获取所有选中的事件项
                    let mut event_ids: Vec<ItemId> = self
                        .ui_models
                        .line_editor_state
                        .get_selected_events(&self.state.project)
                        .into_iter()
                        .collect();

                    // 确保当前 event_id 也在列表中（同时也避免重复）
                    if !event_ids.contains(&event_id) {
                        event_ids.push(event_id);
                    }

                    // 批量调整
                    let mut updated_count = 0;
                    let total_count = event_ids.len();

                    for (index, &id) in event_ids.iter().enumerate() {
                        if let Ok(time) =
                            self.state
                                .project
                                .structure
                                .adjust_event_time(id, is_start_time, delta)
                        {
                            // 最后一个事件时更新分组头，其他事件只更新自身
                            let is_last = index == total_count - 1;

                            if is_last {
                                // 最后一个：使用 update_row_time 更新分组头
                                self.ui_models
                                    .line_editor_state
                                    .update_row_time_full_bounds_recursive(
                                        &self.state.project,
                                        id,
                                        is_start_time,
                                        time,
                                    );
                            } else {
                                // 其他事件：只更新自身，不更新分组头
                                self.ui_models
                                    .line_editor_state
                                    .update_row_time_without_ancestor_groups(
                                        id,
                                        is_start_time,
                                        time,
                                    );
                            }
                            updated_count += 1;
                        }
                    }

                    // FIXME: 批量分支尚未走 StateChange/UIUpdater 管道，
                    // 此处直接修改了 UI 投影；后续应引入 `TimecodesBatchUpdated` payload。
                    if updated_count > 0 {
                        return Ok(StateChange::None);
                    }
                } else {
                    // 单个调整
                    if let Ok(time) = self.state.project.structure.adjust_event_time(
                        event_id,
                        is_start_time,
                        delta,
                    ) {
                        return Ok(StateChange::LineEditor(
                            LineEditorChange::TimecodeDecremented {
                                event_id,
                                is_start_time,
                                time,
                            },
                        ));
                    }
                }
                Ok(StateChange::None)
            }
            LineEditorAction::DragTimecodePreview {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            } => {
                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(StateChange::None);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    let mut event_ids: HashSet<ItemId> = self
                        .ui_models
                        .line_editor_state
                        .get_selected_events(&self.state.project);

                    // 确保当前 event_id 也在列表中（同时也避免重复）
                    event_ids.insert(event_id);

                    let mut batch_event_ids: Vec<ItemId> = Vec::new();
                    let mut batch_times: Vec<TimeDelta> = Vec::new();

                    // 批量预览调整
                    for id in event_ids {
                        match self.state.project.structure.preview_adjust_event_time(
                            id,
                            is_start_time,
                            delta,
                        ) {
                            Ok(time) => {
                                batch_event_ids.push(id);
                                batch_times.push(time);
                            }
                            Err(err) => {
                                log::error!(
                                    "Failed to preview adjust event time for id {}: {:?}",
                                    id,
                                    err
                                );
                            }
                        }
                    }
                    if !batch_event_ids.is_empty() && !batch_times.is_empty() {
                        return Ok(StateChange::LineEditor(
                            LineEditorChange::TimecodesBatchPreviewDraged {
                                event_ids: batch_event_ids,
                                is_start_time,
                                times: batch_times,
                            },
                        ));
                    }
                } else {
                    // 单个调整
                    if let Ok(time) = self.state.project.structure.preview_adjust_event_time(
                        event_id,
                        is_start_time,
                        delta,
                    ) {
                        return Ok(StateChange::LineEditor(
                            LineEditorChange::TimecodePreviewDraged {
                                event_id,
                                is_start_time,
                                time,
                            },
                        ));
                    }
                }

                Ok(StateChange::None)
            }
            LineEditorAction::DragTimecodeCommit {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            } => {
                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(StateChange::None);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    // 批量调整：获取所有选中的事件项
                    let mut event_ids: HashSet<ItemId> = self
                        .ui_models
                        .line_editor_state
                        .get_selected_events(&self.state.project)
                        .into_iter()
                        .collect();

                    // 确保当前 event_id 也在列表中（同时也避免重复）
                    event_ids.insert(event_id);

                    // 批量调整
                    let mut updated_count = 0;
                    let total_count = event_ids.len();

                    for (index, &id) in event_ids.iter().enumerate() {
                        if let Ok(time) =
                            self.state
                                .project
                                .structure
                                .adjust_event_time(id, is_start_time, delta)
                        {
                            // 最后一个事件时更新分组头，其他事件只更新自身
                            let is_last = index == total_count - 1;

                            if is_last {
                                // 最后一个：全量递归更新分组头
                                self.ui_models
                                    .line_editor_state
                                    .update_row_time_full_bounds_recursive(
                                        &self.state.project,
                                        id,
                                        is_start_time,
                                        time,
                                    );
                            } else {
                                // 其他事件：只更新自身，不更新分组头
                                self.ui_models
                                    .line_editor_state
                                    .update_row_time_without_ancestor_groups(
                                        id,
                                        is_start_time,
                                        time,
                                    );
                            }
                            updated_count += 1;
                        }
                    }

                    // FIXME: 批量分支尚未走 StateChange/UIUpdater 管道，
                    // 此处直接修改了 UI 投影；后续应引入 `TimecodesBatchUpdated` payload。
                    if updated_count > 0 {
                        return Ok(StateChange::None);
                    }
                } else {
                    match self.state.project.structure.adjust_event_time(
                        event_id,
                        is_start_time,
                        delta,
                    ) {
                        Ok(time) => {
                            return Ok(StateChange::LineEditor(
                                LineEditorChange::TimecodeCommitDraged {
                                    event_id,
                                    is_start_time,
                                    time,
                                },
                            ));
                        }
                        Err(e) => {
                            log::warn!("{e}")
                        }
                    }
                }

                Ok(StateChange::None)
            }
            LineEditorAction::DragTimecodeCancel {
                event_id,
                is_start_time,
                is_batch,
            } => {
                if is_batch {
                    // 批量取消：获取所有选中的事件项
                    let mut event_ids: HashSet<ItemId> = self
                        .ui_models
                        .line_editor_state
                        .get_selected_events(&self.state.project);

                    // 确保当前 event_id 也在列表中（同时也避免重复）
                    event_ids.insert(event_id);

                    // 批量恢复到原始时间
                    let mut updated_count = 0;
                    for &id in &event_ids {
                        if let Ok(time) = self.state.project.structure.event_time(id, is_start_time)
                        {
                            self.ui_models
                                .line_editor_state
                                .update_row_time_without_ancestor_groups(id, is_start_time, time);
                            updated_count += 1;
                        }
                    }

                    // FIXME: 批量分支尚未走 StateChange/UIUpdater 管道，
                    // 此处直接修改了 UI 投影；后续应引入 `TimecodesBatchUpdated` payload。
                    if updated_count > 0 {
                        return Ok(StateChange::None);
                    }
                } else {
                    // 单个取消（恢复到 Domain 当前值）
                    if let Ok(time) = self
                        .state
                        .project
                        .structure
                        .event_time(event_id, is_start_time)
                    {
                        return Ok(StateChange::LineEditor(
                            LineEditorChange::TimecodeCancelDraged {
                                event_id,
                                is_start_time,
                                time,
                            },
                        ));
                    }
                }
                Ok(StateChange::None)
            }
            LineEditorAction::SelectSingle { item_id } => {
                Ok(StateChange::LineEditor(LineEditorChange::SelectSingle {
                    item_id,
                }))
            }
            LineEditorAction::SelectRange { item_id } => {
                Ok(StateChange::LineEditor(LineEditorChange::SelectRange {
                    item_id,
                }))
            }
            LineEditorAction::SelectToggle { item_id } => {
                Ok(StateChange::LineEditor(LineEditorChange::SelectToggle {
                    item_id,
                }))
            }
            LineEditorAction::AddLine {
                item_id,
                start_time,
                end_time,
                actor,
                text,
                segment_timing,
                // FIXME: 之后需要更改为节点链
                style,
            } => {
                let new_item_id = self.state.project.structure.insert_event_after(
                    item_id,
                    start_time,
                    end_time,
                    actor,
                    text,
                    segment_timing,
                    style,
                )?;

                Ok(StateChange::Multiple(vec![
                    StateChange::LineEditor(LineEditorChange::ItemsInserted {
                        item_id,
                        new_item_id,
                    }),
                    StateChange::LineEditor(LineEditorChange::SelectSingle {
                        item_id: new_item_id,
                    }),
                ]))
            }
            LineEditorAction::AddGroup { item_id } => {
                let group_count = self
                    .state
                    .project
                    .structure
                    .items
                    .values()
                    .filter(|item| matches!(item.content, ItemContent::Group(_)))
                    .count();

                let name = format!("新分组 {}", group_count + 1);

                let new_group_item_id = self
                    .state
                    .project
                    .structure
                    .insert_group_after(item_id, name)?;
                Ok(StateChange::Multiple(vec![
                    StateChange::LineEditor(LineEditorChange::ItemsInserted {
                        item_id,
                        new_item_id: new_group_item_id,
                    }),
                    StateChange::LineEditor(LineEditorChange::SelectSingle {
                        item_id: new_group_item_id,
                    }),
                ]))
            }
            LineEditorAction::DeleteItems {} => {
                let removed = self.delete_currently_selected();
                Ok(StateChange::LineEditor(LineEditorChange::ItemsRemoved {
                    item_ids: removed,
                }))
            }
            LineEditorAction::CopyItems => {
                let count = self.copy_currently_selected()?;
                Ok(StateChange::LineEditor(LineEditorChange::ItemsCopied {
                    count,
                }))
            }
            LineEditorAction::CutItems => {
                let count = self.copy_currently_selected()?;
                let removed = self.delete_currently_selected();
                Ok(StateChange::Multiple(vec![
                    StateChange::LineEditor(LineEditorChange::ItemsCopied { count }),
                    StateChange::LineEditor(LineEditorChange::ItemsRemoved { item_ids: removed }),
                ]))
            }
            LineEditorAction::UnGroup { group_item_id } => {
                let new_ids = self
                    .state
                    .project
                    .structure
                    .ungroup(group_item_id)
                    .map_err(|e| {
                        log::error!("解散分组失败 {:?}: {}", group_item_id, e);
                        e
                    })?;
                // UI 更新：移除旧分组行 + 插入提升的子项行
                Ok(StateChange::LineEditor(LineEditorChange::GroupsUngrouped {
                    group_id: group_item_id,
                    new_ids,
                }))
            }
            LineEditorAction::PasteItems { anchor } => {
                let Some(clip) = self.clipboard.paste_items()? else {
                    log::info!("剪贴板没有可粘贴的内容");
                    return Ok(StateChange::None);
                };
                let (new_root_ids, base_location) =
                    clip.instantiate(&mut self.state.project.structure, anchor)?;
                Ok(StateChange::LineEditor(LineEditorChange::ItemsPasted {
                    new_root_ids,
                    base_location,
                }))
            }
        }
    }

    /// 从当前选中集合构造 `ItemsClip` 并写入剪贴板，返回 root 数量。
    /// 选中为空时返回 0 且不写入剪贴板（保留旧内容）
    // NOTE: 也许输入为空的时候清空当前剪贴板更好？
    fn copy_currently_selected(&mut self) -> Result<usize> {
        let selected = self.ui_models.line_editor_state.directly_selected_items();
        if selected.is_empty() {
            return Ok(0);
        }
        let clip = ItemsClip::from_selection(&self.state.project.structure, selected);
        let count = clip.root_count();
        self.clipboard.copy_items(&clip)?;
        Ok(count)
    }

    /// 从 Domain 删除当前选中集合（过滤"祖先已选中"），返回实际被移除的 root id 列表。
    fn delete_currently_selected(&mut self) -> Vec<ItemId> {
        let selected_set = self
            .ui_models
            .line_editor_state
            .directly_selected_items()
            .clone();
        let to_delete: Vec<ItemId> = selected_set
            .iter()
            .copied()
            .filter(|id| {
                !&self
                    .state
                    .project
                    .structure
                    .has_ancestor_in_set(id, &selected_set)
            })
            .collect();

        let mut removed: Vec<ItemId> = Vec::with_capacity(to_delete.len());
        for id in to_delete {
            match self.state.project.structure.remove_item(id) {
                Ok(()) => removed.push(id),
                Err(e) => log::warn!("删除 structure 项失败 id={:?}: {}", id, e),
            }
        }
        removed
    }
}
