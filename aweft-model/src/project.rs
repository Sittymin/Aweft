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
    pub structure: ProjectStructure,
    /// 样式应用层：按优先级排列的应用图列表（应用图编辑器编辑）
    pub style_layer: ApplicationLayer,
}

impl Project {
    pub fn new() -> Self {
        Self {
            version: 0,
            meta: ProjectMetadata::default(),
            library: ProjectLibrary::default(),
            structure: ProjectStructure {
                root_items: Vec::new(),
                items: std::collections::HashMap::new(),
            },
            style_layer: ApplicationLayer::new(),
        }
    }
}
