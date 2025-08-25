use crate::ffi;
use std::slice;

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

    // 转换成归一化的 f32
    pub fn to_f32_array(&self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
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

    // 全新混合之前需要清空缓冲区
    pub fn blend_into_buffer(&self, buffer: &mut [u8]) {
        let width = self.width();
        let height = self.height();
        if width <= 0 || height <= 0 {
            return;
        }

        let expected_size = (width * height * 4) as usize;
        assert_eq!(
            buffer.len(),
            expected_size,
            "Provided buffer has incorrect size"
        );

        let color = self.color();
        let bitmap = self.bitmap();
        let stride = self.stride();
        let dst_x = self.dst_x();
        let dst_y = self.dst_y();

        for y in 0..height {
            for x in 0..width {
                // 从 bitmap 中获取源像素的 Alpha 值
                let bitmap_index = (y * stride + x) as usize;
                let src_alpha_value = bitmap[bitmap_index];

                // 如果像素是完全透明的，则跳过，不做任何处理。
                if src_alpha_value == 0 {
                    continue;
                }

                // 计算这个像素在最终目标画布上的坐标
                let target_x = dst_x + x;
                let target_y = dst_y + y;

                // 计算该像素在 buffer 中的起始索引
                let target_base_index = ((target_y * width) + target_x) as usize * 4;
                // a. 将源 Alpha 转换为 0.0-1.0 的浮点数
                let src_alpha = src_alpha_value as f32 / 255.0;

                // b. 将源颜色 (R, G, B) 与其 Alpha 相乘，得到预乘颜色
                let pm_src_r = color.r as f32 * src_alpha;
                let pm_src_g = color.g as f32 * src_alpha;
                let pm_src_b = color.b as f32 * src_alpha;

                // c. 读取画布上已有的目标颜色 (Destination color)
                //    由于我们不断在 buffer 上混合，所以 buffer 中的颜色已经是预乘格式了
                let dst_r = buffer[target_base_index] as f32;
                let dst_g = buffer[target_base_index + 1] as f32;
                let dst_b = buffer[target_base_index + 2] as f32;
                let dst_alpha = buffer[target_base_index + 3] as f32 / 255.0;

                // d. 应用标准的预乘 Alpha 混合公式: Out = Src + Dst * (1 - SrcA)
                let one_minus_src_alpha = 1.0 - src_alpha;
                let out_r = pm_src_r + dst_r * one_minus_src_alpha;
                let out_g = pm_src_g + dst_g * one_minus_src_alpha;
                let out_b = pm_src_b + dst_b * one_minus_src_alpha;

                // e. Alpha 通道也需要混合
                let out_alpha = src_alpha + dst_alpha * one_minus_src_alpha;

                // f. 将计算出的新颜色写回画布
                buffer[target_base_index] = out_r as u8;
                buffer[target_base_index + 1] = out_g as u8;
                buffer[target_base_index + 2] = out_b as u8;
                buffer[target_base_index + 3] = (out_alpha * 255.0) as u8;
            }
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
