use crate::library::ProjectLibrary;
use crate::metadata::ProjectMetadata;
use crate::structure::ProjectStructure;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    // 版本号：用于未来的迁移和兼容性检查
    pub version: u32,

    pub meta: ProjectMetadata,
    pub library: ProjectLibrary,
    pub structure: ProjectStructure,
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
        }
    }
}
