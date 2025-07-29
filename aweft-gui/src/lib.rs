slint::include_modules!();

mod types;
mod app_state;
mod screen;

use anyhow::{Context, Result};
use app_state::{APP_STATE, get_app_state, get_app_state_mut};
use screen::geometry::*;
use types::*;
use slotmap::{Key, KeyData};

/// 简化的应用窗口管理器
pub struct AppWindowManager {
    ui: MainWindow,
}

impl AppWindowManager {
    /// 创建新的应用窗口管理器
    pub fn new() -> Result<Self> {
        let ui = MainWindow::new().context("创建 Slint 主窗口失败")?;
        Ok(Self { ui })
    }

    /// 获取 MainWindow 的引用
    pub fn get_window(&self) -> &MainWindow {
        &self.ui
    }

    /// 运行应用
    pub fn run(&mut self) -> Result<()> {
        // 安排在窗口显示后初始化屏幕布局（异步执行）
        let window_weak = self.ui.as_weak();

        if let Err(e) = slint::invoke_from_event_loop(move || {
            if let Some(app) = window_weak.upgrade() {
                if let Err(e) = Self::initialize_layout(&app) {
                    eprintln!("初始化布局失败: {}", e);
                }
            }
        }) {
            eprintln!("安排初始化任务失败: {}", e);
        }

        // 直接启动 UI 事件循环
        self.ui.run().context("启动 UI 事件循环失败")
    }

    /// 初始化屏幕布局
    fn initialize_layout(app: &MainWindow) -> Result<()> {
        let physical_window_size = app.window().size(); // 物理像素大小
        let scale_factor = app.window().scale_factor();

        println!(
            "窗口已初始化，物理尺寸: {}x{}, 缩放因子: {:.2}",
            physical_window_size.width, physical_window_size.height, scale_factor
        );

        // 初始化应用状态和提取必要数据
        let (area_keys, ui_scale_fac, actionzone_side_length) = {
            let mut state = get_app_state_mut()?;
            
            if !state.initialized {
                state.ui_settings = UISettings::from_slint_window(&app.window());

                // 克隆UI设置以避免借用冲突
                let ui_settings = state.ui_settings.clone();
                let area_keys = state.screen_map.create_three_area_layout(
                    physical_window_size.width,  // 使用物理像素进行内部计算
                    physical_window_size.height, // 使用物理像素进行内部计算
                    &ui_settings,
                );

                state.initialized = true;
                (
                    Some(area_keys), 
                    state.ui_settings.ui_scale_fac(),
                    state.ui_settings.actionzone_side_length()
                )
            } else {
                (
                    None, 
                    state.ui_settings.ui_scale_fac(),
                    state.ui_settings.actionzone_side_length()
                )
            }
        }; // 作用域结束，自动释放锁

        // 打印区域信息并传递给Slint UI
        if let Some(area_keys) = area_keys {
            let slint_area_data = {
                let state = get_app_state()?;
                let mut areas_data = Vec::new();
                
                for area_key in &area_keys {
                    if let Some(area) = state.screen_map.areas.get(*area_key) {
                        let (geometry_rect, winx, winy) = area.geometry();
                        println!("区域 {:?}: {}x{} (物理像素), 坐标({},{})", area_key, winx, winy, geometry_rect.xmin, geometry_rect.ymin);

                        // 转换为逻辑像素（Slint期望的坐标系统）
                        let scale_factor = state.ui_settings.ui_scale_fac();
                        let logical_x = geometry_rect.xmin as f32 / scale_factor;
                        let logical_y = geometry_rect.ymin as f32 / scale_factor;
                        let logical_width = winx as f32 / scale_factor;
                        let logical_height = winy as f32 / scale_factor;

                        areas_data.push(ScrAreaData {
                            key: area_key.data().as_ffi() as i32,
                            x: logical_x,
                            y: logical_y,
                            width: logical_width,
                            height: logical_height,
                            actionzone_side_length: state.ui_settings.actionzone_side_length(),
                        });
                    } else {
                        println!("区域 {:?} 不存在", area_key);
                    }
                }
                areas_data
            };

            // 使用Slint自动生成的setter方法传递数据
            app.set_scrareas(slint_area_data.as_slice().into());
            
            // 设置高性能拖拽回调 - 预先提取所有必要数据，完全避免在回调中访问全局状态
            app.on_actionzone_drag({
                move |area_key, corner, offset_x, offset_y| {
                    // 直接使用预先提取的数据，无需访问全局状态
                    let physical_offset_x = offset_x * ui_scale_fac;
                    let physical_offset_y = offset_y * ui_scale_fac;
                    
                    if let Err(e) = Self::handle_actionzone_drag_optimized(
                        area_key as u64, 
                        corner, 
                        physical_offset_x, 
                        physical_offset_y,
                        actionzone_side_length
                    ) {
                        eprintln!("处理actionzone拖拽失败: {}", e);
                    }
                }
            });

            println!("传递给Slint UI（逻辑像素）: {:#?}", slint_area_data);
        }

        Ok(())
    }

    /// 优化的 ActionZone 拖拽处理（完全避免全局状态访问）
    fn handle_actionzone_drag_optimized(
        area_key: u64, 
        corner: ActionZoneCorner, 
        physical_offset_x: f32, 
        physical_offset_y: f32,
        actionzone_side_length: f32
    ) -> Result<()> {
        // 快速获取状态，查找区域后立即释放锁
        let current_area = {
            let state = get_app_state()?;
            state.screen_map.find_area_with_actionzone(
                ScrAreaKey::from(KeyData::from_ffi(area_key)), 
                corner, 
                physical_offset_x, 
                physical_offset_y,
                actionzone_side_length
            )
        }; // 锁在这里被释放
        
        println!(
            "回调数据: area_key: {area_key}, corner: {:?}, x（物理像素）: {}, y（物理像素）: {}, 当前区域: {:?}",
            corner,
            physical_offset_x,
            physical_offset_y,
            current_area
        );
        
        // 根据当前区域决定操作类型（无需再次获取锁）
        let source_area_key = ScrAreaKey::from(KeyData::from_ffi(area_key));
        match current_area {
            Some(current) if current == source_area_key => {
                println!("在相同区域内 - 执行分割操作");
            },
            Some(other) => {
                println!("移动到不同区域 {:?} - 执行合并操作", other);
            },
            None => {
                println!("移动到区域外 - 可能执行窗口操作");
            }
        }

        Ok(())
    }

    /// 处理窗口大小或DPI变化
    pub fn handle_window_change(&mut self) -> Result<()> {
        let mut state = get_app_state_mut()?;

        let physical_window_size = self.ui.window().size(); // 物理像素大小
        let scale_factor = self.ui.window().scale_factor();

        // 更新UI设置
        state.ui_settings = UISettings::from_slint_window(&self.ui.window());

        // 计算新的窗口矩形（使用物理像素）
        let window_rect = Rcti {
            xmin: 0,
            xmax: physical_window_size.width - 1,
            ymin: 0,
            ymax: physical_window_size.height - 1,
        };

        // 克隆UI设置以避免借用冲突
        let ui_settings = state.ui_settings.clone();

        // 更新所有区域的大小和位置
        state.screen_map.update_all_areas_with_scaling(
            &window_rect,
            &ui_settings,
            false, // 是否全屏
            false, // 是否临时窗口
        );

        println!("窗口变化后的区域信息:");
        println!(
            "  物理窗口大小: {}x{}",
            physical_window_size.width, physical_window_size.height
        );
        println!(
            "  逻辑窗口大小: {}x{}",
            (physical_window_size.width as f32 / scale_factor) as u32,
            (physical_window_size.height as f32 / scale_factor) as u32
        );
        println!("  缩放因子: {:.2}", scale_factor);

        for (area_key, area) in &state.screen_map.areas {
            let (_, winx, winy) = area.geometry();
            println!("  区域 {:?}: {}x{} (物理像素)", area_key, winx, winy);
        }

        Ok(())
    }

    /// 检查是否已初始化
    pub fn is_initialized() -> bool {
        APP_STATE.read().map_or(false, |state| state.initialized)
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
