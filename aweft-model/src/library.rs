use crate::ids::{ActorId, CurveId, NodeGraphId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// 预先声明一下之后要设计的结构体，占个坑
// 这些会在后续步骤详细设计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {/* ... */}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartCurve {/* ... */}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeGraph {/* ... */}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectLibrary {
    // 用户下拉菜单当中直接按首字母排序
    // 角色表：Map<ActorId, Actor>
    pub actors: HashMap<ActorId, Actor>,

    // 曲线库：存储所有自定义的“9-slice”动画曲线
    pub curves: HashMap<CurveId, SmartCurve>,

    // 节点图
    pub node_graphs: HashMap<NodeGraphId, NodeGraph>,
}

impl Default for ProjectLibrary {
    fn default() -> Self {
        Self {
            actors: HashMap::new(),
            curves: HashMap::new(),
            node_graphs: HashMap::new(),
        }
    }
}
