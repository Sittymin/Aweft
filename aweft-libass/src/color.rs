use palette::{Srgb, Srgba, WithAlpha};
use std::ops::{Deref, DerefMut};

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
    // WARN: libass 颜色表示为 RGBA 的 u32 当中的 A 是**透明度** (palette 中为不透明度)
    pub fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self(Srgba::new(red, green, blue, alpha))
    }

    pub fn to_libass_u32(&self) -> u32 {
        ((self.red as u32) << 24)
            | ((self.green as u32) << 16)
            | ((self.blue as u32) << 8)
            | (255u8.saturating_sub(self.alpha) as u32)
    }

    pub fn from_libass_u32(color: u32) -> Self {
        Self::new(
            ((color >> 24) & 0xFF) as u8,
            ((color >> 16) & 0xFF) as u8,
            ((color >> 8) & 0xFF) as u8,
            255u8.saturating_sub((color & 0xFF) as u8),
        )
    }
    pub fn to_ass_style_u32(&self) -> u32 {
        ((255u8.saturating_sub(self.alpha) as u32) << 24)
            | ((self.blue as u32) << 16)
            | ((self.green as u32) << 8)
            | self.red as u32
    }
    pub fn from_ass_style_u32(color: u32) -> Self {
        Self::new(
            (color & 0xFF) as u8,
            ((color >> 8) & 0xFF) as u8,
            ((color >> 16) & 0xFF) as u8,
            255u8.saturating_sub(((color >> 24) & 0xFF) as u8),
        )
    }

    pub fn to_f32_array(&self) -> [f32; 4] {
        self.into_format::<f32, f32>().into()
    }
}
