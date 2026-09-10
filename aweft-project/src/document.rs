use aweft_model::project::Project;
use serde::{Deserialize, Serialize};

/// 工程文件格式版本。读到不支持的版本必须拒绝并报错，不做静默兼容。
pub const AWEFT_DOCUMENT_FORMAT: u32 = 1;

/// 可写入磁盘的屏幕布局表（占位形状）。
/// 后续把 `ScrAreaMap` 的平面数据搬入 `areas`，首版保持为空表。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayoutTable {
    /// 布局子格式版本，与文档 `format` 独立演进。
    pub schema: u32,
    pub areas: Vec<LayoutArea>,
}

/// 单个布局区域的磁盘形状：字符串键 + 包围矩形。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutArea {
    pub id: String,
    /// 包围矩形 [xmin, ymin, xmax, ymax]，物理像素。
    pub rect: [i32; 4],
}

/// 可写入磁盘的视频条目：只含描述数据，不含管线句柄。
/// 管线/解码器/`wgpu` 句柄与拖拽中间态永远不进入文件，打开工程后由界面重建。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaEntry {
    pub uri: String,
    /// 视口世界坐标位置（相对主视频世界矩形左上角）。
    pub x: f64,
    pub y: f64,
    pub muted: bool,
    pub selected: bool,
}

/// 统一工程文档：共享域真相 + 界面专属可落盘数据。
/// `Project` 归属 `aweft-model`，本结构只做包裹与版本门。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AweftDocument {
    pub format: u32,
    pub project: Project,
    pub layout: LayoutTable,
    pub media: Vec<MediaEntry>,
}

impl AweftDocument {
    pub fn new() -> Self {
        Self {
            format: AWEFT_DOCUMENT_FORMAT,
            project: Project::new(),
            layout: LayoutTable::default(),
            media: Vec::new(),
        }
    }
}

impl Default for AweftDocument {
    fn default() -> Self {
        Self::new()
    }
}
