use serde::{Deserialize, Serialize};

use crate::application_graph::ApplicationLayer;
use crate::library::ProjectLibrary;
use crate::metadata::ProjectMetadata;
use crate::structure::ProjectStructure;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// 版本号：用于未来的迁移和兼容性检查
    pub version: u32,

    pub meta: ProjectMetadata,
    pub library: ProjectLibrary,
    /// 字幕结构：纯值语义（model 库不持有共享可变状态）。
    /// UI 侧需要共享读时，用 Repository trait 在 GUI 层做投影（对齐 todo-mvc 模式）。
    pub structure: ProjectStructure,
    /// 样式应用层：按优先级排列的应用图列表
    pub style_layer: ApplicationLayer,
}

impl Project {
    pub fn new() -> Self {
        Self {
            version: 0,
            meta: ProjectMetadata::default(),
            library: ProjectLibrary::default(),
            structure: ProjectStructure::new(),
            style_layer: ApplicationLayer::new(),
        }
    }

    /// 只读访问字幕结构。
    pub fn structure(&self) -> &ProjectStructure {
        &self.structure
    }

    /// 可变访问字幕结构。
    pub fn structure_mut(&mut self) -> &mut ProjectStructure {
        &mut self.structure
    }
}
