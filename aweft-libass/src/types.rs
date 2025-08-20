

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
