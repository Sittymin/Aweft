use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::application_graph::ApplicationLayer;
use crate::library::ProjectLibrary;
use crate::metadata::ProjectMetadata;
use crate::structure::ProjectStructure;

#[derive(Debug, Clone)]
pub struct Project {
    /// 版本号：用于未来的迁移和兼容性检查
    pub version: u32,

    pub meta: ProjectMetadata,
    pub library: ProjectLibrary,
    /// 字幕结构：`Rc<RefCell>` 共享。reducer 用 `structure_mut()` 修改（零拷贝，
    /// 内部可变性），UI 数据源模型持同一 Rc 实时读取。
    pub structure: Rc<RefCell<ProjectStructure>>,
    /// 样式应用层：按优先级排列的应用图列表
    pub style_layer: ApplicationLayer,
}

impl Project {
    pub fn new() -> Self {
        Self {
            version: 0,
            meta: ProjectMetadata::default(),
            library: ProjectLibrary::default(),
            structure: Rc::new(RefCell::new(ProjectStructure::new())),
            style_layer: ApplicationLayer::new(),
        }
    }

    /// 只读访问字幕结构。
    pub fn structure(&self) -> Ref<ProjectStructure> {
        self.structure.borrow()
    }

    /// 可变访问字幕结构（内部可变性，零拷贝）。
    pub fn structure_mut(&mut self) -> RefMut<ProjectStructure> {
        self.structure.borrow_mut()
    }
}

impl Serialize for Project {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Project", 5)?;
        s.serialize_field("version", &self.version)?;
        s.serialize_field("meta", &self.meta)?;
        s.serialize_field("library", &self.library)?;
        s.serialize_field("structure", &*self.structure.borrow())?;
        s.serialize_field("style_layer", &self.style_layer)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for Project {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct ProjectRepr {
            version: u32,
            meta: ProjectMetadata,
            library: ProjectLibrary,
            structure: ProjectStructure,
            style_layer: ApplicationLayer,
        }
        let repr = ProjectRepr::deserialize(deserializer)?;
        Ok(Project {
            version: repr.version,
            meta: repr.meta,
            library: repr.library,
            structure: Rc::new(RefCell::new(repr.structure)),
            style_layer: repr.style_layer,
        })
    }
}
