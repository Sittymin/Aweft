use crate::error::{LibassError, Result};
use crate::ffi;
use crate::image::Image;
use crate::library::Library;
use crate::track::Track;
use crate::types::{DefaultFontProvider, HintingType, OverrideBits, RenderChange, ShapingLevel};
use libc::{c_char, c_double, c_int};
use std::ffi::{CStr, CString};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone)]
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
    type Error = LibassError;

    fn try_from(raw_style_ptr: *const ffi::ass_style) -> Result<Self> {
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

impl Renderer {
    pub fn new(library: Arc<Library>) -> Result<Self> {
        unsafe {
            let ptr = ffi::ass_renderer_init(library.as_ptr());
            if ptr.is_null() {
                return Err(LibassError::RendererInit {
                    context: "ass_renderer_init 返回空指针".to_string(),
                });
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

    // 使用 ShapingLevel::Complex 时会有更多高级功能
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
        default_font: Option<&Path>,  // 文件兜底（最后）
        default_family: Option<&str>, // 族名回退（先查）
        font_provider: DefaultFontProvider,
        fontconfig_config: Option<&Path>,
    ) -> Result<()> {
        let default_font_cstring;
        let default_font_ptr = match default_font {
            Some(path) => {
                #[cfg(unix)]
                {
                    default_font_cstring = CString::new(path.as_os_str().as_bytes())?;
                }
                #[cfg(not(unix))]
                {
                    let s = path.to_str().ok_or_else(|| LibassError::InvalidParameter {
                        context: format!("默认字体路径包含无效UTF-8: {}", path.display()),
                    })?;
                    default_font_cstring = CString::new(s)?;
                }
                default_font_cstring.as_ptr()
            }
            Option::None => std::ptr::null(),
        };

        // 处理 default_family
        let default_family_cstring;
        let default_family_ptr = match default_family {
            Some(s) => {
                default_family_cstring = CString::new(s)?;
                default_family_cstring.as_ptr()
            }
            Option::None => std::ptr::null(),
        };

        // 处理 config
        let config_cstring;
        let config_ptr = match fontconfig_config {
            Some(path) => {
                #[cfg(unix)]
                {
                    config_cstring = CString::new(path.as_os_str().as_bytes())?;
                }
                #[cfg(not(unix))]
                {
                    let s = path.to_str().ok_or_else(|| LibassError::InvalidParameter {
                        context: format!("配置文件路径包含无效UTF-8: {}", path.display()),
                    })?;
                    config_cstring = CString::new(s)?;
                }
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

    /// Configure libass glyph/bitmap cache limits.
    pub fn set_cache_limits(&mut self, glyph_max: usize, bitmap_max_size: usize) -> Result<()> {
        let glyph_max = i32::try_from(glyph_max).map_err(|_| LibassError::InvalidParameter {
            context: "glyph cache limit exceeds libass range".into(),
        })?;
        let bitmap_max_size =
            i32::try_from(bitmap_max_size).map_err(|_| LibassError::InvalidParameter {
                context: "bitmap cache limit exceeds libass range".into(),
            })?;
        unsafe {
            ffi::ass_set_cache_limits(self.ptr, glyph_max, bitmap_max_size);
        }
        Ok(())
    }

    pub fn render_frame(&mut self, track: &mut Track, timestamp: i64) -> Result<RenderResult> {
        unsafe {
            let mut change = 0i32;
            let images = ffi::ass_render_frame(self.ptr, track.as_ptr(), timestamp, &mut change);
            // ASS_Image nodes and bitmaps are owned by libass and invalidated by
            // the next render. Copy them immediately into an owned frame.
            let mut owned = Vec::new();
            let mut image = images;
            while !image.is_null() {
                if let Some(copy) = Image::from_raw(image) {
                    owned.push(copy);
                }
                image = (*image).next;
            }
            track.mark_rendered();
            Ok(RenderResult::new(owned, RenderChange::from_raw(change)))
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
#[derive(Debug, Clone)]
pub struct RenderResult {
    images: Vec<Image>,
    change: RenderChange,
}

impl RenderResult {
    fn new(images: Vec<Image>, change: RenderChange) -> Self {
        Self { images, change }
    }

    pub fn has_changed(&self) -> bool {
        self.change.is_changed()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    pub fn change(&self) -> RenderChange {
        self.change
    }
    pub fn images(&self) -> &[Image] {
        &self.images
    }
}

impl IntoIterator for RenderResult {
    type Item = Image;
    type IntoIter = std::vec::IntoIter<Image>;

    fn into_iter(self) -> Self::IntoIter {
        self.images.into_iter()
    }
}
