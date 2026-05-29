//! `Clipboard` 门面：业务层唯一入口（强类型 + 系统文本）。
//!
//! 双轨制：
//! - `last_local`：同进程高保真（强类型）。
//! - `backend`：系统剪贴板文本（跨应用兜底）。

use anyhow::Result;

use crate::clipboard::backend::{self, ClipboardBackend};
use crate::clipboard::kinds::ItemsClip;

#[derive(Clone)]
enum LocalClip {
    Items(ItemsClip),
}

pub struct Clipboard {
    backend: Box<dyn ClipboardBackend>,
    last_local: Option<LocalClip>,
}

impl Clipboard {
    pub fn new() -> Self {
        Self {
            backend: backend::default_backend(),
            last_local: None,
        }
    }

    pub fn with_backend(backend: Box<dyn ClipboardBackend>) -> Self {
        Self {
            backend,
            last_local: None,
        }
    }

    /// 复制字幕条目：
    /// - 本地缓存保留高保真结构；
    /// - 系统剪贴板写 plain text（失败仅 warn，不影响本地缓存）。
    pub fn copy_items(&mut self, clip: &ItemsClip) -> Result<()> {
        let text = clip.to_plain_text();
        if let Err(e) = self.backend.write_text(&text) {
            log::warn!("写入系统剪贴板失败（仅作用于内部缓存）：{}", e);
        }
        self.last_local = Some(LocalClip::Items(clip.clone()));
        Ok(())
    }

    /// 粘贴字幕条目：
    /// - 优先返回本地高保真缓存；
    /// - 否则读取系统文本（当前不支持 text 反序列化为结构，返回 None）。
    pub fn paste_items(&mut self) -> Result<Option<ItemsClip>> {
        if let Some(LocalClip::Items(clip)) = &self.last_local {
            return Ok(Some(clip.clone()));
        }

        // TODO: 未来需要也尝试解析系统剪贴板
        let _ = self.backend.read_text()?;
        Ok(None)
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}
