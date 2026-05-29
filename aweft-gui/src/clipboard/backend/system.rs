//! 桌面系统剪贴板后端（基于 arboard）。
//!
//! 仅承担"跨应用文本互通"角色：读写 `text/plain`。

use anyhow::{Result, anyhow};

use crate::clipboard::backend::ClipboardBackend;

pub struct SystemBackend {
    inner: arboard::Clipboard,
}

impl SystemBackend {
    pub fn new() -> Result<Self> {
        let inner =
            arboard::Clipboard::new().map_err(|e| anyhow!("初始化系统剪贴板失败: {}", e))?;
        Ok(Self { inner })
    }
}

impl ClipboardBackend for SystemBackend {
    fn write_text(&mut self, text: &str) -> Result<()> {
        self.inner
            .set_text(text)
            .map_err(|e| anyhow!("写入系统剪贴板失败: {}", e))?;
        Ok(())
    }

    fn read_text(&mut self) -> Result<Option<String>> {
        match self.inner.get_text() {
            Ok(text) => Ok(Some(text)),
            // 剪贴板里没有可读文本（图像或别的）：等价于空
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(anyhow!("读取系统剪贴板失败: {}", e)),
        }
    }
}
