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
            eprintln!("错误：Rcti.width() 中 xmax < xmin!");
            eprintln!(
                "  矩形: xmin={}, xmax={}, ymin={}, ymax={}",
                self.xmin, self.xmax, self.ymin, self.ymax
            );
            panic!("xmax ({}) < xmin ({})", self.xmax, self.xmin);
        }
        self.xmax - self.xmin + 1
    }

    /// 获取矩形的高度
    pub fn height(&self) -> u32 {
        if self.ymax < self.ymin {
            eprintln!("错误：Rcti.height() 中 ymax < ymin!");
            eprintln!(
                "  矩形: xmin={}, xmax={}, ymin={}, ymax={}",
                self.xmin, self.xmax, self.ymin, self.ymax
            );
            panic!("ymax ({}) < ymin ({})", self.ymax, self.ymin);
        }
        self.ymax - self.ymin + 1
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
