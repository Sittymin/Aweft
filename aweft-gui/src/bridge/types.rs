// 将原来的 types.rs 内容移动到这里
slint::include_modules!();
use slotmap::{Key, KeyData};

/// 将 Rust 的 AreaType 转换为 Slint 的 AreaType
pub fn rust_area_type_to_slint(area_type: &crate::screen::area::AreaType) -> AreaType {
    match area_type {
        // HACK: 可能Empty类型不需要
        crate::screen::area::AreaType::Empty => AreaType::LineEditor,
        crate::screen::area::AreaType::LineEditor => AreaType::LineEditor,
        crate::screen::area::AreaType::Viewport => AreaType::Viewport,
        crate::screen::area::AreaType::Timeline => AreaType::LineEditor,
        crate::screen::area::AreaType::Properties => AreaType::LineEditor,
    }
}
pub fn slint_area_type_to_rust(area_type: &AreaType) -> crate::screen::area::AreaType {
    match area_type {
        AreaType::LineEditor => crate::screen::area::AreaType::LineEditor,
        AreaType::Viewport => crate::screen::area::AreaType::Viewport,
    }
}

/// 通用：将 SlotMap 的 Key 拆分为 (index, version)
fn key_to_index_version<K: Key>(key: K) -> (i32, i32) {
    let ffi = key.data().as_ffi();
    let index = (ffi & 0xFFFF_FFFF) as i32;
    let version = (ffi >> 32) as i32;
    (index, version)
}

/// 通用：由 (index, version) 组装 SlotMap 的 Key
fn index_version_to_key<K: Key>(index: i32, version: i32) -> K {
    let ffi = ((version as u64) << 32) | (index as u64);
    K::from(KeyData::from_ffi(ffi))
}

/// 通用：SlotMap Key -> SlintKey
pub fn key_to_slint_area_key<K: Key>(key: K) -> SlintKey {
    let (index, version) = key_to_index_version(key);
    SlintKey {
        key_index: index,
        key_version: version,
    }
}

/// 通用：SlintKey -> SlotMap Key
pub fn slint_area_key_to_key<K: Key>(area_key: &SlintKey) -> K {
    index_version_to_key::<K>(area_key.key_index, area_key.key_version)
}

// 参数修改成相同的Rust定义
pub fn scale_split_overlay(data: SplitPreviewOverlayData, scale: f64) -> SplitPreviewOverlayData {
    let s = scale as f32;
    SplitPreviewOverlayData {
        x1: data.x1 / s,
        y1: data.y1 / s,
        width1: data.width1 / s,
        height1: data.height1 / s,
        x2: data.x2 / s,
        y2: data.y2 / s,
        width2: data.width2 / s,
        height2: data.height2 / s,
    }
}

pub fn scale_join_overlay(data: JoinHighlightOverlayData, scale: f64) -> JoinHighlightOverlayData {
    let s = scale as f32;
    JoinHighlightOverlayData {
        x: data.x / s,
        y: data.y / s,
        width: data.width / s,
        height: data.height / s,
    }
}

pub fn scale_close_overlays(
    datas: Vec<CloseAreaOverlayData>,
    scale: f64,
) -> Vec<CloseAreaOverlayData> {
    let s = scale as f32;
    datas
        .into_iter()
        .map(|d| CloseAreaOverlayData {
            x: d.x / s,
            y: d.y / s,
            width: d.width / s,
            height: d.height / s,
        })
        .collect()
}

/// UI缩放相关的设置
///
/// 设计原则（修正版）：
/// - window().size() 返回物理像素大小
/// - 内部所有计算使用物理像素（与Blender一致）
/// - 边距等UI元素在物理像素基础上缩放
/// - 最终传递给Slint时除以scale_factor转换为逻辑坐标
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeSettings {
    scale_factor: f64,                // 缩放因子（从Slint获取）
    base_border_width: f64,           // 基础边框宽度（物理像素）
    base_actionzone_side_length: f64, // 边角区域边长（物理像素）
    base_half_edge_width: f64,
    area_min_size: u32,
}

impl ThemeSettings {
    /// 从 MainWindow 实例和主题系统获取设置
    /// 这是创建 UISettings 的标准方法，确保所有值都从主题系统读取
    pub fn from_main_window(main_window: &MainWindow) -> Self {
        use slint::ComponentHandle;

        let theme = main_window.global::<Theme>();
        let window = main_window.window();

        Self {
            scale_factor: window.scale_factor() as f64,
            base_border_width: theme.get_data().main_window.border_width as f64,
            base_actionzone_side_length: theme.get_data().action_zone.side_length as f64,
            // 用于计算正确的中心线位置
            base_half_edge_width: theme.get_data().edge_display.width as f64 / 2.0,
            area_min_size: theme.get_data().main_window.area_min_size as u32,
        }
    }

    pub fn update_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// 创建测试用的 ThemeSettings
    #[cfg(test)]
    pub fn new(
        scale_factor: f64,
        base_border_width: f64,
        base_actionzone_side_length: f64,
        base_half_edge_width: f64,
        area_min_size: u32,
    ) -> Self {
        Self {
            scale_factor,
            base_border_width,
            base_actionzone_side_length,
            base_half_edge_width,
            area_min_size,
        }
    }

    /// 计算边距像素值（返回物理像素）
    pub fn calculate_margin(&self, is_edge: bool) -> u32 {
        if is_edge {
            // 窗口边缘的边距（物理像素）
            (self.get_scale_factor() * 2.0).min(self.get_border_width()) as u32
        } else {
            // 区域间的边距（物理像素）
            (self.get_border_width() * self.get_scale_factor()).max(self.get_scale_factor()) as u32
        }
    }

    pub fn get_scale_factor(&self) -> f64 {
        self.scale_factor
    }

    pub fn get_border_width(&self) -> f64 {
        self.base_border_width
    }

    pub fn get_actionzone_side_length(&self) -> f64 {
        self.base_actionzone_side_length
    }
    pub fn get_half_edge_width(&self) -> f64 {
        self.base_half_edge_width
    }

    // NOTE: 窗口边缘Edge自动吸附范围（用于对齐边缘）
    pub fn get_edge_filter_boundary(&self) -> f64 {
        self.area_min_size as f64
    }

    // NOTE: 窗口边缘Edge自动吸附目标位置（与窗口边缘留出一点空间）
    pub fn get_edge_snap_target(&self) -> f64 {
        self.base_border_width * self.scale_factor
    }

    pub fn get_area_min_size(&self) -> u32 {
        self.area_min_size
    }

    /// 检查点是否在边界过滤范围内（距离窗口边缘2倍border_width以内）
    pub fn is_in_edge_filter_boundary(&self, point: u32, window_edge: u32) -> bool {
        let distance = if point > window_edge {
            point - window_edge
        } else {
            window_edge - point
        };
        (distance as f64) <= self.get_edge_filter_boundary()
    }

    /// 将点对齐到边缘（1倍border_width距离）
    pub fn snap_point_to_edge(&self, point: u32, window_min: u32, window_max: u32) -> u32 {
        let target = self.get_edge_snap_target();
        if (point as f64 - window_min as f64) < target {
            return window_min;
        }
        if (window_max as f64 - point as f64) < target {
            return window_max;
        }
        point
    }
}
