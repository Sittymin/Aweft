slint::include_modules!();

// 导出一个公共函数，用于创建和运行窗口
pub fn run() -> Result<(), slint::PlatformError> {
    // 创建 AppWindow 组件的实例
    let ui = AppWindow::new()?;

    // 运行 UI 主循环
    ui.run()
}
