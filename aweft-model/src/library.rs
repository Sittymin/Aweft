use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::animation::AnimationLibrary;
use crate::ids::{ActorId, NodeGraphId};
use crate::node_graph::NodeGraph;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {/* ... */}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectLibrary {
    pub actors: HashMap<ActorId, Actor>,
    pub animation_library: AnimationLibrary,
    pub node_graphs: HashMap<NodeGraphId, NodeGraph>,
}
