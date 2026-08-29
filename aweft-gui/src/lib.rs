pub mod actions;
pub mod app;
pub mod bridge;
pub mod clipboard;
pub mod depsgraph;
pub mod projections;
pub mod reducers;
pub mod render;
pub mod screen;
pub mod store;

use anyhow::{Context, Result};

// 导出一个公共函数，用于创建和运行窗口
pub fn create_window() -> Result<()> {
    let mut app =
        app::AppWindowManager::new().context("创建应用窗口管理器失败（是否nix develop?）")?;
    app.run().context("运行应用失败")
}
