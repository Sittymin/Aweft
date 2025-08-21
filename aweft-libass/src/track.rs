use crate::ffi;
use crate::library::Library;
use crate::types::Color;
use anyhow::{Result, anyhow};
use libc::{c_char, c_void, size_t};
use std::ffi::CString;
use std::ptr;
use std::sync::Arc;

use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(transparent)]
    pub struct AssAlignment: i32 {
        const VALIGN_TOP    = 4;
        const VALIGN_CENTER = 8;
        const VALIGN_SUB    = 0;
        const HALIGN_LEFT   = 1;
        const HALIGN_CENTER = 2;
        const HALIGN_RIGHT  = 3;
    }
}

// 定义一个只包含有效组合的枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    BottomLeft,
    BottomCenter,
    BottomRight,
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
}

impl From<Alignment> for AssAlignment {
    fn from(align: Alignment) -> Self {
        match align {
            Alignment::BottomLeft => Self::from_bits_retain(0 | 1),
            Alignment::BottomCenter => Self::from_bits_retain(0 | 2),
            Alignment::BottomRight => Self::from_bits_retain(0 | 3),
            Alignment::TopLeft => Self::from_bits_retain(4 | 1),
            Alignment::TopCenter => Self::from_bits_retain(4 | 2),
            Alignment::TopRight => Self::from_bits_retain(4 | 3),
            Alignment::MiddleLeft => Self::from_bits_retain(8 | 1),
            Alignment::MiddleCenter => Self::from_bits_retain(8 | 2),
            Alignment::MiddleRight => Self::from_bits_retain(8 | 3),
        }
    }
}

impl TryFrom<AssAlignment> for Alignment {
    type Error = &'static str;

    fn try_from(value: AssAlignment) -> Result<Self, Self::Error> {
        // 使用 `bits()` 进行匹配
        match value.bits() {
            1 => Ok(Alignment::BottomLeft),
            2 => Ok(Alignment::BottomCenter),
            3 => Ok(Alignment::BottomRight),
            5 => Ok(Alignment::TopLeft),
            6 => Ok(Alignment::TopCenter),
            7 => Ok(Alignment::TopRight),
            9 => Ok(Alignment::MiddleLeft),
            10 => Ok(Alignment::MiddleCenter),
            11 => Ok(Alignment::MiddleRight),
            _ => Err("Invalid libass alignment value"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Banner,
    ScrollUp,
    ScrollDown,
}

// 2. 实现转换逻辑
impl Effect {
    pub fn as_str(&self) -> &'static str {
        match self {
            Effect::Banner => "Banner;",
            Effect::ScrollUp => "Scroll up;",
            Effect::ScrollDown => "Scroll down;",
        }
    }

    pub fn to_cstring(&self) -> Result<CString> {
        CString::new(self.as_str())
            .map_err(|e| anyhow!("Effect '{}' 包含无效的NUL字符: {}", self.as_str(), e))
    }
}

// ============================================================================
// Track 封装
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub enum TrackType {
    Unknown = 0,
    Ass = 1,
    Ssa = 2,
}

pub struct Track {
    ptr: *mut ffi::ASS_Track,
    library: Arc<Library>,
}

impl Track {
    /// 创建一个新的空轨道
    ///
    /// 注意：这个方法不会自动创建任何样式或事件。你需要：
    /// - 对于手动工作流程：使用 `alloc_style()` 和 `alloc_event()`
    /// - 对于 ASS 脚本工作流程：使用 `process_data()` 或 `from_file()`
    ///
    /// **重要**: 不要混合使用这两种方式！libass 不支持在手动分配样式后再调用 process_data。
    pub fn new(library: Arc<Library>) -> Result<Self> {
        unsafe {
            // 这里已经创建了一个默认样式
            let ptr = ffi::ass_new_track(library.as_ptr());
            if ptr.is_null() {
                return Err(anyhow!("ass_new_track 失败"));
            }
            Ok(Self { ptr, library })
        }
    }

    // pub fn from_file(
    //     library: Library,
    //     filename: &str,
    //     codepage: Option<&str>,
    // ) -> Result<Self> {
    //     let c_filename = CString::new(filename)
    //         .map_err(|_| LibassError::InvalidParameter("文件名包含无效字符".to_string()))?;
    //     let c_codepage = codepage
    //         .map(|s| CString::new(s))
    //         .transpose()
    //         .map_err(|_| LibassError::InvalidParameter("编码名包含无效字符".to_string()))?;

    //     unsafe {
    //         let ptr = ffi::ass_read_file(
    //             library.as_ptr(),
    //             c_filename.as_ptr(),
    //             c_codepage.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
    //         );
    //         if ptr.is_null() {
    //             return Err(LibassError::InitializationFailed(
    //                 "读取字幕文件失败".to_string(),
    //             ));
    //         }
    //         Ok(Self { ptr, library })
    //     }
    // }

    // pub fn from_memory(library: Library, data: &str, codepage: Option<&str>) -> Result<Self> {
    //     let c_codepage = codepage
    //         .map(|s| CString::new(s))
    //         .transpose()
    //         .map_err(|_| LibassError::InvalidParameter("编码名包含无效字符".to_string()))?;

    //     unsafe {
    //         let ptr = ffi::ass_read_memory(
    //             library.as_ptr(),
    //             data.as_ptr() as *mut i8,
    //             data.len(),
    //             c_codepage.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
    //         );
    //         if ptr.is_null() {
    //             return Err(LibassError::InitializationFailed(
    //                 "从内存读取字幕失败".to_string(),
    //             ));
    //         }
    //         Ok(Self { ptr, library })
    //     }
    // }

    // * The list should have the form [Style.]Param=Value, e.g.
    // *   SomeStyle.Font=Arial
    // *   ScaledBorderAndShadow=yes
    // https://github.com/libass/libass/blob/534a5f8299c5ab3c2782856fcb843bfea47b7afc/libass/ass.h#L329-L331
    pub fn process_override_style(&mut self, list: &[&str]) -> Result<()> {
        let c_strings: Vec<CString> = list
            .iter()
            .map(|&s| {
                CString::new(s).map_err(|e| anyhow!("样式覆盖 '{}' 包含无效的NUL字符: {}", s, e))
            })
            .collect::<Result<Vec<_>>>()?;

        let mut c_pointers: Vec<*mut c_char> = c_strings
            .iter()
            .map(|cs| cs.as_ptr() as *mut c_char)
            .collect();

        c_pointers.push(ptr::null_mut());

        let list_ptr = c_pointers.as_mut_ptr();

        unsafe {
            ffi::ass_set_style_overrides(self.library.as_ptr(), list_ptr);
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

    pub fn play_resolution(&self) -> (i32, i32) {
        unsafe { ((*self.ptr).PlayResX, (*self.ptr).PlayResY) }
    }

    // 创建新样式
    pub fn create_style(&mut self, name: &str, font_name: &str) -> Result<StyleHandle> {
        unsafe {
            let style_id = ffi::ass_alloc_style(self.ptr);
            if style_id < 0 {
                return Err(anyhow!("分配样式失败"));
            }

            Ok(StyleHandle {
                track: self,
                id: StyleId(style_id),
            }
            .set_name(name)?
            .set_font_name(font_name)?
            .set_font_size(18.0)
            .set_border_style(1)
            .set_alignment(Alignment::BottomCenter))
        }
    }

    /// 获取样式数量
    pub fn style_count(&self) -> i32 {
        unsafe { (*self.ptr).n_styles }
    }

    /// 迭代所有样式ID
    pub fn style_ids(&self) -> impl Iterator<Item = StyleId> {
        let count = self.style_count();
        (0..count).map(StyleId)
    }

    /// 获取默认样式的可变引用（Rust风格的安全API）
    pub fn default_style_mut(&mut self) -> Result<StyleHandle> {
        unsafe {
            if (*self.ptr).styles.is_null() || (*self.ptr).n_styles == 0 {
                return Err(anyhow!("没有可用的样式"));
            }
            // 使用 libass 自动创建的默认样式（default_style 字段）
            let default_style_id = (*self.ptr).default_style;
            if default_style_id < 0 || default_style_id >= (*self.ptr).n_styles {
                return Err(anyhow!("默认样式索引无效"));
            }
            let id = StyleId(default_style_id);
            Ok(StyleHandle { track: self, id })
        }
    }

    pub fn style_mut(&mut self, id: StyleId) -> Result<StyleHandle> {
        if id.0 < 0 || id.0 >= self.style_count() {
            return Err(anyhow!("无效的样式ID: {}", id.0));
        }
        Ok(StyleHandle { track: self, id })
    }

    /// 获取事件数量  
    pub fn event_count(&self) -> i32 {
        unsafe { (*self.ptr).n_events }
    }

    /// 迭代所有事件ID
    pub fn event_ids(&self) -> impl Iterator<Item = EventId> {
        let count = self.event_count();
        (0..count).map(EventId)
    }

    /// 处理 ASS 脚本数据
    ///
    /// 这个方法用于 ASS 脚本工作流程，会解析完整的 ASS 脚本并创建相应的样式和事件。
    ///
    /// ⚠️ **重要**: 不要在使用手动创建样式/事件后调用此方法！libass 不支持混合使用。
    pub fn process_data(&mut self, data: &str) -> Result<()> {
        let c_data = CString::new(data).map_err(|_| anyhow!("ASS数据包含无效字符"))?;
        unsafe {
            ffi::ass_process_data(self.ptr, c_data.as_ptr(), data.len() as i32);
        }
        Ok(())
    }
    // 从 MKV 封装的字幕轨提取
    // void ass_process_codec_private(ASS_Track *track, const char *data, int size);
    // void ass_process_chunk(ASS_Track *track, const char *data, int size, long long timecode, long long duration);
    // void ass_set_check_readorder(ASS_Track *track, int check_readorder);

    /// 创建新事件
    pub fn create_event(&mut self, text: &str, style_id: StyleId) -> Result<EventHandle> {
        unsafe {
            let event_id = ffi::ass_alloc_event(self.ptr);
            if event_id < 0 {
                return Err(anyhow!("分配事件失败"));
            }

            let id = EventId(event_id);
            Ok(EventHandle { track: self, id }
                .set_text(text)?
                .set_style(style_id)
                // 由于 aweft 每次仅用 libass 渲染当前 event
                // 并不会将全部 event 传递，所以这里设置必然的时间范围
                .set_timing(0, i64::MAX))
        }
    }

    // 清理 track 当中显示时间已过的 event
    pub fn prune_events(&mut self, deadline: i64) {
        unsafe { ffi::ass_prune_events(self.ptr, deadline) }
    }

    // 配置每次调用 render_frame 时自动运行 prune_events 删除结束时间 < (当前渲染时间 now - delay) 的 event
    pub fn configure_prune(&mut self, delay: i64) {
        unsafe { ffi::ass_configure_prune(self.ptr, delay) }
    }

    // 删除 track 所有 event
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
// Rust风格的Style和Event管理
// ============================================================================

/// 样式ID，用于安全地引用Track中的样式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StyleId(pub(crate) i32);

/// 事件ID，用于安全地引用Track中的事件  
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId(pub(crate) i32);

/// 样式构建器，使用ID而不是直接指针
pub struct StyleHandle<'a> {
    track: &'a mut Track,
    id: StyleId,
}

/// 事件构建器，使用ID而不是直接指针
pub struct EventHandle<'a> {
    track: &'a mut Track,
    id: EventId,
}

impl StyleId {
    pub fn as_index(self) -> i32 {
        self.0
    }
}

impl EventId {
    pub fn as_index(self) -> i32 {
        self.0
    }
}

impl<'a> StyleHandle<'a> {
    /// 安全地获取样式指针（仅在需要时）
    fn as_mut_ptr(&mut self) -> *mut ffi::ASS_Style {
        unsafe {
            let track_ptr = self.track.as_ptr();
            debug_assert!(
                !(*track_ptr).styles.is_null()
                    && self.id.0 >= 0
                    && self.id.0 < (*track_ptr).n_styles
            );
            (*track_ptr).styles.offset(self.id.0 as isize)
        }
    }

    pub fn set_name(mut self, name: &str) -> Result<Self> {
        // C 侧虽然不会修改，但是它会 free 它，so can't direct use CString
        let c_name = CString::new(name).map_err(|_| anyhow!("样式名包含无效字符"))?;
        let c_name_bytes = c_name.as_bytes_with_nul();

        unsafe {
            let style_ptr = self.as_mut_ptr();
            if !(*style_ptr).Name.is_null() {
                ffi::ass_free((*style_ptr).Name as *mut c_void)
            }
            let p = ffi::ass_malloc(c_name_bytes.len() as size_t);
            if p.is_null() {
                return Err(anyhow!("分配内存失败"));
            }
            ptr::copy_nonoverlapping(c_name_bytes.as_ptr(), p as *mut u8, c_name_bytes.len());
            (*style_ptr).Name = p as *mut c_char;
        }
        Ok(self)
    }

    pub fn set_font_name(mut self, font_name: &str) -> Result<Self> {
        let c_font_name = CString::new(font_name).map_err(|_| anyhow!("字体名包含无效字符"))?;
        let c_font_name_bytes = c_font_name.as_bytes_with_nul();
        unsafe {
            let style_ptr = self.as_mut_ptr();
            if !(*style_ptr).FontName.is_null() {
                ffi::ass_free((*style_ptr).FontName as *mut c_void);
            }
            let p = ffi::ass_malloc(c_font_name_bytes.len() as size_t);
            if p.is_null() {
                return Err(anyhow!("分配内存失败"));
            }
            ptr::copy_nonoverlapping(
                c_font_name_bytes.as_ptr(),
                p as *mut u8,
                c_font_name_bytes.len(),
            );
            (*style_ptr).FontName = p as *mut c_char;
        }
        Ok(self)
    }

    pub fn set_font_size(mut self, size: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).FontSize = size;
        }
        self
    }

    pub fn set_primary_color(mut self, color: Color) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).PrimaryColour = color.to_libass_format();
        }
        self
    }

    pub fn set_secondary_color(mut self, color: Color) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).SecondaryColour = color.to_libass_format();
        }
        self
    }

    pub fn set_outline_color(mut self, color: Color) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).OutlineColour = color.to_libass_format();
        }
        self
    }

    pub fn set_back_color(mut self, color: Color) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).BackColour = color.to_libass_format();
        }
        self
    }

    pub fn set_bold(mut self, bold: bool) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Bold = if bold { 1 } else { 0 };
        }
        self
    }

    pub fn set_italic(mut self, italic: bool) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Italic = if italic { 1 } else { 0 };
        }
        self
    }

    // WARN: 需要使用枚举参数
    pub fn set_border_style(mut self, border_style: i32) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).BorderStyle = border_style;
        }
        self
    }

    pub fn set_outline(mut self, outline: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Outline = outline;
        }
        self
    }

    pub fn set_shadow(mut self, shadow: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Shadow = shadow;
        }
        self
    }

    pub fn set_alignment(mut self, alignment: Alignment) -> Self {
        let ffi_alignment: AssAlignment = alignment.into();
        let ffi_value: i32 = ffi_alignment.bits();
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Alignment = ffi_value;
        }
        self
    }

    pub fn set_margins(mut self, left: i32, right: i32, vertical: i32) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).MarginL = left;
            (*style_ptr).MarginR = right;
            (*style_ptr).MarginV = vertical;
        }
        self
    }

    /// 获取样式ID
    pub fn id(&self) -> StyleId {
        self.id
    }

    /// 初始化样式为 libass 的默认值
    /// 这个方法应该在创建新样式后立即调用
    pub fn initialize_defaults(mut self) -> Self {
        let style_ptr = self.as_mut_ptr();
        unsafe {
            // 设置默认的 Name（如果还没有设置）
            if (*style_ptr).Name.is_null() {
                let default_name = CString::new("Default").unwrap();
                let default_name_bytes = default_name.as_bytes_with_nul();
                if !(*style_ptr).FontName.is_null() {
                    ffi::ass_free((*style_ptr).FontName as *mut c_void);
                }
                let p = ffi::ass_malloc(default_name_bytes.len() as size_t);
                if p.is_null() {
                    panic!("默认样式名分配内存失败");
                }
                ptr::copy_nonoverlapping(
                    default_name_bytes.as_ptr(),
                    p as *mut u8,
                    default_name_bytes.len(),
                );
                (*style_ptr).Name = p as *mut c_char;
            }

            // 设置默认的 FontName（如果还没有设置）
            if (*style_ptr).FontName.is_null() {
                let default_font = CString::new("Arial").unwrap();
                let default_font_bytes = default_font.as_bytes_with_nul();

                if !(*style_ptr).FontName.is_null() {
                    ffi::ass_free((*style_ptr).FontName as *mut c_void);
                }
                let p = ffi::ass_malloc(default_font_bytes.len() as size_t);
                if p.is_null() {
                    panic!("默认样式字体名分配内存失败");
                }
                ptr::copy_nonoverlapping(
                    default_font_bytes.as_ptr(),
                    p as *mut u8,
                    default_font_bytes.len(),
                );
                (*style_ptr).FontName = p as *mut c_char;
            }

            // 设置其他默认值（基于 libass 的 set_default_style 函数）
            if (*style_ptr).FontSize == 0.0 {
                (*style_ptr).FontSize = 18.0;
            }
            if (*style_ptr).PrimaryColour == 0 {
                (*style_ptr).PrimaryColour = 0xffffff00; // 白色
            }
            if (*style_ptr).SecondaryColour == 0 {
                (*style_ptr).SecondaryColour = 0x00ffff00; // 青色
            }
            if (*style_ptr).OutlineColour == 0 {
                (*style_ptr).OutlineColour = 0x00000000; // 黑色
            }
            if (*style_ptr).BackColour == 0 {
                (*style_ptr).BackColour = 0x00000080; // 半透明黑色
            }
            if (*style_ptr).Bold == 0 {
                (*style_ptr).Bold = 200; // libass 默认值
            }
            if (*style_ptr).ScaleX == 0.0 {
                (*style_ptr).ScaleX = 1.0;
            }
            if (*style_ptr).ScaleY == 0.0 {
                (*style_ptr).ScaleY = 1.0;
            }
            if (*style_ptr).Spacing == 0.0 {
                (*style_ptr).Spacing = 0.0;
            }
            if (*style_ptr).BorderStyle == 0 {
                (*style_ptr).BorderStyle = 1;
            }
            if (*style_ptr).Outline == 0.0 {
                (*style_ptr).Outline = 2.0;
            }
            if (*style_ptr).Shadow == 0.0 {
                (*style_ptr).Shadow = 3.0;
            }
            if (*style_ptr).Alignment == 0 {
                (*style_ptr).Alignment = 2; // 底部居中
            }
            if (*style_ptr).MarginL == 0 && (*style_ptr).MarginR == 0 && (*style_ptr).MarginV == 0 {
                (*style_ptr).MarginL = 20;
                (*style_ptr).MarginR = 20;
                (*style_ptr).MarginV = 20;
            }
        }
        self
    }
}

impl<'a> EventHandle<'a> {
    /// 安全地获取事件指针（仅在需要时）
    fn as_mut_ptr(&mut self) -> *mut ffi::ASS_Event {
        unsafe {
            let track_ptr = self.track.as_ptr();
            debug_assert!(
                !(*track_ptr).styles.is_null()
                    && self.id.0 >= 0
                    && self.id.0 < (*track_ptr).n_styles
            );
            (*track_ptr).events.offset(self.id.0 as isize)
        }
    }

    pub fn set_text(mut self, text: &str) -> Result<Self> {
        let c_text = CString::new(text).map_err(|_| anyhow!("事件文本包含无效字符"))?;
        let c_text_bytes = c_text.as_bytes_with_nul();
        unsafe {
            let event_ptr = self.as_mut_ptr();
            if !(*event_ptr).Text.is_null() {
                ffi::ass_free((*event_ptr).Text as *mut c_void);
            }
            let p = ffi::ass_malloc(c_text_bytes.len() as size_t);
            if p.is_null() {
                return Err(anyhow!("分配内存失败"));
            }
            ptr::copy_nonoverlapping(c_text_bytes.as_ptr(), p as *mut u8, c_text_bytes.len());
            (*event_ptr).Text = p as *mut c_char;
        }
        Ok(self)
    }

    pub fn set_timing(mut self, start: i64, duration: i64) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).Start = start;
            (*event_ptr).Duration = duration;
        }
        self
    }

    pub fn set_style(mut self, style_id: StyleId) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).Style = style_id.0;
        }
        self
    }

    pub fn set_layer(mut self, layer: i32) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).Layer = layer;
        }
        self
    }

    pub fn set_margins(mut self, left: i32, right: i32, vertical: i32) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).MarginL = left;
            (*event_ptr).MarginR = right;
            (*event_ptr).MarginV = vertical;
        }
        self
    }

    pub fn set_effect(mut self, effect: Effect) -> Result<Self> {
        let c_effect = effect.to_cstring()?;
        let c_effect_bytes = c_effect.as_bytes_with_nul();
        unsafe {
            let event_ptr = self.as_mut_ptr();
            if !(*event_ptr).Effect.is_null() {
                ffi::ass_free((*event_ptr).Effect as *mut c_void);
            }
            let p = ffi::ass_malloc(c_effect_bytes.len() as size_t);
            if p.is_null() {
                return Err(anyhow!("分配内存失败"));
            }
            ptr::copy_nonoverlapping(c_effect_bytes.as_ptr(), p as *mut u8, c_effect_bytes.len());
            (*event_ptr).Effect = p as *mut c_char;
        }
        Ok(self)
    }
    pub fn set_name(mut self, name: &str) -> Result<Self> {
        let c_name = CString::new(name).map_err(|_| anyhow!("事件人物名包含无效字符"))?;
        let c_name_bytes = c_name.as_bytes_with_nul();
        unsafe {
            let event_ptr = self.as_mut_ptr();
            if !(*event_ptr).Name.is_null() {
                ffi::ass_free((*event_ptr).Name as *mut c_void);
            }
            let p = ffi::ass_malloc(c_name_bytes.len() as size_t);
            if p.is_null() {
                return Err(anyhow!("分配内存失败"));
            }
            ptr::copy_nonoverlapping(c_name_bytes.as_ptr(), p as *mut u8, c_name_bytes.len());
            (*event_ptr).Name = p as *mut c_char;
        }
        Ok(self)
    }

    /// 获取事件ID
    pub fn id(&self) -> EventId {
        self.id
    }
}
