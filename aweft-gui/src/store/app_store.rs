use crate::actions::Action;
use crate::actions::area::AreaAction;
use crate::actions::line_editor::LineEditorAction;
use crate::store::state::AppState;
use crate::store::state_change::{LineEditorChange, StateChange};
use anyhow::Result;
use aweft_model::structure::ItemContent;
use chrono::TimeDelta;

/// 应用状态存储 - 单一数据源
pub struct AppStore {
    state: AppState,
}

impl AppStore {
    /// 创建新的 Store（从 MainWindow 以访问主题系统）
    pub fn new(main_window: &crate::bridge::types::MainWindow) -> Self {
        use slint::ComponentHandle;

        let theme_settings = crate::bridge::types::ThemeSettings::from_main_window(main_window);
        Self {
            state: AppState::new(&main_window.window(), theme_settings),
        }
    }

    /// 分发 Action，返回状态变更描述
    pub fn dispatch(&mut self, action: Action) -> Result<StateChange> {
        // 可以在这里添加日志
        // println!("📤 Dispatching action: {:?}", action);

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

        // 可以在这里添加日志
        // println!("📥 State change: {:?}", change);

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
    fn dispatch_line_editor(&mut self, action: LineEditorAction) -> Result<StateChange> {
        match action {
            LineEditorAction::ToggleGroupCollapsed { group_id } => {
                // Rust 数据层修改 (持久化状态)
                if let Some(item) = self.state.project.structure.get_mut(group_id) {
                    if let ItemContent::Group(g) = &mut item.content {
                        g.collapsed = !g.collapsed;
                    }
                }

                // FIXME: 此部分虽然还是Rust侧持有的数据，但是是修改UI部分的
                // 可能需要改动Updater处理的细粒度

                // ViewModel 层修改 (驱动 UI 变化)
                self.state
                    .line_editor_state
                    .toggle_group(group_id.to_string());

                Ok(StateChange::LineEditor(
                    LineEditorChange::GroupCollapsedToggled,
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
                    // FIXME: 此部分虽然还是Rust侧持有的数据，但是是修改UI部分的
                    // 可能需要改动Updater处理的细粒度
                    self.state.line_editor_state.update_row_time(
                        &self.state.project,
                        event_id,
                        is_start_time,
                        time,
                    );
                    return Ok(StateChange::LineEditor(LineEditorChange::TimecodeInputed));
                }
                Ok(StateChange::None)
            }
            LineEditorAction::IncrementTimecode {
                event_id,
                is_start_time,
            } => {
                if let Ok(time) = self.state.project.structure.adjust_event_time(
                    event_id,
                    is_start_time,
                    TimeDelta::seconds(1),
                ) {
                    // FIXME: 此部分虽然还是Rust侧持有的数据，但是是修改UI部分的
                    // 可能需要改动Updater处理的细粒度
                    self.state.line_editor_state.update_row_time(
                        &self.state.project,
                        event_id,
                        is_start_time,
                        time,
                    );
                    return Ok(StateChange::LineEditor(
                        LineEditorChange::TimecodeIncremented,
                    ));
                }
                Ok(StateChange::None)
            }
            LineEditorAction::DecrementTimecode {
                event_id,
                is_start_time,
            } => {
                if let Ok(time) = self.state.project.structure.adjust_event_time(
                    event_id,
                    is_start_time,
                    TimeDelta::seconds(-1),
                ) {
                    // FIXME: 此部分虽然还是Rust侧持有的数据，但是是修改UI部分的
                    // 可能需要改动Updater处理的细粒度
                    self.state.line_editor_state.update_row_time(
                        &self.state.project,
                        event_id,
                        is_start_time,
                        time,
                    );
                    return Ok(StateChange::LineEditor(
                        LineEditorChange::TimecodeDecremented,
                    ));
                }
                Ok(StateChange::None)
            }
            LineEditorAction::DragTimecodePreview {
                event_id,
                is_start_time,
                delta_x,
            } => {
                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(StateChange::None);
                }
                if let Ok(time) = self.state.project.structure.preview_adjust_event_time(
                    event_id,
                    is_start_time,
                    TimeDelta::milliseconds((delta_x.trunc() as i64) * 10),
                ) {
                    // FIXME: 此部分虽然还是Rust侧持有的数据，但是是修改UI部分的
                    // 可能需要改动Updater处理的细粒度
                    self.state
                        .line_editor_state
                        .update_row_time_without_ancestor_groups(event_id, is_start_time, time);
                    return Ok(StateChange::LineEditor(
                        LineEditorChange::TimecodePreviewDraged,
                    ));
                }
                Ok(StateChange::None)
            }
            LineEditorAction::DragTimecodeCommit {
                event_id,
                is_start_time,
                delta_x,
            } => {
                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(StateChange::None);
                }
                if let Ok(time) = self.state.project.structure.adjust_event_time(
                    event_id,
                    is_start_time,
                    TimeDelta::milliseconds((delta_x.trunc() as i64) * 10),
                ) {
                    // FIXME: 此部分虽然还是Rust侧持有的数据，但是是修改UI部分的
                    // 可能需要改动Updater处理的细粒度
                    self.state.line_editor_state.update_row_time(
                        &self.state.project,
                        event_id,
                        is_start_time,
                        time,
                    );
                    return Ok(StateChange::LineEditor(
                        LineEditorChange::TimecodePreviewDraged,
                    ));
                }
                Ok(StateChange::None)
            }
            LineEditorAction::DragTimecodeCancel {
                event_id,
                is_start_time,
            } => {
                if let Ok(time) = self
                    .state
                    .project
                    .structure
                    .event_time(event_id, is_start_time)
                {
                    self.state.line_editor_state.update_row_time(
                        &self.state.project,
                        event_id,
                        is_start_time,
                        time,
                    );
                    return Ok(StateChange::LineEditor(
                        LineEditorChange::TimecodeCancelDraged,
                    ));
                }
                Ok(StateChange::None)
            }
        }
    }
}
