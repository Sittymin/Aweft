slint::include_modules!();

pub mod screen;

use anyhow::{Context, Result};
use screen::geometry;
use screen::types::{ScrAreaKey, ScrAreaMap, UIScaleSettings};
use slotmap::Key;
use std::sync::{Arc, Mutex};

// 共享的应用状态
#[derive(Debug)]
struct AppState {
    screen_map: ScrAreaMap,
    ui_settings: UIScaleSettings,
    initialized: bool,
}

pub struct AppWindowManager {
    ui: MainWindow,
    state: Arc<Mutex<AppState>>,
}

impl AppWindowManager {
    pub fn new() -> Result<Self> {
        let ui = MainWindow::new().context("创建 Slint 主窗口失败")?;

        let state = Arc::new(Mutex::new(AppState {
            screen_map: ScrAreaMap::new(),
            ui_settings: UIScaleSettings::default(),
            initialized: false,
        }));

        Ok(Self { ui, state })
    }

    pub fn run(&mut self) -> Result<()> {
        // 安排在窗口显示后初始化屏幕布局（异步执行）
        let window_weak = self.ui.as_weak();
        let state_clone = Arc::clone(&self.state);

        if let Err(e) = slint::invoke_from_event_loop(move || {
            if let Some(app) = window_weak.upgrade() {
                let physical_window_size = app.window().size(); // 这返回的是物理像素大小
                let scale_factor = app.window().scale_factor();

                println!(
                    "窗口已初始化，物理尺寸: {}x{}, 缩放因子: {:.2}",
                    physical_window_size.width, physical_window_size.height, scale_factor
                );

                // 初始化应用状态
                let area_key: Option<ScrAreaKey> = {
                    match state_clone.lock() {
                        Ok(mut state) => {
                            if !state.initialized {
                                state.ui_settings =
                                    UIScaleSettings::from_slint_window(&app.window());

                                // 克隆UI设置以避免借用冲突
                                let ui_settings = state.ui_settings.clone();
                                let area_key = state.screen_map.create_initial_screen_with_scaling(
                                    physical_window_size.width,  // 使用物理像素进行内部计算
                                    physical_window_size.height, // 使用物理像素进行内部计算
                                    &ui_settings,
                                );

                                state.initialized = true;
                                Some(area_key)
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            eprintln!("初始化时锁获取失败: {}", e);
                            None
                        }
                    }
                }; // 作用域结束，自动释放锁

                // 打印区域信息并传递给Slint UI
                if let Some(area_key) = area_key {
                    let slint_area_data = if let Ok(state) = state_clone.lock() {
                        if let Some(area) = state.screen_map.areas.get(area_key) {
                            let (geometry_rect, winx, winy) = area.geometry();
                            println!("区域 {:?}: {}x{} (物理像素)", area_key, winx, winy);

                            // 转换为逻辑像素（Slint期望的坐标系统）
                            let scale_factor = state.ui_settings.scale_factor;
                            let logical_x = geometry_rect.xmin as f32 / scale_factor;
                            let logical_y = geometry_rect.ymin as f32 / scale_factor;
                            let logical_width = winx as f32 / scale_factor;
                            let logical_height = winy as f32 / scale_factor;

                            vec![ScrAreaData {
                                key: area_key.data().as_ffi() as i32,
                                x: logical_x,
                                y: logical_y,
                                width: logical_width,
                                height: logical_height,
                            }]
                        } else {
                            println!("区域 {:?} 不存在", area_key);
                            vec![]
                        }
                    } else {
                        println!("无法获取状态锁");
                        vec![]
                    };

                    // 使用Slint自动生成的setter方法传递数据
                    app.set_scrareas(slint_area_data.as_slice().into());
                    app.on_actionzone_drag({
                        move |area_key, corner, x, y| {
                            println!(
                                "回调数据: area_key: {area_key}, corner: {:?}, x: {x}, y: {y}",
                                corner
                            )
                        }
                    });

                    println!("传递给Slint UI（逻辑像素）: {:?}", slint_area_data);
                }
            }
        }) {
            eprintln!("安排初始化任务失败: {}", e);
        }

        // 直接启动 UI 事件循环（初始化会在窗口显示后异步执行）
        self.ui.run().context("启动 UI 事件循环失败")
    }

    /// 演示如何在窗口大小或DPI变化时更新布局
    pub fn handle_window_change(&mut self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| anyhow::anyhow!("无法获取状态锁: {}", e))?;

        let physical_window_size = self.ui.window().size(); // 这返回物理像素大小
        let scale_factor = self.ui.window().scale_factor();

        // 更新UI设置
        state.ui_settings = UIScaleSettings::from_slint_window(&self.ui.window());

        // 计算新的窗口矩形（使用物理像素）
        let window_rect = screen::types::Rcti {
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
    pub fn is_initialized(&self) -> bool {
        self.state.lock().map_or_else(
            |e| {
                eprintln!("检查初始化状态时锁获取失败: {}", e);
                false
            },
            |state| state.initialized,
        )
    }
}

// 导出一个公共函数，用于创建和运行窗口
pub fn create_window() -> Result<()> {
    let mut app = AppWindowManager::new().context("创建应用窗口管理器失败（是否nix develope?）")?;
    app.run().context("运行应用失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_scale_calculation() {
        // 模拟不同缩放因子下的计算
        let mut ui_settings = UIScaleSettings::default();

        // 1x缩放
        ui_settings.scale_factor = 1.0;
        assert_eq!(ui_settings.ui_scale_fac(), 1.0);
        assert_eq!(ui_settings.calculate_margin(false), 1);

        // 2x缩放
        ui_settings.scale_factor = 2.0;
        assert_eq!(ui_settings.ui_scale_fac(), 2.0);
        assert_eq!(ui_settings.calculate_margin(false), 2);

        // 1.5x缩放
        ui_settings.scale_factor = 1.5;
        assert_eq!(ui_settings.ui_scale_fac(), 1.5);
        assert_eq!(ui_settings.calculate_margin(false), 1); // 向下取整
    }

    #[test]
    fn test_area_size_calculation() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UIScaleSettings::default();

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
