use crate::actions::Action;
use crate::store::state::AppState;
use crate::store::state_change::StateChange;
use anyhow::Result;

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

    fn dispatch_area(&mut self, action: crate::actions::area::AreaAction) -> Result<StateChange> {
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
}
