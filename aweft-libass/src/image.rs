use crate::color::AssColorRgba;
use crate::ffi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Character = 0,
    Outline = 1,
    Shadow = 2,
}

/// Rust-owned copy of one node from libass's image list.
#[derive(Debug, Clone)]
pub struct Image {
    width: i32,
    height: i32,
    stride: i32,
    dst_x: i32,
    dst_y: i32,
    color: AssColorRgba,
    image_type: ImageType,
    bitmap: Vec<u8>,
}

impl Image {
    pub(crate) unsafe fn from_raw(ptr: *const ffi::ASS_Image) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        let raw = unsafe { &*ptr };
        let bitmap_len = if raw.bitmap.is_null() || raw.w <= 0 || raw.h <= 0 || raw.stride <= 0 {
            0
        } else {
            (raw.stride as usize)
                .checked_mul((raw.h - 1) as usize)?
                .checked_add(raw.w as usize)?
        };
        let bitmap = if bitmap_len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(raw.bitmap, bitmap_len) }.to_vec()
        };
        Some(Self {
            width: raw.w,
            height: raw.h,
            stride: raw.stride,
            dst_x: raw.dst_x,
            dst_y: raw.dst_y,
            color: AssColorRgba::from_libass_u32(raw.color),
            image_type: match raw.type_ {
                0 => ImageType::Character,
                1 => ImageType::Outline,
                2 => ImageType::Shadow,
                unknown => unreachable!("libass returned unknown ASS_Image type: {unknown}"),
            },
            bitmap,
        })
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn stride(&self) -> i32 {
        self.stride
    }

    pub fn dst_x(&self) -> i32 {
        self.dst_x
    }

    pub fn dst_y(&self) -> i32 {
        self.dst_y
    }

    pub fn color(&self) -> AssColorRgba {
        self.color
    }

    pub fn image_type(&self) -> ImageType {
        self.image_type
    }

    pub fn bitmap(&self) -> &[u8] {
        &self.bitmap
    }
}
