use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::animation::AnimationLibrary;
use crate::ids::{ActorId, NodeGraphId};
use crate::node_graph::NodeGraph;

/// 字幕角色（Actor）：应用图匹配/样式依据。
///
/// 只存主名字——匹配（`MatchActor` 的 source）直接用 `name`。
/// 别名/颜色等属于更高层（UI/导入器）的呈现，不入 model；
/// model 是比 Aegisub 更高层的抽象，不依赖外部字幕格式的导入归一。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub id: ActorId,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectLibrary {
    pub actors: HashMap<ActorId, Actor>,
    pub animation_library: AnimationLibrary,
    pub node_graphs: HashMap<NodeGraphId, NodeGraph>,
}

impl ProjectLibrary {
    /// 追加/覆盖一个角色。
    pub fn insert_actor(&mut self, actor: Actor) {
        self.actors.insert(actor.id, actor);
    }

    /// 取角色主名（应用图匹配的 source；未知角色返回 None）。
    pub fn actor_name(&self, id: ActorId) -> Option<&str> {
        self.actors.get(&id).map(|a| a.name.as_str())
    }
}
