//! 自动彩虹调色板：按下标取色（彩虹色系）。
//! 曲线编辑器通道颜色（`store/animation/graph/channel.rs`）与多视频控制时间条
//! （`reducers/media.rs`）共用同一调色板，保证两类彩色元素视觉一致。

use slint::Color;

/// 自动彩虹调色板：按下标取色（彩虹色系），超过 6 色循环。
pub fn rainbow_color(index: usize) -> Color {
    const PALETTE: [(u8, u8, u8); 6] = [
        (242, 109, 120), // 红
        (240, 193, 78),  // 黄
        (99, 199, 99),   // 绿
        (78, 205, 196),  // 青
        (112, 178, 255), // 蓝
        (170, 122, 214), // 紫
    ];
    let (r, g, b) = PALETTE[index % PALETTE.len()];
    Color::from_rgb_u8(r, g, b)
}
