pub mod actions;
pub mod app;
pub mod bridge;
pub mod clipboard;
pub mod collab_bridge;
pub mod depsgraph;
pub mod file_picker;
pub mod plugin_bridge;
pub mod projections;
pub mod reducers;
pub mod render;
pub mod screen;
pub mod store;

// 测试专用：GStreamer 解码管线在本进程内并发启动会争抢 VA/解码资源导致互相拖慢、
// 背压测试时序不稳。所有真正解码的测试持此锁串行执行（不影响非 GStreamer 测试并行）。
#[cfg(test)]
pub(crate) static GST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

use anyhow::{Context, Result};

// 导出一个公共函数，用于创建和运行窗口
pub fn create_window() -> Result<()> {
    let mut app =
        app::AppWindowManager::new().context("创建应用窗口管理器失败（是否nix develop?）")?;
    app.run().context("运行应用失败")
}
