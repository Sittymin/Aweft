//! 剪贴板后端抽象（文本版）。
//!
//! - `ClipboardBackend`：仅负责系统剪贴板文本读写。
//! - `default_backend`：按编译目标自动选择实现。桌面接系统剪贴板，
//!   其他平台退化为内存实现。

use anyhow::Result;

pub mod memory;

#[cfg(target_os = "android")]
pub mod android;

// arboard 在 Android 上没有实现（该包 platform 模块在该目标为空），
#[cfg(not(target_os = "android"))]
pub mod system;

pub trait ClipboardBackend: Send {
    fn write_text(&mut self, text: &str) -> Result<()>;
    fn read_text(&mut self) -> Result<Option<String>>;
}

/// 平台默认 backend。
///
/// 桌面：尝试系统剪贴板，失败回退到内存（避免崩溃）。
/// 其他：内存实现。
pub fn default_backend() -> Box<dyn ClipboardBackend> {
    #[cfg(target_os = "android")]
    {
        return Box::new(android::AndroidBackend);
    }
    #[cfg(not(target_os = "android"))]
    {
        match system::SystemBackend::new() {
            Ok(backend) => return Box::new(backend),
            Err(e) => log::warn!("初始化系统剪贴板失败，回退到内存实现：{}", e),
        }
    }

    #[cfg(not(target_os = "android"))]
    return Box::new(memory::InMemoryBackend::default());
}
