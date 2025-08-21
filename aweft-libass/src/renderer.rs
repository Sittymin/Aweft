use crate::ffi;
use crate::library::Library;
use crate::track::Track;
use crate::types::ImageIterator;
use anyhow::{Result, anyhow};
use bitflags::bitflags;
use libc::{c_char, c_double, c_int};
use std::ffi::{CStr, CString};
use std::fs;
use std::path::Path;
use std::sync::Arc;

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
    pub fn to_raw_guard(&self) -> Result<AssStyleGuard> {
        // WARN: 不应该使用 CString 如果 C 侧会修改的话
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
    type Error = anyhow::Error;

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
    _library: Arc<Library>,
    ptr: *mut ffi::ASS_Renderer,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(C)]
    pub struct OverrideBits: u32 {
        /**
         * 默认模式 (值为 0)。
         * 所有选择性覆盖功能都将被禁用，但像 ass_set_font_scale() 这样的传统覆盖会无条件应用。
         */
        const DEFAULT = 0;

        /**
         * 对看起来像对话的事件应用通过 ass_set_selective_style_override() 设置的样式。
         * 其他样式覆盖也以此方式应用，除了字体缩放。
         *
         * 这等效于设置以下所有标志：
         * - `FONT_NAME`
         * - `FONT_SIZE_FIELDS`
         * - `COLORS`
         * - `BORDER`
         * - `ATTRIBUTES`
         */
        const STYLE = 1 << 0;

        /**
         * 仅对看起来像对话的事件应用 ass_set_font_scale()。
         * 如果未设置，字体缩放将应用于所有事件。
         */
        const SELECTIVE_FONT_SCALE = 1 << 1;

        /**
         * 在对话事件上覆盖：FontSize, Spacing, ScaleX, ScaleY
         */
        const FONT_SIZE_FIELDS = 1 << 2;

        /**
         * 在对话事件上覆盖：FontName, treat_fontname_as_pattern
         */
        const FONT_NAME = 1 << 3;

        /**
         * 在对话事件上覆盖：PrimaryColour, SecondaryColour, OutlineColour, BackColour
         */
        const COLORS = 1 << 4;

        /**
         * 在对话事件上覆盖：Bold, Italic, Underline, StrikeOut
         */
        const ATTRIBUTES = 1 << 5;

        /**
         * 在对话事件上覆盖：BorderStyle, Outline, Shadow
         */
        const BORDER = 1 << 6;

        /**
         * 在对话事件上覆盖：Alignment
         */
        const ALIGNMENT = 1 << 7;

        /**
         * 在对话事件上覆盖：MarginL, MarginR, MarginV
         */
        const MARGINS = 1 << 8;

        /**
         * 无条件地用 ass_set_selective_style_override() 提供的新样式替换所有样式的全部字段。
         */
        const FULL_STYLE = 1 << 9;

        /**
         * 在对话事件上覆盖：Justify
         */
        const JUSTIFY = 1 << 10;

        /**
         * 在对话事件上覆盖：Blur
         */
        const BLUR = 1 << 11;
    }
}
impl Renderer {
    pub fn new(library: Arc<Library>) -> Result<Self> {
        unsafe {
            let ptr = ffi::ass_renderer_init(library.as_ptr());
            if ptr.is_null() {
                return Err(anyhow!("ass_renderer_init 失败"));
            }
            Ok(Self {
                ptr,
                _library: library,
            })
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
    ) -> Result<()> {
        // 处理 default_font
        let default_font_cstring;
        let default_font_ptr = match default_font {
            Some(path) => {
                // [修改] 将路径转换为绝对路径
                let absolute_path = fs::canonicalize(path).map_err(|e| {
                    anyhow!(
                        "无法将默认字体路径 '{}' 转换为绝对路径: {}",
                        path.display(),
                        e
                    )
                })?;

                // Path -> &str
                let path_str = absolute_path.to_str().ok_or_else(|| {
                    anyhow!("默认字体路径包含无效UTF-8字符: {}", absolute_path.display())
                })?;
                // &str -> CString
                default_font_cstring =
                    CString::new(path_str).map_err(|e| anyhow!("默认字体路径包含空字符: {}", e))?;
                default_font_cstring.as_ptr()
            }
            Option::None => std::ptr::null(),
        };

        // 处理 default_family
        let default_family_cstring;
        let default_family_ptr = match default_family {
            Some(s) => {
                // [修复] 使用 map_err 将 NulError 转换为 anyhow
                default_family_cstring =
                    CString::new(s).map_err(|e| anyhow!("默认字体族包含空字符: {}", e))?;
                default_family_cstring.as_ptr()
            }
            Option::None => std::ptr::null(),
        };

        // 处理 config
        let config_cstring;
        let config_ptr = match config {
            Some(path) => {
                // [修改] 将路径转换为绝对路径
                let absolute_path = fs::canonicalize(path).map_err(|e| {
                    anyhow!(
                        "无法将配置文件路径 '{}' 转换为绝对路径: {}",
                        path.display(),
                        e
                    )
                })?;

                // Path -> &str
                let path_str = absolute_path.to_str().ok_or_else(|| {
                    anyhow!("配置文件路径包含无效UTF-8字符: {}", absolute_path.display())
                })?;
                // &str -> CString
                config_cstring =
                    CString::new(path_str).map_err(|e| anyhow!("配置文件路径包含空字符: {}", e))?;
                config_cstring.as_ptr()
            }
            Option::None => std::ptr::null(),
        };

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

    pub fn selective_style_override(
        &mut self,
        style: AssStyle,
        override_bits: OverrideBits,
    ) -> Result<()> {
        let mut ass_style_guard = style.to_raw_guard()?;
        let style_ptr: *mut ffi::ass_style = &mut ass_style_guard.raw as *mut ffi::ass_style;
        let ffi_bits: ffi::ASS_OverrideBits = override_bits.bits();
        unsafe {
            ffi::ass_set_selective_style_override(self.ptr, style_ptr);
            ffi::ass_set_selective_style_override_enabled(self.ptr, ffi_bits as i32);
        }
        Ok(())
    }

    // 在 libass 中注释说不要使用它，除非是内存受限的地方与调试
    // pub fn set_cache_limits(self, glyph_max: i32, bitmap_max_size: i32){}

    pub fn render_frame(&mut self, track: &Track, timestamp: i64) -> Result<RenderResult> {
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
}
