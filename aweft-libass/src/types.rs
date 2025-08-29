use crate::ffi;
use palette::{Srgb, Srgba, WithAlpha};
use std::ops::{Deref, DerefMut};
use std::slice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssColorRgb(pub Srgb<u8>);
impl Deref for AssColorRgb {
    type Target = Srgb<u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for AssColorRgb {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AssColorRgb {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self(Srgb::new(red, green, blue))
    }

    pub fn with_alpha(self, alpha: u8) -> AssColorRgba {
        // NOTE: 避免调用自己
        AssColorRgba(self.0.with_alpha(alpha))
    }
    pub fn to_ass_override_u32(&self) -> u32 {
        ((self.blue as u32) << 16) | ((self.green as u32) << 8) | self.red as u32
    }
    // ASS 的覆盖规范是 &HBBGGRR
    pub fn from_ass_override_u32(color: u32) -> Self {
        Self::new(
            (color & 0xFF) as u8,
            ((color >> 8) & 0xFF) as u8,
            ((color >> 16) & 0xFF) as u8,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssColorRgba(pub Srgba<u8>);

impl Deref for AssColorRgba {
    type Target = Srgba<u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for AssColorRgba {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl From<AssColorRgba> for Srgba<u8> {
    fn from(color: AssColorRgba) -> Self {
        color.0
    }
}

impl AssColorRgba {
    pub fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self(Srgba::new(red, green, blue, alpha))
    }

    // 转换成归一化的 f32
    pub fn to_f32_array(&self) -> [f32; 4] {
        self.into_format::<f32, f32>().into()
    }

    // WARN: libass 颜色表示为 RGBA 的 u32 当中的 A 是**透明度** (转换为不透明度)
    pub fn to_libass_u32(&self) -> u32 {
        ((self.red as u32) << 24)
            | ((self.green as u32) << 16)
            | ((self.blue as u32) << 8)
            // WARN: 将不透明度转换为 libass 透明度
            | (255u8.saturating_sub(self.alpha) as u32)
    }

    // WARN: 从 libass 的 RGBA 格式解析 (A 是**透明度**)
    pub fn from_libass_u32(color: u32) -> Self {
        Self::new(
            ((color >> 24) & 0xFF) as u8,
            ((color >> 16) & 0xFF) as u8,
            ((color >> 8) & 0xFF) as u8,
            // WARN: 将 libass 的透明度转换为不透明度
            255u8.saturating_sub((color & 0xFF) as u8),
        )
    }
    pub fn to_ass_style_u32(&self) -> u32 {
        // WARN: 将不透明度转换为 ASS 规范的透明度
        ((255u8.saturating_sub(self.alpha) as u32) << 24)
            | ((self.blue as u32) << 16)
            | ((self.green as u32) << 8)
            | self.red as u32
    }
    // ASS 的样式规范是 &HAABBGGRR (A 是透明度)
    pub fn from_ass_style_u32(color: u32) -> Self {
        Self::new(
            (color & 0xFF) as u8,
            ((color >> 8) & 0xFF) as u8,
            ((color >> 16) & 0xFF) as u8,
            // WARN: 将 ASS 规范的透明度转换为不透明度
            255u8.saturating_sub(((color >> 24) & 0xFF) as u8),
        )
    }
}
#[derive(Debug, Clone, Copy)]
pub enum ImageType {
    Character = 0,
    Outline = 1,
    Shadow = 2,
}

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

    pub fn color(&self) -> AssColorRgba {
        unsafe { AssColorRgba::from_libass_u32((*self.ptr).color) }
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
}

pub struct ImageIterator {
    current: *const ffi::ASS_Image,
}

impl ImageIterator {
    pub(crate) fn new(first: *const ffi::ASS_Image) -> Self {
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
