use crate::ffi;
use crate::types::Color;
use image::{Rgba, RgbaImage};
use std::slice;

// ============================================================================
// Image 封装
// ============================================================================

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
