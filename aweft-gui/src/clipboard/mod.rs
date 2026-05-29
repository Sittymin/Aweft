//! 剪贴板子系统（精简版）。
//!
//! 目标：
//! - 同进程高保真：使用本地强类型缓存（`ItemsClip`）。
//! - 跨应用互通：仅通过系统剪贴板 `text/plain`。
//!
//! 说明：
//! - 不再在内部维护多 MIME 协议与字符串协商；
//! - 业务层通过 `Clipboard` 的强类型方法读写（如 `copy_items` / `paste_items`）。

pub mod backend;
pub mod facade;
pub mod kinds;

pub use facade::Clipboard;
pub use kinds::ItemsClip;
