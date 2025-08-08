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

/// 拖拽操作的状态信息
#[derive(Debug, Clone)]
pub enum DragOperation {
    /// 无拖拽操作
    None,
    /// 分割操作 - 存储分割的源区域和预览数据
    Split {
        source_area: ScrAreaKey,
        preview_data: SplitPreviewOverlayData,
    },
    /// 合并操作 - 存储源区域、目标区域和邻接关系
    Join {
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
        adjacency: Adjacency,
    },
}

/// 应用状态和逻辑的唯一管理者
pub struct AppState {
    pub screen_map: ScrAreaMap,
    pub ui_settings: UISettings,
    pub window_size: PhysicalSize,
    pub initialized: bool,
    /// 当前拖拽操作状态
    pub current_drag_operation: DragOperation,
}
impl AppState {
    /// 构造函数，从 Slint 窗口初始化
    pub fn new(window: &slint::Window) -> Self {
        Self {
            screen_map: ScrAreaMap::new(),
            ui_settings: UISettings::from_slint_window(window),
            window_size: window.size(),
            initialized: false,
            current_drag_operation: DragOperation::None,
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

            let key_ffi = area_key.data().as_ffi();
            let area_key_index = (key_ffi & 0xFFFFFFFF) as i32;      // 低32位：索引
            let area_key_version = (key_ffi >> 32) as i32;           // 高32位：版本
            
            areas_data.push(ScrAreaData {
                area_key_index,
                area_key_version,
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
            move |area_key_index, area_key_version, corner, offset_x, offset_y| {
                let mut state = state_for_drag.borrow_mut();
                let scale_factor = state.ui_settings.get_scale_factor();
                let actionzone_side_length = state.ui_settings.get_actionzone_side_length();

                let physical_offset_x = offset_x as f64 * scale_factor;
                let physical_offset_y = offset_y as f64 * scale_factor;

                // 重新组合完整的键值（版本 + 索引）
                let area_key_ffi = ((area_key_version as u64) << 32) | (area_key_index as u64);
                let source_key = ScrAreaKey::from(KeyData::from_ffi(area_key_ffi));
                
                // 防护检查：确保源区域仍然存在
                if !state.screen_map.areas.contains_key(source_key) {
                    println!("警告：源区域 {:?} 已不存在，忽略拖拽事件", source_key);
                    return;
                }
                
                if let Some(instance) = weak_instance_for_drag.upgrade() {
                    if let Some(target_area_key) = state.screen_map.find_area_with_actionzone(
                        source_key,
                        corner,
                        physical_offset_x,
                        physical_offset_y,
                        actionzone_side_length,
                    ) {
                        if target_area_key == source_key {
                            // println!("在相同区域内 - 执行分割操作");
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
                            
                            // 更新拖拽状态
                            state.current_drag_operation = DragOperation::Split {
                                source_area: source_key,
                                preview_data: split_preview_overlay_data.clone(),
                            };
                            
                            // 显示分割预览，隐藏合并预览
                            instance.set_show_join_highlight_overlay(false);
                            instance.set_show_close_area_overlays(false);
                            instance.set_show_split_preview_overlay(true);
                            // println!("对于区域左上角偏移({relative_offset_x},{relative_offset_y})");
                            // println!("生成的分割数据: {:#?}", split_preview_overlay_data);
                            instance.set_split_preview_overlay_data(split_preview_overlay_data);
                        } else {
                            // println!("移动到不同区域 {:?} - 执行合并操作", target_area_key);
                            // 计算邻接关系
                            if let Some(adjacency) = state
                                .screen_map
                                .detect_adjacency_by_keys(source_key, target_area_key)
                            {
                                // 更新拖拽状态
                                state.current_drag_operation = DragOperation::Join {
                                    source_area_key: source_key,
                                    target_area_key: target_area_key,
                                    adjacency,
                                };
                                
                                // 生成合并高亮与即将关闭区域预览
                                match state.screen_map.preview_join_adjacent_areas(
                                    source_key,
                                    adjacency,
                                    &state.ui_settings,
                                ) {
                                    Ok((join_data, close_datas)) => {
                                        // 显示合并预览，隐藏分割预览
                                        instance.set_show_split_preview_overlay(false);
                                        instance.set_join_highlight_overlay_data(join_data);
                                        instance.set_show_join_highlight_overlay(true);
                                        instance.set_close_area_overlay_datas(
                                            close_datas.as_slice().into(),
                                        );
                                        instance
                                            .set_show_close_area_overlays(!close_datas.is_empty());
                                    }
                                    Err(e) => {
                                        eprintln!("生成合并预览失败: {}", e);
                                        instance.set_show_join_highlight_overlay(false);
                                        instance.set_show_close_area_overlays(false);
                                    }
                                }
                            } else {
                                // 非相邻：清除拖拽状态，隐藏合并预览
                                state.current_drag_operation = DragOperation::None;
                                instance.set_show_join_highlight_overlay(false);
                                instance.set_show_close_area_overlays(false);
                            }
                        }
                    } else {
                        // println!("移动到区域外 - 可能执行窗口操作");
                        // 离开任何区域，清除拖拽状态，隐藏所有预览
                        state.current_drag_operation = DragOperation::None;
                        instance.set_show_split_preview_overlay(false);
                        instance.set_show_join_highlight_overlay(false);
                        instance.set_show_close_area_overlays(false);
                    }
                }
            },
        );

        let weak_instance_for_drag_end = weak_instance.clone();
        let state_for_drag_end = Rc::clone(&self.state);
        self.instance
            .global::<ActionManager>()
            .on_actionzone_drag_end(move |area_key_index, area_key_version, is_cancel| {
                println!("鼠标拖动结束, 是否是右键取消: {is_cancel}");

                if !is_cancel {
                    if let Some(instance) = weak_instance_for_drag_end.upgrade() {
                        let mut state = state_for_drag_end.borrow_mut();
                        
                        // 先提取拖拽状态，避免借用冲突
                        let drag_operation = state.current_drag_operation.clone();
                        
                        // 根据存储的拖拽状态执行相应操作
                        match drag_operation {
                            DragOperation::Split { source_area, preview_data } => {
                                println!("执行拆分操作，源区域: {:?}, 预览数据: {:#?}", source_area, preview_data);
                                
                                // 验证分割数据是否有效（避免0尺寸的区域）
                                if preview_data.width2 <= 0.0 || preview_data.height2 <= 0.0 {
                                    println!("分割数据无效（第二个区域尺寸为0），取消分割操作");
                                } else {
                                    let ui_settings = &state.ui_settings.clone();
                                    if let Err(e) = state.screen_map.split_area(
                                        source_area,
                                        preview_data,
                                        ui_settings,
                                    ) {
                                        eprintln!("执行区域拆分失败: {}", e);
                                    } else {
                                        // 更新UI显示
                                        let slint_data = state.convert_areas_to_slint_data();
                                        instance.set_scrareas(slint_data.as_slice().into());
                                        println!("拆分完成，更新后的区域数据: {:#?}", slint_data);
                                    }
                                }
                            }
                            DragOperation::Join { source_area_key, target_area_key, adjacency } => {
                                println!("执行合并操作，源区域: {:?} -> 目标区域: {:?}", source_area_key, target_area_key);
                                
                                // 立即清除拖拽状态，避免后续事件使用过期的区域键
                                state.current_drag_operation = DragOperation::None;
                                
                                let ui_settings = &state.ui_settings.clone();
                                if let Err(e) = state.screen_map.apply_join_adjacent_areas(
                                    source_area_key,
                                    target_area_key,
                                    adjacency,
                                    ui_settings
                                ) {
                                    eprintln!("执行区域合并失败: {}", e);
                                } else {
                                    // 重新计算所有区域的几何数据
                                    state.screen_map.update_all_areas_rcti(ui_settings);
                                    
                                    // 更新UI显示
                                    let slint_data = state.convert_areas_to_slint_data();
                                    instance.set_scrareas(slint_data.as_slice().into());
                                    
                                    println!("合并完成，更新后的区域数据: {:#?}", slint_data);
                                }
                            }
                            DragOperation::None => {
                                // println!("无有效的拖拽操作，取消操作");
                            }
                        }

                        // 清除拖拽状态和预览（如果还没清除的话）
                        state.current_drag_operation = DragOperation::None;
                        instance.set_show_split_preview_overlay(false);
                        instance.set_show_join_highlight_overlay(false);
                        instance.set_show_close_area_overlays(false);
                    }
                } else {
                    // println!("用户取消拖拽操作");
                    // 清除拖拽状态和预览
                    if let Some(instance) = weak_instance_for_drag_end.upgrade() {
                        let mut state = state_for_drag_end.borrow_mut();
                        state.current_drag_operation = DragOperation::None;
                        instance.set_show_split_preview_overlay(false);
                        instance.set_show_join_highlight_overlay(false);
                        instance.set_show_close_area_overlays(false);
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
