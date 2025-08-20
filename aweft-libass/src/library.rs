use crate::ffi;
use crate::renderer::DefaultFontProvider;
use anyhow::{anyhow, Result as Result};
use libc::{c_char, c_int, c_void};
use std::collections::HashMap;
use std::ffi::{CString};
use std::ptr;
use std::slice;
use std::sync::{Arc, LazyLock, Mutex};

// ============================================================================
// 消息回调类型定义
// ============================================================================

/// 消息级别枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLevel {
    Fatal = 0,
    Error = 1,
    Warning = 2,
    Info = 3,
    Verbose = 4,
    Debug = 5,
    Trace = 6,
    Unknown = 7,
}

impl From<i32> for MessageLevel {
    fn from(level: i32) -> Self {
        match level {
            0 => MessageLevel::Fatal,
            1 => MessageLevel::Error,
            2 => MessageLevel::Warning,
            3 => MessageLevel::Info,
            4 => MessageLevel::Verbose,
            5 => MessageLevel::Debug,
            6 => MessageLevel::Trace,
            _ => MessageLevel::Unknown,
        }
    }
}

/// 消息回调函数类型
pub type MessageCallback = dyn Fn(MessageLevel, &str) + Send + Sync;

// 全局回调存储：使用库指针作为键来存储回调
static GLOBAL_CALLBACKS: LazyLock<Mutex<HashMap<usize, Arc<MessageCallback>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 从 C 端调用的 Rust 函数（通过 trampoline）
#[unsafe(no_mangle)]
pub extern "C" fn rust_handle_message_callback(
    level: i32,
    message: *const c_char,
    data: *mut std::ffi::c_void,
) {
    if message.is_null() {
        return;
    }

    let level = MessageLevel::from(level);
    let message_str = unsafe {
        match std::ffi::CStr::from_ptr(message).to_str() {
            Ok(s) => s,
            Err(_) => return, // 无效的 UTF-8，忽略
        }
    };

    // data 是库指针，用于查找对应的回调
    let library_ptr = data as usize;

    if let Ok(callbacks) = GLOBAL_CALLBACKS.lock() {
        if let Some(callback) = callbacks.get(&library_ptr) {
            callback(level, message_str);
        }
    }
}

// ============================================================================
// Library 封装
// ============================================================================

pub struct Library {
    ptr: *mut ffi::ASS_Library,
}

impl Library {
    pub fn new() -> Result<Self> {
        unsafe {
            let ptr = ffi::ass_library_init();
            if ptr.is_null() {
                return Err(anyhow!("ass_library_init 失败"));
            }
            Ok(Self { ptr })
        }
    }

    pub fn version() -> i32 {
        unsafe { ffi::ass_library_version() }
    }

    pub fn set_fonts_dir(&mut self, fonts_dir: &str) -> Result<()> {
        let c_fonts_dir = CString::new(fonts_dir)
            .map_err(|e| anyhow!("字体目录包含无效字符: {}", e))?;
        unsafe {
            ffi::ass_set_fonts_dir(self.ptr, c_fonts_dir.as_ptr());
        }
        Ok(())
    }

    pub fn set_extract_fonts(&mut self, extract: bool) {
        unsafe {
            ffi::ass_set_extract_fonts(self.ptr, if extract { 1 } else { 0 });
        }
    }

    /// 设置消息回调函数
    ///
    /// # 参数
    /// * `callback` - 消息回调函数，接收消息级别和消息内容
    ///
    /// # 示例
    /// ```rust
    /// use aweft_libass::*;
    ///
    /// let mut library = Library::new().unwrap();
    /// library.set_message_callback(|level, message| {
    ///     match level {
    ///         MessageLevel::Error | MessageLevel::Fatal => {
    ///             eprintln!("libass 错误: {}", message);
    ///         }
    ///         MessageLevel::Warning => {
    ///             println!("libass 警告: {}", message);
    ///         }
    ///         MessageLevel::Info => {
    ///             println!("libass 信息: {}", message);
    ///         }
    ///         _ => {
    ///             // 忽略调试消息
    ///         }
    ///     }
    /// });
    /// ```
    pub fn set_message_callback<F>(&self, callback: F) -> Result<()>
    where
        F: Fn(MessageLevel, &str) + Send + Sync + 'static,
    {
        let callback_arc = Arc::new(callback);
        let library_ptr = self.ptr as usize;

        // 存储回调到全局映射中
        {
            let mut callbacks = GLOBAL_CALLBACKS
                .lock()
                .map_err(|_| anyhow!("无法获取回调锁"))?;
            callbacks.insert(library_ptr, callback_arc);
        }

        // 调用 C trampoline 函数
        unsafe {
            ffi::set_rust_message_callback(self.ptr, self.ptr as *mut std::ffi::c_void);
        }

        Ok(())
    }

    /// 清除消息回调函数
    pub fn clear_message_callback(&self) -> Result<()> {
        let library_ptr = self.ptr as usize;

        // 从全局映射中移除回调
        {
            let mut callbacks = GLOBAL_CALLBACKS
                .lock()
                .map_err(|_| anyhow!("无法获取回调锁"))?;
            callbacks.remove(&library_ptr);
        }

        // 清除 C 端的回调
        unsafe {
            ffi::clear_message_callback(self.ptr);
        }

        Ok(())
    }

    pub fn get_available_font_providers(&self) -> Vec<DefaultFontProvider> {
        let mut providers_ptr: *mut ffi::ASS_DefaultFontProvider = ptr::null_mut();
        let mut size: libc::size_t = 0;

        unsafe {
            ffi::ass_get_available_font_providers(self.ptr, &mut providers_ptr, &mut size);
        }

        if size == usize::MAX || providers_ptr.is_null() {
            return Vec::new();
        }

        unsafe {
            let c_slice: &[ffi::ASS_DefaultFontProvider] =
                slice::from_raw_parts(providers_ptr, size);

            let rust_vec: Vec<DefaultFontProvider> = c_slice
                .iter()
                .map(|&c_provider| DefaultFontProvider::from(c_provider))
                .collect();

            libc::free(providers_ptr as *mut libc::c_void);

            rust_vec
        }
    }

    pub fn add_font(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let c_name = CString::new(name)
            .map_err(|e| anyhow!("字体名包含无效字符: {}", e))?;

        unsafe {
            ffi::ass_add_font(
                self.ptr,
                c_name.as_ptr(),
                data.as_ptr() as *const c_void,
                data.len() as c_int,
            );
        };
        Ok(())
    }

    pub fn clear_fonts(&mut self) {
        unsafe {
            ffi::ass_clear_fonts(self.ptr);
        }
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::ASS_Library {
        self.ptr
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // 清理回调（忽略错误，因为在 Drop 中）
        let _ = self.clear_message_callback();

        unsafe {
            ffi::ass_library_done(self.ptr);
        }
    }
}
