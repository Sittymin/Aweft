//! 进程内内存剪贴板（文本版）。
//!
//! - 嵌入式 / wasm 等无系统剪贴板的平台默认 backend。
//! - 桌面 backend 故障时的 fallback。
//! - 单元测试使用。

use anyhow::Result;

use crate::clipboard::backend::ClipboardBackend;

#[derive(Default)]
pub struct InMemoryBackend {
    text: Option<String>,
}

impl ClipboardBackend for InMemoryBackend {
    fn write_text(&mut self, text: &str) -> Result<()> {
        self.text = Some(text.to_string());
        Ok(())
    }

    fn read_text(&mut self) -> Result<Option<String>> {
        Ok(self.text.clone())
    }
}
