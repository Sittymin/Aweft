use crate::types::{ScrAreaMap, UISettings};
use std::sync::{LazyLock, RwLock};
use anyhow::Result;

#[derive(Debug)]
pub struct AppState {
    pub screen_map: ScrAreaMap,
    pub ui_settings: UISettings,
    pub initialized: bool,
}

// 全局静态状态，使用 RwLock（读多写少场景的最佳选择）
pub static APP_STATE: LazyLock<RwLock<AppState>> = LazyLock::new(|| {
    println!("[app_state.rs] Initializing global APP_STATE with RwLock...");
    RwLock::new(AppState::default())
});

// 2. 提供一个默认实现，作为初始化的基础
impl Default for AppState {
    fn default() -> Self {
        AppState {
            screen_map: ScrAreaMap::new(),
            ui_settings: UISettings::default(),
            initialized: false,
        }
    }
}

/// 获取全局应用状态的读锁（高性能，可并发）
pub fn get_app_state() -> Result<std::sync::RwLockReadGuard<'static, AppState>> {
    APP_STATE
        .read()
        .map_err(|e| anyhow::anyhow!("无法获取应用状态读锁: {}", e))
}

/// 获取全局应用状态的写锁（独占访问）
pub fn get_app_state_mut() -> Result<std::sync::RwLockWriteGuard<'static, AppState>> {
    APP_STATE
        .write()
        .map_err(|e| anyhow::anyhow!("无法获取应用状态写锁: {}", e))
}