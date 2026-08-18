use crate::depsgraph::Tag;
use crate::store::state::AppState;
use anyhow::Result;

/// 处理窗口大小改变
pub fn handle_resize(state: &mut AppState, width: u32, height: u32) -> Result<Vec<Tag>> {
    use crate::screen::primitives::Rcti;
    use slint::PhysicalSize;

    let old_size = (state.window_size.width, state.window_size.height);

    let old_window_rect = Rcti {
        xmin: 0,
        xmax: old_size.0.saturating_sub(1),
        ymin: 0,
        ymax: old_size.1.saturating_sub(1),
    };

    let new_window_rect = Rcti {
        xmin: 0,
        xmax: width.saturating_sub(1),
        ymin: 0,
        ymax: height.saturating_sub(1),
    };

    // 缩放所有顶点
    state.screen_map.scale_vertices_to_window(
        &old_window_rect,
        &new_window_rect,
        &state.theme_settings,
    );

    // 更新状态中的窗口大小
    state.window_size = PhysicalSize { width, height };

    Ok(vec![Tag::ScreenResized {
        old_size,
        new_size: (width, height),
    }])
}

/// 处理缩放因子改变
pub fn handle_scale_change(state: &mut AppState, scale_factor: f64) -> Result<Vec<Tag>> {
    let old_scale = state.theme_settings.get_scale_factor();

    state.theme_settings.update_scale_factor(scale_factor);

    // 更新所有区域的几何属性
    state
        .screen_map
        .update_all_areas_rcti(&state.theme_settings);

    Ok(vec![Tag::ScaleChanged {
        old_scale,
        new_scale: scale_factor,
    }])
}

/// 处理布局初始化
pub fn handle_initialize(state: &mut AppState, width: u32, height: u32) -> Result<Vec<Tag>> {
    if state.initialized {
        return Ok(vec![]);
    }

    // 布局 size 即窗口实际尺寸：同步 window_size，供后续 resize 用正确的 old_rect
    // （否则第一次 resize 会用过时的 AppState::new 窗口尺寸作缩放基准）。
    state.window_size = slint::PhysicalSize { width, height };

    state
        .screen_map
        .create_three_area_layout(&width, &height, &state.theme_settings);

    // 标记窗口边框边缘
    let current_rect = state.current_window_rect();
    state
        .screen_map
        .mark_window_border_edges(&current_rect, &state.theme_settings);

    state.initialized = true;

    Ok(vec![Tag::ScreenInitialized])
}

impl crate::store::AppStore {
    pub(crate) fn dispatch_screen(
        &mut self,
        action: crate::actions::screen::ScreenAction,
    ) -> Result<Vec<Tag>> {
        use crate::actions::screen::ScreenAction;

        match action {
            ScreenAction::WindowResized { width, height } => {
                handle_resize(&mut self.state, width, height)
            }
            ScreenAction::ScaleFactorChanged { scale_factor } => {
                handle_scale_change(&mut self.state, scale_factor)
            }
            ScreenAction::InitializeLayout { width, height } => {
                handle_initialize(&mut self.state, width, height)
            }
        }
    }
}
