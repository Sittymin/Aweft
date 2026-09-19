use serde::{Deserialize, Serialize};

use crate::application_graph::ApplicationLayer;
use crate::library::ProjectLibrary;
use crate::metadata::ProjectMetadata;
use crate::sequence::Sequence;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub meta: ProjectMetadata,
    pub library: ProjectLibrary,
    /// 序列：带时序的层级内容树，纯值语义。
    /// UI 侧字幕行编辑器/时间线均为此序列的不同视图。
    pub sequence: Sequence,
    /// 样式应用层：按优先级排列的应用图列表
    pub style_layer: ApplicationLayer,
}

impl Project {
    pub fn new() -> Self {
        Self {
            meta: ProjectMetadata::default(),
            library: ProjectLibrary::default(),
            sequence: Sequence::new(),
            style_layer: ApplicationLayer::new(),
        }
    }

    /// 只读访问序列。
    pub fn sequence(&self) -> &Sequence {
        &self.sequence
    }

    /// 可变访问序列。
    pub fn sequence_mut(&mut self) -> &mut Sequence {
        &mut self.sequence
    }
}
