//! 工程文件的磁盘形状：一份 `Project` 加上只属于界面的可落盘数据。
//!
//! 这个类型回答的是"磁盘上有什么"，不是"运行时有什么"：管线句柄、解码器、`wgpu`
//! 资源、拖拽中间态一律不进这里，打开工程后由界面重建。协作快照用的也是这一份形状
//! （见 `aweft-gui` 的 `collab_bridge`），所以文件的版本门与网络的版本门是同一道。

use aweft_model::project::Project;
use aweft_storage::JsonFile;
use serde::{Deserialize, Serialize};

/// 工程文件格式版本。加字段不 bump（容器级 `#[serde(default)]` 兜住旧文件）；
/// 只有"旧值需要改写才有意义"的改动才 bump，并在 [`JsonFile::migrate`] 里补一步。
pub const AWEFT_PROJECT_FORMAT: u32 = 1;

/// 可写入磁盘的屏幕布局表（占位形状）。
/// 后续把 `ScrAreaMap` 的平面数据搬入 `areas`，首版保持为空表。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayoutTable {
    /// 布局子格式版本，与工程文件 `format` 独立演进。
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

/// 存盘与协作快照共用的那一份工程形状：共享域真相 + 界面专属可落盘数据。
/// `Project` 归属 `aweft-model`，本结构只做包裹与版本门。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedProject {
    /// 文件自身的格式版本。缺字段的旧文件按 0 处理，补写之后盖成当前值。
    pub format: u32,
    pub project: Project,
    pub layout: LayoutTable,
    pub media: Vec<MediaEntry>,
}

impl Default for SavedProject {
    /// 空工程：`Project` 自己没有 `Default`（构造要走 `Project::new`），
    /// 所以这里手写一份，`new()` 与 serde 的容器级 `#[serde(default)]` 共用它。
    fn default() -> Self {
        Self {
            format: AWEFT_PROJECT_FORMAT,
            project: Project::new(),
            layout: LayoutTable::default(),
            media: Vec::new(),
        }
    }
}

impl SavedProject {
    pub fn new() -> Self {
        Self::default()
    }
}

impl JsonFile for SavedProject {
    const FORMAT: u32 = AWEFT_PROJECT_FORMAT;

    fn format(&self) -> u32 {
        self.format
    }

    fn set_format(&mut self, format: u32) {
        self.format = format;
    }
}
