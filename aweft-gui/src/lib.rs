slint::include_modules!();

mod screen;
pub mod types;

use std::{cell::RefCell, rc::Rc};

use anyhow::{Context, Result};
use screen::geometry::*;
use slint::{
    PhysicalSize, RenderingState,
    winit_030::{WinitWindowAccessor, WinitWindowEventResult, winit},
};
use slotmap::{Key, KeyData};
use types::*;

/// 应用状态和逻辑的唯一管理者
pub struct AppState {
    pub screen_map: ScrAreaMap,
    pub ui_settings: UISettings,
    pub window_size: PhysicalSize,
    pub initialized: bool,
}
impl AppState {
    /// 构造函数，从 Slint 窗口初始化
    pub fn new(window: &slint::Window) -> Self {
        Self {
            screen_map: ScrAreaMap::new(),
            ui_settings: UISettings::from_slint_window(window),
            window_size: window.size(),
            initialized: false,
        }
    }

    /// 将内部区域数据转换为 Slint 需要的格式
    /// 这个函数现在是 AppState 的一部分，可以直接访问其字段，无需加锁
    fn convert_areas_to_slint_data(&self) -> Vec<ScrAreaData> {
        let mut areas_data = Vec::new();
        let scale_factor = self.ui_settings.get_scale_factor();
        let actionzone_side_length = self.ui_settings.get_actionzone_side_length() as f32;

        for (area_key, area) in &self.screen_map.areas {
            let (geometry_rect, winx, winy) = area.geometry();

            // 物理像素 -> 逻辑像素转换
            let logical_x = geometry_rect.xmin as f64 / scale_factor;
            let logical_y = geometry_rect.ymin as f64 / scale_factor;
            let logical_width = winx as f64 / scale_factor;
            let logical_height = winy as f64 / scale_factor;

            areas_data.push(ScrAreaData {
                key: area_key.data().as_ffi() as i32,
                x: logical_x as f32,
                y: logical_y as f32,
                width: logical_width as f32,
                height: logical_height as f32,
                actionzone_side_length,
            });
        }
        areas_data
    }
}

/// 简化的应用窗口管理器
pub struct AppWindowManager {
    instance: MainWindow,
    state: Rc<RefCell<AppState>>,
}

impl AppWindowManager {
    /// 创建新的应用窗口管理器
    pub fn new() -> Result<Self> {
        use slint::BackendSelector;
        // 强制选择winit后端
        BackendSelector::new()
            .backend_name("winit".into())
            // 使用 WGPU 渲染
            .require_wgpu_24(slint::wgpu_24::WGPUConfiguration::default())
            .select()?;
        let instance = MainWindow::new().context("创建 Slint 主窗口失败")?;
        let state = Rc::new(RefCell::new(AppState::new(&instance.window())));
        Ok(Self { instance, state })
    }

    /// 获取 MainWindow 的引用
    pub fn get_instance(&self) -> &MainWindow {
        &self.instance
    }

    /// 运行应用
    pub fn run(&mut self) -> Result<()> {
        self.setup_callbacks();

        // 直接启动 UI 事件循环
        self.instance.run().context("启动 UI 事件循环失败")
    }

    fn setup_callbacks(&self) {
        let weak_instance = self.instance.as_weak();
        let window = self.instance.window();

        let weak_instance_for_init = weak_instance.clone();
        let state_for_init = Rc::clone(&self.state);
        if let Err(e) = window.set_rendering_notifier(move |rendering_state, graphics_api| {
            if let Some(instance) = weak_instance_for_init.upgrade() {
                match rendering_state {
                    RenderingState::RenderingSetup => {
                        println!(
                            "窗口初始化完成: {:?}, {:?}",
                            instance.window().size(),
                            graphics_api
                        );
                        if let Err(e) =
                            AppWindowManager::initialize_layout(&state_for_init, &instance)
                        {
                            panic!("布局初始化失败: {e}")
                        }
                    }
                    _ => (),
                }
            }
        }) {
            eprintln!("注册渲染初始化失败: {:?}", e);
        };

        let weak_instance_for_drag = weak_instance.clone();
        let state_for_drag = Rc::clone(&self.state);
        self.instance.global::<ActionManager>().on_actionzone_drag(
            move |area_key, corner, offset_x, offset_y| {
                let state = state_for_drag.borrow();
                let scale_factor = state.ui_settings.get_scale_factor();
                let actionzone_side_length = state.ui_settings.get_actionzone_side_length();

                let physical_offset_x = offset_x as f64 * scale_factor;
                let physical_offset_y = offset_y as f64 * scale_factor;

                let source_key = ScrAreaKey::from(KeyData::from_ffi(area_key as u64));
                if let Some(instance) = weak_instance_for_drag.upgrade() {
                    if let Some(target_area_key) = state.screen_map.find_area_with_actionzone(
                        source_key,
                        corner,
                        physical_offset_x,
                        physical_offset_y,
                        actionzone_side_length,
                    ) {
                        if target_area_key == source_key {
                            println!("在相同区域内 - 执行分割操作");
                            let (relative_offset_x, relative_offset_y) =
                                state.screen_map.convert_corner_offset_to_relative_offset(
                                    target_area_key,
                                    corner,
                                    physical_offset_x,
                                    physical_offset_y,
                                    actionzone_side_length,
                                );
                            let split_preview_overlay_data =
                                state.screen_map.calculate_split_areas(
                                    target_area_key,
                                    physical_offset_x,
                                    physical_offset_y,
                                    relative_offset_x,
                                    relative_offset_y,
                                );
                            instance.set_show_split_preview_overlay(true);
                            println!("对于区域左上角偏移({relative_offset_x},{relative_offset_y})");
                            println!("生成的分割数据: {:#?}", split_preview_overlay_data);
                            instance.set_split_preview_overlay_data(split_preview_overlay_data);
                        } else {
                            println!("移动到不同区域 {:?} - 执行合并操作", target_area_key);
                            // TODO: 调用合并逻辑
                        }
                    } else {
                        println!("移动到区域外 - 可能执行窗口操作");
                    }
                }
            },
        );

        let weak_instance_for_drag_end = weak_instance.clone();
        let state_for_drag_end = Rc::clone(&self.state);
        self.instance
            .global::<ActionManager>()
            .on_actionzone_drag_end(move |area_key, is_cancel| {
                println!("鼠标拖动结束, 是否是右键取消: {is_cancel}");

                if !is_cancel {
                    // 获取当前状态
                    let mut state = state_for_drag_end.borrow_mut();

                    // 获取拆分预览数据
                    if let Some(instance) = weak_instance_for_drag_end.upgrade() {
                        let split_data = instance.get_split_preview_overlay_data();

                        // 检查是否有有效的拆分数据（第二个区域不为0表示有效拆分）
                        if split_data.width2 > 0.0 && split_data.height2 > 0.0 {
                            println!("执行拆分操作，预览数据: {:#?}", split_data);
                            let ui_settings = &state.ui_settings.clone();
                            // 执行实际的拆分操作
                            if let Err(e) = state.screen_map.split_area(
                                ScrAreaKey::from(KeyData::from_ffi(area_key as u64)),
                                split_data,
                                ui_settings,
                            ) {
                                eprintln!("执行区域拆分失败: {}", e);
                            } else {
                                // 更新UI显示
                                let slint_data = state.convert_areas_to_slint_data();
                                instance.set_scrareas(slint_data.as_slice().into());
                                println!("拆分完成，更新后的区域数据: {:#?}", slint_data);
                            }
                        } else {
                            println!("无效的拆分数据，取消拆分操作");
                        }

                        // 清除预览
                        instance.set_show_split_preview_overlay(false);
                    }
                } else {
                    println!("用户取消拆分操作");
                    // 清除预览
                    if let Some(instance) = weak_instance_for_drag_end.upgrade() {
                        instance.set_show_split_preview_overlay(false);
                    }
                }
            });
        // Winit 事件处理
        let state_for_event = Rc::clone(&self.state);
        self.instance
            .window()
            .on_winit_window_event(move |_slint_window, event| {
                match event {
                    winit::event::WindowEvent::Resized(new_size) => {
                        println!("窗口大小发生改变: {}x{}", new_size.width, new_size.height);
                        let mut state = state_for_event.borrow_mut();

                        // 调用新的、清晰的逻辑
                        if let Err(e) = Self::handle_window_resize(&mut state, *new_size) {
                            eprintln!("处理窗口大小变化失败: {}", e);
                        }

                        // 更新 UI
                        let slint_data = state.convert_areas_to_slint_data();
                        if let Some(instance) = weak_instance.upgrade() {
                            instance.set_scrareas(slint_data.as_slice().into());
                        }
                    }
                    winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        println!("检测到屏幕缩放因子变化: {:.2}", scale_factor);
                        let mut state = state_for_event.borrow_mut();

                        if let Err(e) = Self::handle_scale_change(&mut state, *scale_factor) {
                            eprintln!("处理缩放因子变化失败: {}", e);
                        }

                        // 更新 UI
                        let slint_data = state.convert_areas_to_slint_data();
                        println!("更新后的传入数据: {:#?}", slint_data);
                        if let Some(instance) = weak_instance.upgrade() {
                            instance.set_scrareas(slint_data.as_slice().into());
                        }
                    }
                    _ => {}
                }
                WinitWindowEventResult::Propagate
            });
    }
    fn update_slint_areas_data(state: &AppState, instance: &MainWindow) -> Result<()> {
        let slint_area_data = state.convert_areas_to_slint_data();

        println!("正在更新 Slint UI 数据: {:#?}", slint_area_data);
        instance.set_scrareas(slint_area_data.as_slice().into());

        Ok(())
    }

    fn initialize_layout(state_rc: &Rc<RefCell<AppState>>, instance: &MainWindow) -> Result<()> {
        let mut state = state_rc.borrow_mut();
        let (width, height, ui_settings) = {
            if state.initialized {
                return Ok(());
            }

            println!(
                "窗口已初始化，物理尺寸: {}x{}, 缩放因子: {:.2}",
                state.window_size.width,
                state.window_size.height,
                state.ui_settings.get_scale_factor()
            );
            (
                state.window_size.width,
                state.window_size.height,
                state.ui_settings.clone(),
            )
        };

        state
            .screen_map
            .create_three_area_layout(&width, &height, &ui_settings);
        state.initialized = true;

        Self::update_slint_areas_data(&state, instance)?;
        Ok(())
    }

    fn handle_window_resize(
        state: &mut AppState,
        new_size: winit::dpi::PhysicalSize<u32>,
    ) -> Result<()> {
        let old_size = state.window_size;

        let old_window_rect = Rcti {
            xmin: 0,
            xmax: old_size.width.saturating_sub(1),
            ymin: 0,
            ymax: old_size.height.saturating_sub(1),
        };

        let new_window_rect = Rcti {
            xmin: 0,
            xmax: new_size.width.saturating_sub(1),
            ymin: 0,
            ymax: new_size.height.saturating_sub(1),
        };

        // 关键：缩放所有顶点
        state.screen_map.scale_vertices_to_window(
            &old_window_rect,
            &new_window_rect,
            &state.ui_settings,
        );

        // 更新状态中的窗口大小
        state.window_size = PhysicalSize {
            width: new_size.width,
            height: new_size.height,
        };
        // TODO: 由于是直接缩放，如果持续有误差可能会导致窗口最边角错位
        // 之后实现一下窗口边缘resize之后吸附靠近的顶点到最边缘
        Ok(())
    }
    fn handle_scale_change(state: &mut AppState, new_scale_factor: f64) -> Result<()> {
        state.ui_settings.update_scale_factor(new_scale_factor);

        // 更新所有区域的几何属性，以应用新的边距等
        state.screen_map.update_all_areas_rcti(
            &state.ui_settings, // 直接传递引用
        );
        Ok(())
    }
}

// 导出一个公共函数，用于创建和运行窗口
pub fn create_window() -> Result<()> {
    let mut app = AppWindowManager::new().context("创建应用窗口管理器失败（是否nix develop?）")?;
    app.run().context("运行应用失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_area_size_calculation() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        // 创建一个1920x1080的屏幕（物理像素）
        let area_key = screen_map.create_initial_screen_with_scaling(1920, 1080, &ui_settings);

        let area = screen_map.areas.get(area_key).unwrap();

        // 使用geometry()方法获取几何信息 - 返回的是物理像素坐标和大小
        let (_, winx, winy) = area.geometry();

        // 验证计算结果
        assert!(winx > 0);
        assert!(winy > 0);
        assert!(winx <= 1920);
        assert!(winy <= 1080);

        println!(
            "测试区域大小: {}x{} (物理像素，与Blender保持一致)",
            winx, winy
        );
    }
}
