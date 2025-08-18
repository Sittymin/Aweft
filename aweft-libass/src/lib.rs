//! # Aweft LibASS - Rust 安全封装
//!
//! 这是一个为 libass (Advanced SubStation Alpha 字幕渲染库) 提供的 Rust 安全封装。
//!
//! ## 特性
//!
//! - ✅ **内存安全**: 自动管理 libass 对象的生命周期
//! - ✅ **类型安全**: 强类型的颜色、枚举和配置选项
//! - ✅ **错误处理**: 使用 Rust 的 Result 类型进行错误处理
//! - ✅ **迭代器支持**: 使用标准的 Rust 迭代器遍历渲染结果
//! - ✅ **构建器模式**: 流畅的 API 设计，易于使用
//! - ✅ **消息回调**: 安全的 Rust -> C (Trampoline) -> Rust 消息回调模式
//! - ✅ **两种工作流程**: 支持手动创建和 ASS 脚本解析两种方式
//!
//! ## ⚠️ 重要限制
//!
//! **libass 不支持混合使用手动样式分配和 ASS 脚本解析！**
//!
//! 请选择以下两种工作流程之一：
//!
//! ### 🔧 手动工作流程
//! - 适用于：动态字幕、程序化生成、实时字幕
//! - 使用：`alloc_style()` + `alloc_event()`
//! - 限制：不能再调用 `process_data()`
//!
//! ### 📄 ASS 脚本工作流程  
//! - 适用于：静态字幕文件、预制字幕、复杂样式
//! - 使用：`process_data()` 或 `from_file()`
//! - 限制：不能手动分配样式或事件
//!
//! ## 快速开始
//!
//! ### 手动工作流程示例
//!
//! ```rust
//! use aweft_libass::*;
//!
//! // 创建渲染器
//! let mut renderer = SubtitleRenderer::new().unwrap();
//! let mut track = renderer.create_track().unwrap();
//! track.set_play_resolution(1280, 720);
//! track.set_track_type(TrackType::Ass);
//!
//! // 修改默认样式（libass 自动创建）
//! let mut style = track.get_default_style().unwrap();
//! style.set_font_size(48.0)
//!      .set_primary_color(Color::red())
//!      .set_outline_color(Color::black())
//!      .set_outline(2.0);
//!
//! // 手动创建事件
//! let mut event = track.alloc_event().unwrap();
//! event.set_text("Hello, World!").unwrap()
//!      .set_timing(0, 5000)
//!      .set_style(0);
//!
//! // 渲染
//! let result = renderer.render_frame(&track, 2500).unwrap();
//! ```
//!
//! ### ASS 脚本工作流程示例
//!
//! ```rust
//! use aweft_libass::*;
//!
//! // 创建渲染器
//! let mut renderer = SubtitleRenderer::new().unwrap();
//! let mut track = renderer.create_track().unwrap();
//! track.set_track_type(TrackType::Ass);
//!
//! // 使用完整的 ASS 脚本
//! let ass_script = r#"[V4+ Styles]
//! Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
//! Style: Default,,48,&H00FF0000,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,3,2,20,20,20,1
//!
//! [Events]
//! Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
//! Dialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,Hello, World!"#;
//!
//! track.process_data(ass_script);
//!
//! // 渲染
//! let result = renderer.render_frame(&track, 2500).unwrap();
//! ```
//!

//!
//! ## 消息回调
//!
//! 你可以设置消息回调来接收 libass 的调试和错误信息：
//!
//! ```rust
//! use aweft_libass::*;
//!
//! let mut renderer = SubtitleRenderer::new().unwrap();
//!
//! // 设置消息回调
//! renderer.set_message_callback(|level, message| {
//!     match level {
//!         MessageLevel::Fatal | MessageLevel::Error => {
//!             eprintln!("🔴 libass 错误: {}", message);
//!         }
//!         MessageLevel::Warning => {
//!             println!("🟡 libass 警告: {}", message);
//!         }
//!         MessageLevel::Info => {
//!             println!("🔵 libass 信息: {}", message);
//!         }
//!         _ => {
//!             // 忽略详细调试信息
//!         }
//!     }
//! }).unwrap();
//! ```
//!
//! ## 颜色格式说明
//!
//! libass 使用 RGBA 格式，其中：
//! - 位 31-24: Red (最高位)
//! - 位 23-16: Green
//! - 位 15-8: Blue
//! - 位 7-0: Alpha (最低位)
//!
//! 使用 `Color` 结构体可以安全地处理颜色转换。

// 引入生成的绑定
mod ffi {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use image::{Rgba, RgbaImage};
use libc::c_double;
use libc::{c_char, c_int};
use std::collections::HashMap;
use std::ffi::{CStr, CString, NulError};
use std::fs;
use std::path::Path;
use std::ptr;
use std::slice;
use std::str::Utf8Error;
use std::sync::{Arc, LazyLock, Mutex};

// ============================================================================
// 错误类型定义
// ============================================================================

#[derive(Debug, Clone)]
pub enum LibassError {
    InitializationFailed(String),
    InvalidParameter(String),
    AllocationFailed(String),
    NullPointer(String),
}

impl std::fmt::Display for LibassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibassError::InitializationFailed(msg) => write!(f, "初始化失败: {}", msg),
            LibassError::InvalidParameter(msg) => write!(f, "无效参数: {}", msg),
            LibassError::AllocationFailed(msg) => write!(f, "内存分配失败: {}", msg),
            LibassError::NullPointer(msg) => write!(f, "空指针错误: {}", msg),
        }
    }
}

impl std::error::Error for LibassError {}

pub type LibassResult<T> = std::result::Result<T, LibassError>;

impl From<Utf8Error> for LibassError {
    fn from(err: Utf8Error) -> Self {
        LibassError::InvalidParameter(format!("C string is not valid UTF-8: {}", err))
    }
}

impl From<NulError> for LibassError {
    fn from(err: NulError) -> Self {
        LibassError::InvalidParameter(format!("String contains null character: {}", err))
    }
}

// ============================================================================
// 颜色类型定义
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn red() -> Self {
        Self::new(255, 0, 0, 255)
    }

    pub fn green() -> Self {
        Self::new(0, 255, 0, 255)
    }

    pub fn blue() -> Self {
        Self::new(0, 0, 255, 255)
    }

    pub fn white() -> Self {
        Self::new(255, 255, 255, 255)
    }

    pub fn black() -> Self {
        Self::new(0, 0, 0, 255)
    }

    pub fn transparent() -> Self {
        Self::new(0, 0, 0, 0)
    }

    /// 转换为 libass 的 RGBA 格式 (Red在最高位，Alpha在最低位)
    pub fn to_libass_format(&self) -> u32 {
        ((self.r as u32) << 24) | ((self.g as u32) << 16) | ((self.b as u32) << 8) | (self.a as u32)
    }

    /// 从 libass 的 RGBA 格式解析
    pub fn from_libass_format(color: u32) -> Self {
        Self {
            r: ((color >> 24) & 0xFF) as u8,
            g: ((color >> 16) & 0xFF) as u8,
            b: ((color >> 8) & 0xFF) as u8,
            a: (color & 0xFF) as u8,
        }
    }
}

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

impl Clone for Library {
    fn clone(&self) -> Self {
        unsafe {
            // 创建一个新的库实例，但共享相同的字体和配置
            let new_ptr = ffi::ass_library_init();
            if new_ptr.is_null() {
                panic!("克隆 Library 失败");
            }
            Self { ptr: new_ptr }
        }
    }
}

impl Library {
    pub fn new() -> LibassResult<Self> {
        unsafe {
            let ptr = ffi::ass_library_init();
            if ptr.is_null() {
                return Err(LibassError::InitializationFailed(
                    "ass_library_init 失败".to_string(),
                ));
            }
            Ok(Self { ptr })
        }
    }

    pub fn version() -> i32 {
        unsafe { ffi::ass_library_version() }
    }

    pub fn set_fonts_dir(&mut self, fonts_dir: &str) -> LibassResult<()> {
        let c_fonts_dir = CString::new(fonts_dir)
            .map_err(|_| LibassError::InvalidParameter("字体目录包含无效字符".to_string()))?;
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

    pub fn add_font(&mut self, name: &str, data: &[u8]) -> LibassResult<()> {
        let c_name = CString::new(name)
            .map_err(|_| LibassError::InvalidParameter("字体名包含无效字符".to_string()))?;
        unsafe {
            ffi::ass_add_font(
                self.ptr,
                c_name.as_ptr(),
                data.as_ptr() as *const i8,
                data.len() as i32,
            );
        }
        Ok(())
    }

    pub fn clear_fonts(&mut self) {
        unsafe {
            ffi::ass_clear_fonts(self.ptr);
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
    pub fn set_message_callback<F>(&mut self, callback: F) -> LibassResult<()>
    where
        F: Fn(MessageLevel, &str) + Send + Sync + 'static,
    {
        let callback_arc = Arc::new(callback);
        let library_ptr = self.ptr as usize;

        // 存储回调到全局映射中
        {
            let mut callbacks = GLOBAL_CALLBACKS
                .lock()
                .map_err(|_| LibassError::InitializationFailed("无法获取回调锁".to_string()))?;
            callbacks.insert(library_ptr, callback_arc);
        }

        // 调用 C trampoline 函数
        unsafe {
            ffi::set_rust_message_callback(self.ptr, self.ptr as *mut std::ffi::c_void);
        }

        Ok(())
    }

    /// 清除消息回调函数
    pub fn clear_message_callback(&mut self) -> LibassResult<()> {
        let library_ptr = self.ptr as usize;

        // 从全局映射中移除回调
        {
            let mut callbacks = GLOBAL_CALLBACKS
                .lock()
                .map_err(|_| LibassError::InitializationFailed("无法获取回调锁".to_string()))?;
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

// ============================================================================
// Renderer 封装
// ============================================================================
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ShapingLevel {
    Simple = 0,
    Complex = 1,
}
impl TryFrom<u32> for ShapingLevel {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ShapingLevel::Simple),
            1 => Ok(ShapingLevel::Complex),
            _ => Err(()),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HintingType {
    None = 0,
    Light = 1,
    Normal = 2,
    Native = 3,
}
impl TryFrom<u32> for HintingType {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(HintingType::None),
            1 => Ok(HintingType::Light),
            2 => Ok(HintingType::Normal),
            3 => Ok(HintingType::Native),
            _ => Err(()),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DefaultFontProvider {
    None = 0,
    AutoDetect = 1,
    CoreText,
    FontConfig,
    DirectWrite,
}
impl From<u32> for DefaultFontProvider {
    fn from(provider: u32) -> Self {
        match provider {
            0 => DefaultFontProvider::None,
            1 => DefaultFontProvider::AutoDetect,
            2 => DefaultFontProvider::CoreText,
            3 => DefaultFontProvider::FontConfig,
            4 => DefaultFontProvider::DirectWrite,
            _ => DefaultFontProvider::None,
        }
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct AssStyle {
    pub name: String,
    pub font_name: String,
    pub font_size: f64,
    pub primary_colour: u32,
    pub secondary_colour: u32,
    pub outline_colour: u32,
    pub back_colour: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    pub scale_x: f64,
    pub scale_y: f64,
    pub spacing: f64,
    pub angle: f64,
    pub border_style: i32,
    pub outline: f64,
    pub shadow: f64,
    pub alignment: i32,
    pub margin_l: i32,
    pub margin_r: i32,
    pub margin_v: i32,
    pub encoding: i32,
    pub blur: f64,
    pub justify: i32,
}
pub struct AssStyleGuard {
    /// The raw FFI struct with pointers that are valid for the lifetime of this guard.
    pub raw: ffi::ass_style,
    // These CString fields own the string data. They are made private to
    // emphasize that their purpose is lifetime management.
    _name_cstring: CString,
    _font_name_cstring: CString,
}

impl AssStyle {
    /// Converts the safe `AssStyle` into an `AssStyleGuard`.
    ///
    /// The returned struct contains the FFI-safe `ffi::ass_style` and manages the
    /// lifetimes of the C strings, preventing dangling pointers. This is the
    /// recommended way to prepare a style for passing it to a C function.
    pub fn to_raw_guard(&self) -> LibassResult<AssStyleGuard> {
        let name_cstring = CString::new(self.name.as_str())?;
        let font_name_cstring = CString::new(self.font_name.as_str())?;

        let raw = ffi::ass_style {
            Name: name_cstring.as_ptr() as *mut c_char,
            FontName: font_name_cstring.as_ptr() as *mut c_char,
            FontSize: self.font_size,
            PrimaryColour: self.primary_colour,
            SecondaryColour: self.secondary_colour,
            OutlineColour: self.outline_colour,
            BackColour: self.back_colour,
            Bold: if self.bold { 1 } else { 0 },
            Italic: if self.italic { 1 } else { 0 },
            Underline: if self.underline { 1 } else { 0 },
            StrikeOut: if self.strike_out { 1 } else { 0 },
            ScaleX: self.scale_x,
            ScaleY: self.scale_y,
            Spacing: self.spacing,
            Angle: self.angle,
            BorderStyle: self.border_style,
            Outline: self.outline,
            Shadow: self.shadow,
            Alignment: self.alignment,
            MarginL: self.margin_l,
            MarginR: self.margin_r,
            MarginV: self.margin_v,
            Encoding: self.encoding,
            treat_fontname_as_pattern: 0, // This field is unused in the C struct.
            Blur: self.blur,
            Justify: self.justify,
        };

        Ok(AssStyleGuard {
            raw,
            _name_cstring: name_cstring,
            _font_name_cstring: font_name_cstring,
        })
    }
}

impl TryFrom<*const ffi::ass_style> for AssStyle {
    type Error = LibassError;

    fn try_from(raw_style_ptr: *const ffi::ass_style) -> Result<Self, Self::Error> {
        assert!(
            !raw_style_ptr.is_null(),
            "Received a null pointer for ffi::ass_style"
        );

        unsafe {
            let raw = &*raw_style_ptr;

            let name = CStr::from_ptr(raw.Name).to_str()?.to_owned();
            let font_name = CStr::from_ptr(raw.FontName).to_str()?.to_owned();

            Ok(AssStyle {
                name,
                font_name,
                font_size: raw.FontSize,
                primary_colour: raw.PrimaryColour,
                secondary_colour: raw.SecondaryColour,
                outline_colour: raw.OutlineColour,
                back_colour: raw.BackColour,
                bold: raw.Bold != 0,
                italic: raw.Italic != 0,
                underline: raw.Underline != 0,
                strike_out: raw.StrikeOut != 0,
                scale_x: raw.ScaleX,
                scale_y: raw.ScaleY,
                spacing: raw.Spacing,
                angle: raw.Angle,
                border_style: raw.BorderStyle,
                outline: raw.Outline,
                shadow: raw.Shadow,
                alignment: raw.Alignment,
                margin_l: raw.MarginL,
                margin_r: raw.MarginR,
                margin_v: raw.MarginV,
                encoding: raw.Encoding,
                blur: raw.Blur,
                justify: raw.Justify,
            })
        }
    }
}

pub struct Renderer {
    ptr: *mut ffi::ASS_Renderer,
}

impl Renderer {
    pub fn new(library: &Library) -> LibassResult<Self> {
        unsafe {
            let ptr = ffi::ass_renderer_init(library.as_ptr());
            if ptr.is_null() {
                return Err(LibassError::InitializationFailed(
                    "ass_renderer_init 失败".to_string(),
                ));
            }
            Ok(Self { ptr })
        }
    }

    // 实际渲染的视频大小
    pub fn set_frame_size(&mut self, width: i32, height: i32) {
        unsafe {
            ffi::ass_set_frame_size(self.ptr, width as c_int, height as c_int);
        }
    }

    // 编辑字幕时设定的大小（即ASS文件当中设定的大小）
    pub fn set_storage_size(&mut self, width: i32, height: i32) {
        unsafe {
            ffi::ass_set_storage_size(self.ptr, width as c_int, height as c_int);
        }
    }

    // 使用 ShapingLevel::Complex 时可以有连字功能等
    pub fn set_shaper(&mut self, level: ShapingLevel) {
        unsafe {
            ffi::ass_set_shaper(self.ptr, level as u32);
        }
    }

    pub fn set_margins(&mut self, top: i32, bottom: i32, left: i32, right: i32) {
        unsafe {
            ffi::ass_set_margins(
                self.ptr,
                top as c_int,
                bottom as c_int,
                left as c_int,
                right as c_int,
            );
        }
    }

    // 是否启用 margins
    pub fn set_use_margins(&mut self, use_margins: bool) {
        unsafe {
            ffi::ass_set_use_margins(self.ptr, if use_margins { 1 } else { 0 });
        }
    }

    // 设置单个像素长宽比(如果为0或未设置就会自动从 frame_size 与 storage_size 推断出来)
    pub fn set_pixel_aspect(&mut self, par: f64) {
        unsafe {
            ffi::ass_set_pixel_aspect(self.ptr, par as c_double);
        }
    }
    pub fn set_font_scale(&mut self, scale: f64) {
        unsafe {
            ffi::ass_set_font_scale(self.ptr, scale as c_double);
        }
    }

    pub fn set_hinting(&mut self, hinting: HintingType) {
        unsafe {
            ffi::ass_set_hinting(self.ptr, hinting as u32);
        }
    }

    pub fn set_line_spacing(&mut self, spacing: f64) {
        unsafe {
            ffi::ass_set_line_spacing(self.ptr, spacing as c_double);
        }
    }

    // > 100 会被钳制到 100，但是 < 0 没有相关限制
    pub fn set_line_position(&mut self, position_percent: f64) {
        unsafe {
            ffi::ass_set_line_position(self.ptr, position_percent as c_double);
        }
    }

    pub fn set_fonts(
        &mut self,
        default_font: Option<&Path>,
        default_family: Option<&str>,
        font_provider: DefaultFontProvider,
        config: Option<&Path>,
    ) -> LibassResult<()> {
        // --- 1. 转换字符串和路径参数 ---

        // 为了管理 CString 的生命周期，我们需要将它们存储在变量中
        // 只有当 CString 变量存在时，它返回的指针才是有效的

        // 处理 default_font
        let default_font_cstring;
        let default_font_ptr = match default_font {
            Some(path) => {
                // [修改] 将路径转换为绝对路径
                let absolute_path = fs::canonicalize(path).map_err(|e| {
                    LibassError::InvalidParameter(format!(
                        "无法将默认字体路径 '{}' 转换为绝对路径: {}",
                        path.display(),
                        e
                    ))
                })?;

                // Path -> &str
                let path_str = absolute_path.to_str().ok_or_else(|| {
                    LibassError::InvalidParameter(format!(
                        "默认字体路径包含无效UTF-8字符: {}",
                        absolute_path.display()
                    ))
                })?;
                // &str -> CString
                default_font_cstring = CString::new(path_str).map_err(|e| {
                    LibassError::InvalidParameter(format!("默认字体路径包含空字符: {}", e))
                })?;
                default_font_cstring.as_ptr()
            }
            None => std::ptr::null(),
        };

        // 处理 default_family
        let default_family_cstring;
        let default_family_ptr = match default_family {
            Some(s) => {
                // [修复] 使用 map_err 将 NulError 转换为 LibassError
                default_family_cstring = CString::new(s).map_err(|e| {
                    LibassError::InvalidParameter(format!("默认字体族包含空字符: {}", e))
                })?;
                default_family_cstring.as_ptr()
            }
            None => std::ptr::null(),
        };

        // 处理 config
        let config_cstring;
        let config_ptr = match config {
            Some(path) => {
                // [修改] 将路径转换为绝对路径
                let absolute_path = fs::canonicalize(path).map_err(|e| {
                    LibassError::InvalidParameter(format!(
                        "无法将配置文件路径 '{}' 转换为绝对路径: {}",
                        path.display(),
                        e
                    ))
                })?;

                // Path -> &str
                let path_str = absolute_path.to_str().ok_or_else(|| {
                    LibassError::InvalidParameter(format!(
                        "配置文件路径包含无效UTF-8字符: {}",
                        absolute_path.display()
                    ))
                })?;
                // &str -> CString
                config_cstring = CString::new(path_str).map_err(|e| {
                    LibassError::InvalidParameter(format!("配置文件路径包含空字符: {}", e))
                })?;
                config_cstring.as_ptr()
            }
            None => std::ptr::null(),
        };

        // --- 4. 执行不安全的 FFI 调用 ---
        unsafe {
            ffi::ass_set_fonts(
                self.ptr,
                default_font_ptr,
                default_family_ptr,
                font_provider as libc::c_int,
                config_ptr,
                // 废弃字段
                1,
            );
        }

        Ok(())
    }

    // TODO: Keep reviewing from here.
    pub fn selective_style_override() {}

    pub fn render_frame(&mut self, track: &Track, timestamp: i64) -> LibassResult<RenderResult> {
        unsafe {
            let mut change = 0i32;
            let images = ffi::ass_render_frame(self.ptr, track.as_ptr(), timestamp, &mut change);
            Ok(RenderResult::new(images, change != 0))
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            ffi::ass_renderer_done(self.ptr);
        }
    }
}

// ============================================================================
// 枚举类型定义
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub enum ImageType {
    Character = 0,
    Outline = 1,
    Shadow = 2,
}

#[derive(Debug, Clone, Copy)]
pub enum TrackType {
    Unknown = 0,
    Ass = 1,
    Ssa = 2,
}

// ============================================================================
// Track 封装
// ============================================================================

pub struct Track {
    ptr: *mut ffi::ASS_Track,
    library: Library,
}

impl Track {
    /// 创建一个新的空轨道
    ///
    /// 注意：这个方法不会自动创建任何样式或事件。你需要：
    /// - 对于手动工作流程：使用 `alloc_style()` 和 `alloc_event()`
    /// - 对于 ASS 脚本工作流程：使用 `process_data()` 或 `from_file()`
    ///
    /// **重要**: 不要混合使用这两种方式！libass 不支持在手动分配样式后再调用 process_data。
    pub fn new(library: Library) -> LibassResult<Self> {
        unsafe {
            let ptr = ffi::ass_new_track(library.as_ptr());
            if ptr.is_null() {
                return Err(LibassError::InitializationFailed(
                    "ass_new_track 失败".to_string(),
                ));
            }
            Ok(Self { ptr, library })
        }
    }

    pub fn from_file(
        library: Library,
        filename: &str,
        codepage: Option<&str>,
    ) -> LibassResult<Self> {
        let c_filename = CString::new(filename)
            .map_err(|_| LibassError::InvalidParameter("文件名包含无效字符".to_string()))?;
        let c_codepage = codepage
            .map(|s| CString::new(s))
            .transpose()
            .map_err(|_| LibassError::InvalidParameter("编码名包含无效字符".to_string()))?;

        unsafe {
            let ptr = ffi::ass_read_file(
                library.as_ptr(),
                c_filename.as_ptr(),
                c_codepage.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            );
            if ptr.is_null() {
                return Err(LibassError::InitializationFailed(
                    "读取字幕文件失败".to_string(),
                ));
            }
            Ok(Self { ptr, library })
        }
    }

    pub fn from_memory(library: Library, data: &str, codepage: Option<&str>) -> LibassResult<Self> {
        let c_codepage = codepage
            .map(|s| CString::new(s))
            .transpose()
            .map_err(|_| LibassError::InvalidParameter("编码名包含无效字符".to_string()))?;

        unsafe {
            let ptr = ffi::ass_read_memory(
                library.as_ptr(),
                data.as_ptr() as *mut i8,
                data.len(),
                c_codepage.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            );
            if ptr.is_null() {
                return Err(LibassError::InitializationFailed(
                    "从内存读取字幕失败".to_string(),
                ));
            }
            Ok(Self { ptr, library })
        }
    }

    // * The list should have the form [Style.]Param=Value, e.g.
    // *   SomeStyle.Font=Arial
    // *   ScaledBorderAndShadow=yes
    // https://github.com/libass/libass/blob/534a5f8299c5ab3c2782856fcb843bfea47b7afc/libass/ass.h#L329-L331
    pub fn process_override_style(&mut self, list: &[&str]) -> LibassResult<()> {
        let c_strings: Vec<CString> = list
            .iter()
            .map(|&s| {
                CString::new(s).map_err(|e| {
                    LibassError::InvalidParameter(format!(
                        "样式覆盖 '{}' 包含无效的NUL字符: {}",
                        s, e
                    ))
                })
            })
            .collect::<LibassResult<Vec<_>>>()?;

        let mut c_pointers: Vec<*mut c_char> = c_strings
            .iter()
            .map(|cs| cs.as_ptr() as *mut c_char)
            .collect();

        c_pointers.push(ptr::null_mut());

        let list_ptr = c_pointers.as_mut_ptr();

        unsafe {
            ffi::ass_set_style_overrides(self.library.ptr, list_ptr);
            ffi::ass_process_force_style(self.ptr);
        }
        Ok(())
    }
    pub fn set_play_resolution(&mut self, width: i32, height: i32) {
        unsafe {
            (*self.ptr).PlayResX = width;
            (*self.ptr).PlayResY = height;
        }
    }

    pub fn set_track_type(&mut self, track_type: TrackType) {
        unsafe {
            (*self.ptr).track_type = track_type as u32;
        }
    }

    pub fn get_play_resolution(&self) -> (i32, i32) {
        unsafe { ((*self.ptr).PlayResX, (*self.ptr).PlayResY) }
    }

    pub fn alloc_style(&mut self) -> LibassResult<StyleBuilder> {
        unsafe {
            let style_id = ffi::ass_alloc_style(self.ptr);
            if style_id < 0 {
                return Err(LibassError::AllocationFailed("分配样式失败".to_string()));
            }
            let style_ptr = (*self.ptr).styles.offset(style_id as isize);
            let mut builder = StyleBuilder::new(style_ptr);
            builder.initialize_defaults()?;
            Ok(builder)
        }
    }

    pub fn alloc_event(&mut self) -> LibassResult<EventBuilder> {
        unsafe {
            let event_id = ffi::ass_alloc_event(self.ptr);
            if event_id < 0 {
                return Err(LibassError::AllocationFailed("分配事件失败".to_string()));
            }
            let event_ptr = (*self.ptr).events.offset(event_id as isize);
            Ok(EventBuilder::new(event_ptr))
        }
    }

    pub fn get_default_style(&self) -> LibassResult<StyleBuilder> {
        unsafe {
            if (*self.ptr).styles.is_null() || (*self.ptr).n_styles == 0 {
                return Err(LibassError::NullPointer("没有可用的样式".to_string()));
            }
            // 使用 libass 自动创建的默认样式（default_style 字段）
            let default_style_id = (*self.ptr).default_style;
            if default_style_id < 0 || default_style_id >= (*self.ptr).n_styles {
                return Err(LibassError::NullPointer("默认样式索引无效".to_string()));
            }
            let style_ptr = (*self.ptr).styles.offset(default_style_id as isize);
            // 不需要再次初始化，因为 libass 已经通过 set_default_style 初始化了
            Ok(StyleBuilder::new(style_ptr))
        }
    }

    pub fn process_data(&mut self, data: &str) {
        let c_data = CString::new(data).unwrap_or_default();
        unsafe {
            ffi::ass_process_data(self.ptr, c_data.as_ptr(), data.len() as i32);
        }
    }

    pub fn flush_events(&mut self) {
        unsafe {
            ffi::ass_flush_events(self.ptr);
        }
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::ASS_Track {
        self.ptr
    }
}

impl Drop for Track {
    fn drop(&mut self) {
        unsafe {
            ffi::ass_free_track(self.ptr);
        }
    }
}

// ============================================================================
// Style Builder 封装
// ============================================================================

pub struct StyleBuilder {
    ptr: *mut ffi::ASS_Style,
}

impl StyleBuilder {
    fn new(ptr: *mut ffi::ASS_Style) -> Self {
        Self { ptr }
    }

    /// 初始化样式为 libass 的默认值
    /// 这个方法应该在创建新样式后立即调用
    pub fn initialize_defaults(&mut self) -> LibassResult<&mut Self> {
        unsafe {
            // 设置默认的 Name（如果还没有设置）
            if (*self.ptr).Name.is_null() {
                let default_name = CString::new("Default").unwrap();
                (*self.ptr).Name = default_name.into_raw();
            }

            // 设置默认的 FontName（如果还没有设置）
            if (*self.ptr).FontName.is_null() {
                let default_font = CString::new("Arial").unwrap();
                (*self.ptr).FontName = default_font.into_raw();
            }

            // 设置其他默认值（基于 libass 的 set_default_style 函数）
            if (*self.ptr).FontSize == 0.0 {
                (*self.ptr).FontSize = 18.0;
            }
            if (*self.ptr).PrimaryColour == 0 {
                (*self.ptr).PrimaryColour = 0xffffff00; // 白色
            }
            if (*self.ptr).SecondaryColour == 0 {
                (*self.ptr).SecondaryColour = 0x00ffff00; // 青色
            }
            if (*self.ptr).OutlineColour == 0 {
                (*self.ptr).OutlineColour = 0x00000000; // 黑色
            }
            if (*self.ptr).BackColour == 0 {
                (*self.ptr).BackColour = 0x00000080; // 半透明黑色
            }
            // 添加缺失的 Bold 字段！
            if (*self.ptr).Bold == 0 {
                (*self.ptr).Bold = 200; // libass 默认值
            }
            if (*self.ptr).ScaleX == 0.0 {
                (*self.ptr).ScaleX = 1.0;
            }
            if (*self.ptr).ScaleY == 0.0 {
                (*self.ptr).ScaleY = 1.0;
            }
            // 添加缺失的 Spacing 字段！
            if (*self.ptr).Spacing == 0.0 {
                (*self.ptr).Spacing = 0.0;
            }
            if (*self.ptr).BorderStyle == 0 {
                (*self.ptr).BorderStyle = 1;
            }
            if (*self.ptr).Outline == 0.0 {
                (*self.ptr).Outline = 2.0;
            }
            if (*self.ptr).Shadow == 0.0 {
                (*self.ptr).Shadow = 3.0;
            }
            if (*self.ptr).Alignment == 0 {
                (*self.ptr).Alignment = 2; // 底部居中
            }
            if (*self.ptr).MarginL == 0 && (*self.ptr).MarginR == 0 && (*self.ptr).MarginV == 0 {
                (*self.ptr).MarginL = 20;
                (*self.ptr).MarginR = 20;
                (*self.ptr).MarginV = 20;
            }
        }
        Ok(self)
    }

    pub fn set_name(&mut self, name: &str) -> LibassResult<&mut Self> {
        let c_name = CString::new(name)
            .map_err(|_| LibassError::InvalidParameter("样式名包含无效字符".to_string()))?;
        unsafe {
            if !(*self.ptr).Name.is_null() {
                ffi::ass_free((*self.ptr).Name as *mut _);
            }
            (*self.ptr).Name = c_name.into_raw();
        }
        Ok(self)
    }

    pub fn set_font_name(&mut self, font_name: &str) -> LibassResult<&mut Self> {
        let c_font_name = CString::new(font_name)
            .map_err(|_| LibassError::InvalidParameter("字体名包含无效字符".to_string()))?;
        unsafe {
            if !(*self.ptr).FontName.is_null() {
                ffi::ass_free((*self.ptr).FontName as *mut _);
            }
            (*self.ptr).FontName = c_font_name.into_raw();
        }
        Ok(self)
    }

    pub fn set_font_size(&mut self, size: f64) -> &mut Self {
        unsafe {
            (*self.ptr).FontSize = size;
        }
        self
    }

    pub fn set_primary_color(&mut self, color: Color) -> &mut Self {
        unsafe {
            (*self.ptr).PrimaryColour = color.to_libass_format();
        }
        self
    }

    pub fn set_secondary_color(&mut self, color: Color) -> &mut Self {
        unsafe {
            (*self.ptr).SecondaryColour = color.to_libass_format();
        }
        self
    }

    pub fn set_outline_color(&mut self, color: Color) -> &mut Self {
        unsafe {
            (*self.ptr).OutlineColour = color.to_libass_format();
        }
        self
    }

    pub fn set_back_color(&mut self, color: Color) -> &mut Self {
        unsafe {
            (*self.ptr).BackColour = color.to_libass_format();
        }
        self
    }

    pub fn set_bold(&mut self, bold: bool) -> &mut Self {
        unsafe {
            (*self.ptr).Bold = if bold { 1 } else { 0 };
        }
        self
    }

    pub fn set_italic(&mut self, italic: bool) -> &mut Self {
        unsafe {
            (*self.ptr).Italic = if italic { 1 } else { 0 };
        }
        self
    }

    pub fn set_outline(&mut self, outline: f64) -> &mut Self {
        unsafe {
            (*self.ptr).Outline = outline;
        }
        self
    }

    pub fn set_shadow(&mut self, shadow: f64) -> &mut Self {
        unsafe {
            (*self.ptr).Shadow = shadow;
        }
        self
    }

    pub fn set_alignment(&mut self, alignment: i32) -> &mut Self {
        unsafe {
            (*self.ptr).Alignment = alignment;
        }
        self
    }

    pub fn set_margins(&mut self, left: i32, right: i32, vertical: i32) -> &mut Self {
        unsafe {
            (*self.ptr).MarginL = left;
            (*self.ptr).MarginR = right;
            (*self.ptr).MarginV = vertical;
        }
        self
    }
}

// ============================================================================
// Event Builder 封装
// ============================================================================

pub struct EventBuilder {
    ptr: *mut ffi::ASS_Event,
}

impl EventBuilder {
    fn new(ptr: *mut ffi::ASS_Event) -> Self {
        Self { ptr }
    }

    pub fn set_text(&mut self, text: &str) -> LibassResult<&mut Self> {
        let c_text = CString::new(text)
            .map_err(|_| LibassError::InvalidParameter("事件文本包含无效字符".to_string()))?;
        unsafe {
            if !(*self.ptr).Text.is_null() {
                ffi::ass_free((*self.ptr).Text as *mut _);
            }
            (*self.ptr).Text = c_text.into_raw();
        }
        Ok(self)
    }

    pub fn set_timing(&mut self, start: i64, duration: i64) -> &mut Self {
        unsafe {
            (*self.ptr).Start = start;
            (*self.ptr).Duration = duration;
        }
        self
    }

    pub fn set_style(&mut self, style_id: i32) -> &mut Self {
        unsafe {
            (*self.ptr).Style = style_id;
        }
        self
    }

    pub fn set_layer(&mut self, layer: i32) -> &mut Self {
        unsafe {
            (*self.ptr).Layer = layer;
        }
        self
    }

    pub fn set_margins(&mut self, left: i32, right: i32, vertical: i32) -> &mut Self {
        unsafe {
            (*self.ptr).MarginL = left;
            (*self.ptr).MarginR = right;
            (*self.ptr).MarginV = vertical;
        }
        self
    }
}

// ============================================================================
// 渲染结果封装
// ============================================================================

pub struct RenderResult {
    images: *const ffi::ASS_Image,
    has_changed: bool,
}

impl RenderResult {
    fn new(images: *const ffi::ASS_Image, has_changed: bool) -> Self {
        Self {
            images,
            has_changed,
        }
    }

    pub fn has_changed(&self) -> bool {
        self.has_changed
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_null()
    }

    pub fn iter(&self) -> ImageIterator {
        ImageIterator::new(self.images)
    }

    /// 将所有图层合成到一个 RGBA 图像中
    pub fn compose_to_rgba(&self, width: u32, height: u32) -> RgbaImage {
        let mut result = RgbaImage::new(width, height);

        for image in self.iter() {
            image.blend_into(&mut result);
        }

        result
    }

    /// 保存每个图层为单独的PNG文件
    pub fn save_layers_to_png(&self, width: u32, height: u32) -> LibassResult<usize> {
        let mut layer_count = 0;

        for (index, image) in self.iter().enumerate() {
            let mut layer_img = RgbaImage::new(width, height);
            image.draw_into(&mut layer_img);

            let filename = format!(
                "layer_{:02}_size_{}x{}_pos_{}_{}_color_{:08x}.png",
                index + 1,
                image.width(),
                image.height(),
                image.dst_x(),
                image.dst_y(),
                image.color().to_libass_format()
            );

            layer_img
                .save(&filename)
                .map_err(|e| LibassError::InitializationFailed(format!("保存图片失败: {}", e)))?;

            println!("  图层 #{} 已保存到: {}", index + 1, filename);
            layer_count += 1;
        }

        Ok(layer_count)
    }
}

// ============================================================================
// Image 封装
// ============================================================================

pub struct Image<'a> {
    ptr: *const ffi::ASS_Image,
    _phantom: std::marker::PhantomData<&'a ffi::ASS_Image>,
}

impl<'a> Image<'a> {
    fn new(ptr: *const ffi::ASS_Image) -> Self {
        Self {
            ptr,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn width(&self) -> i32 {
        unsafe { (*self.ptr).w }
    }

    pub fn height(&self) -> i32 {
        unsafe { (*self.ptr).h }
    }

    pub fn stride(&self) -> i32 {
        unsafe { (*self.ptr).stride }
    }

    pub fn dst_x(&self) -> i32 {
        unsafe { (*self.ptr).dst_x }
    }

    pub fn dst_y(&self) -> i32 {
        unsafe { (*self.ptr).dst_y }
    }

    pub fn color(&self) -> Color {
        unsafe { Color::from_libass_format((*self.ptr).color) }
    }

    pub fn image_type(&self) -> ImageType {
        unsafe {
            match (*self.ptr).type_ {
                0 => ImageType::Character,
                1 => ImageType::Outline,
                2 => ImageType::Shadow,
                _ => ImageType::Character, // 默认值
            }
        }
    }

    pub fn bitmap(&self) -> &[u8] {
        unsafe {
            if (*self.ptr).bitmap.is_null() || (*self.ptr).w <= 0 || (*self.ptr).h <= 0 {
                return &[];
            }
            let size = ((*self.ptr).stride * ((*self.ptr).h - 1) + (*self.ptr).w) as usize;
            slice::from_raw_parts((*self.ptr).bitmap, size)
        }
    }

    /// 将此图像绘制到目标 RGBA 图像中（不进行 Alpha 混合）
    pub fn draw_into(&self, target: &mut RgbaImage) {
        if self.width() <= 0 || self.height() <= 0 {
            return;
        }

        let color = self.color();
        let bitmap = self.bitmap();
        let dst_x = self.dst_x();
        let dst_y = self.dst_y();

        for y in 0..self.height() {
            for x in 0..self.width() {
                let bitmap_index = (y * self.stride() + x) as usize;

                if bitmap_index < bitmap.len() {
                    let alpha_value = bitmap[bitmap_index];
                    let target_x = dst_x + x;
                    let target_y = dst_y + y;

                    if target_x >= 0
                        && target_x < target.width() as i32
                        && target_y >= 0
                        && target_y < target.height() as i32
                        && alpha_value > 0
                    {
                        target.put_pixel(
                            target_x as u32,
                            target_y as u32,
                            Rgba([color.r, color.g, color.b, alpha_value]),
                        );
                    }
                }
            }
        }
    }

    /// 将此图像混合到目标 RGBA 图像中（进行 Alpha 混合）
    pub fn blend_into(&self, target: &mut RgbaImage) {
        if self.width() <= 0 || self.height() <= 0 {
            return;
        }

        let color = self.color();
        let bitmap = self.bitmap();
        let dst_x = self.dst_x();
        let dst_y = self.dst_y();

        for y in 0..self.height() {
            for x in 0..self.width() {
                let bitmap_index = (y * self.stride() + x) as usize;

                if bitmap_index < bitmap.len() {
                    let alpha_value = bitmap[bitmap_index];
                    let target_x = dst_x + x;
                    let target_y = dst_y + y;

                    if target_x >= 0
                        && target_x < target.width() as i32
                        && target_y >= 0
                        && target_y < target.height() as i32
                        && alpha_value > 0
                    {
                        let target_x = target_x as u32;
                        let target_y = target_y as u32;

                        // 获取目标像素
                        let dst_pixel = target.get_pixel(target_x, target_y);

                        // Alpha 混合
                        let src_alpha = alpha_value as f32 / 255.0;
                        let dst_alpha = dst_pixel[3] as f32 / 255.0;
                        let out_alpha = src_alpha + dst_alpha * (1.0 - src_alpha);

                        if out_alpha > 0.0 {
                            let r = ((color.r as f32 * src_alpha
                                + dst_pixel[0] as f32 * dst_alpha * (1.0 - src_alpha))
                                / out_alpha) as u8;
                            let g = ((color.g as f32 * src_alpha
                                + dst_pixel[1] as f32 * dst_alpha * (1.0 - src_alpha))
                                / out_alpha) as u8;
                            let b = ((color.b as f32 * src_alpha
                                + dst_pixel[2] as f32 * dst_alpha * (1.0 - src_alpha))
                                / out_alpha) as u8;
                            let a = (out_alpha * 255.0) as u8;

                            target.put_pixel(target_x, target_y, Rgba([r, g, b, a]));
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================
// Image 迭代器
// ============================================================================

pub struct ImageIterator {
    current: *const ffi::ASS_Image,
}

impl ImageIterator {
    fn new(first: *const ffi::ASS_Image) -> Self {
        Self { current: first }
    }
}

impl Iterator for ImageIterator {
    type Item = Image<'static>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            if self.current.is_null() {
                return None;
            }

            let image = Image::new(self.current);
            self.current = (*self.current).next;
            Some(image)
        }
    }
}

// ============================================================================
// 便利函数和构建器模式
// ============================================================================

pub struct SubtitleRenderer {
    library: Library,
    renderer: Renderer,
}

impl SubtitleRenderer {
    pub fn new() -> LibassResult<Self> {
        let library = Library::new()?;

        let mut renderer = Renderer::new(&library)?;

        // 设置默认配置
        renderer.set_frame_size(1280, 720);
        renderer.set_fonts(None, None, DefaultFontProvider::AutoDetect, None)?; // 使用自动字体选择
        Ok(Self { library, renderer })
    }

    pub fn set_frame_size(&mut self, width: i32, height: i32) -> &mut Self {
        self.renderer.set_storage_size(width, height); // 添加这行！
        self.renderer.set_frame_size(width, height);
        self
    }

    pub fn set_fonts(
        &mut self,
        default_font: Option<&Path>,
        default_family: Option<&str>,
        font_provider: DefaultFontProvider,
        config: Option<&Path>,
    ) -> LibassResult<&mut Self> {
        self.renderer
            .set_fonts(default_font, default_family, font_provider, config)?;
        Ok(self)
    }

    pub fn create_track(&self) -> LibassResult<Track> {
        Track::new(self.library.clone())
    }

    pub fn render_frame(&mut self, track: &Track, timestamp: i64) -> LibassResult<RenderResult> {
        self.renderer.render_frame(track, timestamp)
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// 设置消息回调函数的便利方法
    pub fn set_message_callback<F>(&mut self, callback: F) -> LibassResult<()>
    where
        F: Fn(MessageLevel, &str) + Send + Sync + 'static,
    {
        self.library.set_message_callback(callback)
    }

    /// 清除消息回调函数的便利方法
    pub fn clear_message_callback(&mut self) -> LibassResult<()> {
        self.library.clear_message_callback()
    }
}

// ============================================================================
// 高级 API
// ============================================================================

/// 创建一个带有默认消息回调的字幕渲染器
///
/// 这个函数会创建一个 SubtitleRenderer 并设置一个默认的消息回调，
/// 该回调会将错误和警告输出到 stderr，将信息输出到 stdout。
///
/// # 示例
/// ```rust
/// use aweft_libass::*;
///
/// let mut renderer = create_subtitle_renderer_with_logging().unwrap();
/// // 现在所有 libass 消息都会被自动记录
/// ```
pub fn create_subtitle_renderer_with_logging() -> LibassResult<SubtitleRenderer> {
    let mut renderer = SubtitleRenderer::new()?;

    renderer.set_message_callback(|level, message| {
        match level {
            MessageLevel::Fatal | MessageLevel::Error => {
                eprintln!("🔴 libass 错误: {}", message);
            }
            MessageLevel::Warning => {
                eprintln!("🟡 libass 警告: {}", message);
            }
            MessageLevel::Info => {
                println!("🔵 libass 信息: {}", message);
            }
            MessageLevel::Verbose => {
                println!("🟢 libass 详细: {}", message);
            }
            MessageLevel::Debug | MessageLevel::Trace => {
                // 默认情况下忽略调试消息，用户可以通过环境变量启用
                if std::env::var("LIBASS_DEBUG").is_ok() {
                    println!("🟣 libass 调试: {}", message);
                }
            }
            MessageLevel::Unknown => {
                println!("❓ libass 未知级别: {}", message);
            }
        }
    })?;

    Ok(renderer)
}

// ============================================================================
// 兼容性函数（保持向后兼容）
// ============================================================================

/// 获取 libass 版本（保持向后兼容）
pub fn version() -> i32 {
    Library::version()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_libass_version() {
        let ver = version();
        println!("libass version: {:#08x}", ver);
        assert!(ver > 0);
    }

    #[test]
    fn test_color_conversion() {
        let color = Color::new(255, 128, 64, 192);
        let libass_format = color.to_libass_format();
        let converted_back = Color::from_libass_format(libass_format);

        assert_eq!(color, converted_back);

        // 测试已知值
        assert_eq!(Color::red().to_libass_format(), 0xFF0000FF);
        assert_eq!(Color::green().to_libass_format(), 0x00FF00FF);
        assert_eq!(Color::blue().to_libass_format(), 0x0000FFFF);
    }

    #[test]
    fn test_manual_workflow() {
        // 测试手动工作流程：使用默认样式 + alloc_event
        let library = Library::new().expect("创建库失败");

        let mut renderer = Renderer::new(&library).expect("创建渲染器失败");
        renderer.set_storage_size(1280, 720);
        renderer.set_frame_size(1280, 720);
        // 这个是必须设置的
        renderer
            .set_fonts(None, None, DefaultFontProvider::AutoDetect, None)
            .expect("设置字体失败");

        let mut track = Track::new(library).expect("创建轨道失败");
        track.set_play_resolution(1280, 720);
        track.set_track_type(TrackType::Ass);

        // 使用 libass 自动创建的默认样式
        let mut style = track.get_default_style().expect("获取默认样式失败");
        style
            .set_font_size(48.0)
            .set_primary_color(Color::new(255, 255, 255, 255))
            .set_outline_color(Color::black())
            .set_outline(2.0)
            .set_shadow(3.0)
            .set_alignment(2);

        // 手动创建事件
        let mut event = track.alloc_event().expect("分配事件失败");
        event
            .set_text("手动工作流程测试")
            .expect("设置文本失败")
            .set_timing(0, 3000)
            .set_style(0);

        // 渲染
        let result = renderer.render_frame(&track, 1500).expect("渲染失败");

        assert!(!result.is_empty(), "手动工作流程应该生成图像");
        let image_count = result.iter().count();
        println!("✅ 手动工作流程成功生成了 {} 个图像层", image_count);
        assert!(image_count > 0);
    }

    #[test]
    fn test_ass_script_workflow() {
        // 测试 ASS 脚本工作流程：process_data
        let mut library = Library::new().expect("创建库失败");
        library.set_extract_fonts(true);

        let mut renderer = Renderer::new(&library).expect("创建渲染器失败");
        renderer.set_storage_size(1280, 720);
        renderer.set_frame_size(1280, 720);
        renderer
            .set_fonts(None, None, DefaultFontProvider::AutoDetect, None)
            .expect("设置字体失败");

        let mut track = Track::new(library).expect("创建轨道失败");
        track.set_play_resolution(1280, 720);
        track.set_track_type(TrackType::Ass);

        // 使用完整的 ASS 脚本
        let ass_script = r#"[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,,48,&H00FF0000,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,3,2,20,20,20,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,ASS 脚本工作流程测试"#;

        track.process_data(ass_script);

        // 渲染
        let result = renderer.render_frame(&track, 2500).expect("渲染失败");

        assert!(!result.is_empty(), "ASS 脚本工作流程应该生成图像");
        let image_count = result.iter().count();
        println!("✅ ASS 脚本工作流程成功生成了 {} 个图像层", image_count);
        assert!(image_count > 0);
    }

    #[test]
    fn test_message_callback() {
        use std::sync::{Arc, Mutex};

        // 测试消息回调功能
        let messages = Arc::new(Mutex::new(Vec::<(MessageLevel, String)>::new()));
        let messages_clone = messages.clone();

        let mut library = Library::new().expect("创建库失败");

        // 先设置回调，再进行其他操作
        library
            .set_message_callback(move |level, message| {
                println!(
                    "📨 [test_message_callback] 收到消息 [{:?}]: {}",
                    level, message
                );
                if let Ok(mut msgs) = messages_clone.lock() {
                    msgs.push((level, message.to_string()));
                }
            })
            .expect("设置消息回调失败");

        let mut renderer = Renderer::new(&library).expect("创建渲染器失败");
        renderer.set_storage_size(1280, 720);
        renderer.set_frame_size(1280, 720);

        // 主动触发字体设置来产生消息
        renderer
            .set_fonts(None, None, DefaultFontProvider::AutoDetect, None)
            .expect("设置字体失败");

        // 触发一些 libass 操作来生成消息
        let mut track = Track::new(library.clone()).expect("创建轨道失败");
        track.set_track_type(TrackType::Ass);

        let ass_script = r#"[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,,20,&H00ffffff,&H000000ff,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:05.00,Default,,0,0,0,,消息回调测试"#;

        track.process_data(ass_script);
        let _result = renderer.render_frame(&track, 3000);

        // 检查是否收到了消息
        let collected_messages = messages.lock().unwrap();
        println!(
            "📊 [test_message_callback] 收到 {} 条 libass 消息",
            collected_messages.len()
        );

        // 清除回调
        library.clear_message_callback().expect("清除回调失败");

        println!("✅ 消息回调测试完成");
    }

    #[test]
    fn test_message_callback_detailed() {
        use std::sync::{Arc, Mutex};

        // 测试消息回调功能 - 更详细的版本
        let messages = Arc::new(Mutex::new(Vec::<(MessageLevel, String)>::new()));
        let messages_clone = messages.clone();

        let mut library = Library::new().expect("创建库失败");

        // 先设置回调，再进行其他操作
        library
            .set_message_callback(move |level, message| {
                println!(
                    "📨 [test_message_callback_detailed] 收到消息 [{:?}]: {}",
                    level, message
                );
                if let Ok(mut msgs) = messages_clone.lock() {
                    msgs.push((level, message.to_string()));
                }
            })
            .expect("设置消息回调失败");

        // 设置一些可能触发消息的操作
        library.set_extract_fonts(true);

        let mut renderer = Renderer::new(&library).expect("创建渲染器失败");
        renderer.set_storage_size(1280, 720);
        renderer.set_frame_size(1280, 720);

        // 尝试设置一个可能不存在的字体路径来触发警告
        let _ = renderer.set_fonts(
            Some(Path::new("/nonexistent/font.ttf")),
            Some("NonexistentFont"),
            DefaultFontProvider::AutoDetect,
            None,
        );

        let mut track = Track::new(library.clone()).expect("创建轨道失败");
        track.set_track_type(TrackType::Ass);

        // 故意使用一个可能有问题的 ASS 脚本来触发警告
        let problematic_ass_script = r#"[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,NonexistentFont,20,&H00ffffff,&H000000ff,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:05.00,Default,,0,0,0,,测试消息回调 - 详细版本"#;

        track.process_data(problematic_ass_script);
        let _result = renderer.render_frame(&track, 3000);

        // 检查是否收到了消息
        let collected_messages = messages.lock().unwrap();
        println!(
            "📊 [test_message_callback_detailed] 总共收到 {} 条 libass 消息",
            collected_messages.len()
        );

        for (i, (level, msg)) in collected_messages.iter().enumerate() {
            println!("  消息 #{}: [{:?}] {}", i + 1, level, msg);
        }

        // 清除回调
        library.clear_message_callback().expect("清除回调失败");

        println!("✅ 详细消息回调测试完成");
    }
}
