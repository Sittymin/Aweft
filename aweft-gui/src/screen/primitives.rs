//! 屏幕几何基础类型（矩形、向量、邻接关系）。
//!
//! # 坐标系约定（务必阅读，混用是历史上 bug 的根源）
//!
//! screen 层存在**三个坐标系**，语义完全不同，禁止混用：
//!
//! 1. **物理像素 · 原始顶点坐标（`geometry_rect`）**：`ScrVert` 的真实位置，
//!    几何运算（创建/移动顶点、join/split 内部、`apply_verts_move`）一律用它。
//! 2. **物理像素 · 布局坐标（`layout_rect`）**：`geometry_rect` 收缩 margin 后
//!    的结果，即 `tot_rct`。UI 显示、命中检测（`find_area_at_position`）、
//!    分割/合并预览用。二者差一个 margin（非全屏/非临时/非单 area 时）。
//! 3. **逻辑像素**：最终写入 Slint 属性的坐标，物理像素 ÷ `scale_factor`。
//!    converter 层（`bridge/converters`）负责转换，用 `ThemeSettings::to_logical_px`
//!    / `to_physical_px`，不要手写 `as f64 / scale`。
//!
//! 引用 Blender：几何以物理像素存储（与 Blender 一致），仅在 UI 输出时换算
//! 逻辑像素（对应 Slint 的 `LogicalLength`/`PhysicalLength` 区分）。

use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub struct Vec2u32 {
    pub x: u32,
    pub y: u32,
}
#[derive(Debug, Clone)]
pub struct Vec2f64 {
    pub x: f64,
    pub y: f64,
}
impl Vec2u32 {
    /// 安全地将 Vec2f64 转换为 Vec2u32，返回 Result
    ///
    /// # Errors
    ///
    /// 当输入的 x 或 y 坐标为负数时返回错误，因为无法将负数转换为无符号整数
    ///
    /// # Examples
    ///
    /// ```
    /// # use aweft_gui::screen::primitives::{Vec2u32, Vec2f64};
    /// let positive = Vec2f64 { x: 10.5, y: 20.3 };
    /// let result = Vec2u32::try_from_f32(positive);
    /// assert!(result.is_ok());
    ///
    /// let negative = Vec2f64 { x: -5.0, y: 10.0 };
    /// let result = Vec2u32::try_from_f32(negative);
    /// assert!(result.is_err());
    /// ```
    pub fn try_from_f32(value: Vec2f64) -> Result<Self> {
        if value.x < 0.0 || value.y < 0.0 {
            return Err(anyhow!(
                "无法将负数坐标转换为无符号整数: x={}, y={}。负数会导致数据丢失",
                value.x,
                value.y
            ));
        }
        Ok(Self {
            x: value.x as u32,
            y: value.y as u32,
        })
    }
}

/// 整数定义的矩形区域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rcti {
    pub xmin: u32,
    pub xmax: u32,
    pub ymin: u32,
    pub ymax: u32,
}

impl Rcti {
    /// 从四个顶点坐标计算矩形边界（类似Blender中的实现）
    /// 顶点顺序不重要，函数会自动计算边界框
    pub fn from_vertices(v1: &Vec2u32, v2: &Vec2u32, v3: &Vec2u32, v4: &Vec2u32) -> Self {
        // 根据四个顶点找出最小和最大的x、y坐标
        let xmin = v1.x.min(v2.x).min(v3.x).min(v4.x);
        let xmax = v1.x.max(v2.x).max(v3.x).max(v4.x);
        let ymin = v1.y.min(v2.y).min(v3.y).min(v4.y);
        let ymax = v1.y.max(v2.y).max(v3.y).max(v4.y);

        Self {
            xmin,
            xmax,
            ymin,
            ymax,
        }
    }

    /// 获取矩形的宽度
    pub fn width(&self) -> u32 {
        if self.xmax < self.xmin {
            log::error!("错误：Rcti.width() 中 xmax < xmin!");
            log::error!(
                "  矩形: xmin={}, xmax={}, ymin={}, ymax={}",
                self.xmin,
                self.xmax,
                self.ymin,
                self.ymax
            );
            panic!("xmax ({}) < xmin ({})", self.xmax, self.xmin);
        }
        self.xmax - self.xmin + 1
    }

    /// 获取矩形的高度
    pub fn height(&self) -> u32 {
        if self.ymax < self.ymin {
            log::error!("错误：Rcti.height() 中 ymax < ymin!");
            log::error!(
                "  矩形: xmin={}, xmax={}, ymin={}, ymax={}",
                self.xmin,
                self.xmax,
                self.ymin,
                self.ymax
            );
            panic!("ymax ({}) < ymin ({})", self.ymax, self.ymin);
        }
        self.ymax - self.ymin + 1
    }
}

/// 系统栏占掉的一圈（物理像素）。桌面恒为全 0；Android 上来自
/// `Window.safe-area-insets`，见 `bridge::callbacks::handle_rendering_setup`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Insets {
    /// 窗口物理尺寸扣掉四周后的可用矩形（包含端坐标）。
    /// 某一侧内缩超过窗口尺寸时收成零宽/零高的退化矩形，不做无符号回绕。
    pub fn usable_rect(&self, width: u32, height: u32) -> Rcti {
        let last_x = width.saturating_sub(1);
        let last_y = height.saturating_sub(1);
        Rcti {
            xmin: self.left.min(last_x),
            xmax: last_x.saturating_sub(self.right).max(self.left),
            ymin: self.top.min(last_y),
            ymax: last_y.saturating_sub(self.bottom).max(self.top),
        }
    }
}

/// 浮点数定义的矩形区域
#[derive(Debug, Clone)]
pub struct Rctf {
    pub xmin: f32,
    pub xmax: f32,
    pub ymin: f32,
    pub ymax: f32,
}

/// 描述两个区域的邻接关系
#[derive(Debug, Clone, Copy)]
pub enum Adjacency {
    /// Source 在 Target 上方
    TopToBottom { top: Rcti, bottom: Rcti },
    /// Source 在 Target 下方
    BottomToTop { bottom: Rcti, top: Rcti },
    /// Source 在 Target 左方
    LeftToRight { left: Rcti, right: Rcti },
    /// Source 在 Target 右方
    RightToLeft { right: Rcti, left: Rcti },
}
