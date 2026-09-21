//! 树类型注册表（Blender `bNodeTreeType` 对应物）。
//!
//! 一张"图"（样式图 / 应用图）共享同一套拓扑数据结构 `NodeGraph<K>`，
//! 差异全部由 `K: NodeTreeType` 这一"树类型"承担：
//!  - 节点目录（`socket_decls`）
//!  - 节点合法性（`validate_add` / `is_static`）
//!  - 求值语义（`evaluate_node` / `collect_result`）
//!  - 引用数据（`reference_id` / `anim_ref_id`）
//!
//! 对应 Blender：`bNodeTree` 对所有编辑器统一，Shader/Geometry 的差异收进
//! 各自注册的 `bNodeTreeType` 回调，编辑器运行时委托回调而非 `match` 编辑器类型。
//!
//! 纯 model：UI 呈现/本地化（节点标题、端口交互语义）不属于本模块，
//! 由 GUI 侧的 `NodeUi` 注册表承担。

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::application_graph::LineContext;
use crate::ids::{AnimDataId, NodeGraphId, NodeId, SocketId};
use crate::node_graph::style::StyleResult;

use super::error::EvalError;
use super::graph::NodeGraph;
use super::socket::{ApplicationUnit, SocketDecl, SocketValue};

/// 外部求值上下文：应用图求值需要行信息与样式链结果；样式图求值不需要（传 `style()`）。
#[derive(Debug, Clone)]
pub struct EvalContext<'a> {
    pub line: Option<&'a LineContext>,
    pub style_results: Option<&'a HashMap<NodeGraphId, StyleResult>>,
}

impl<'a> EvalContext<'a> {
    /// 样式图求值：无外部输入。
    pub fn style() -> Self {
        Self {
            line: None,
            style_results: None,
        }
    }

    /// 应用图求值：行信息 + 所有被引用样式链的求值结果。
    pub fn app(
        line: &'a LineContext,
        style_results: &'a HashMap<NodeGraphId, StyleResult>,
    ) -> Self {
        Self {
            line: Some(line),
            style_results: Some(style_results),
        }
    }
}

/// 求值缓存：两种图共用一份（应用图多写 `node_applications`，样式图只用其余两个）。
#[derive(Debug, Clone, Default)]
pub struct EvalCache {
    pub socket_values: HashMap<SocketId, SocketValue>,
    pub node_styles: HashMap<NodeId, StyleResult>,
    /// 应用匹配 / 应用动画节点的产出单元（组输出按端口顺序收集）。
    pub node_applications: HashMap<NodeId, ApplicationUnit>,
}

/// 节点图类型（树类型标签）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeGraphKind {
    /// 样式图（`NodeGraph<NodeKind>`）
    Style,
    /// 应用图（`NodeGraph<ApplicationNodeKind>`）
    App,
}

/// 树类型注册表：每种图类型实现本 trait，提供目录/合法性/求值/UI 全部差异。
pub trait NodeTreeType: Clone + PartialEq + Eq + fmt::Debug + 'static {
    /// 图级求值结果：样式图为 `StyleResult`，应用图为 `Option<ApplicationGraphMatch>`。
    type Result;

    /// 添加节点前校验（如样式图只允许一个 Viewer）。
    fn validate_add(&self, graph: &NodeGraph<Self>) -> Result<(), EvalError>;

    /// 是否完全静态节点（不允许通过 extra 追加端口）。
    fn is_static(&self) -> bool;

    /// 节点基础 socket 接口声明（半动态节点返回基础声明，调用方可追加）。
    fn socket_decls(&self) -> Option<(Vec<SocketDecl>, Vec<SocketDecl>)>;

    /// 求值单个节点，结果写入 `cache`。
    fn evaluate_node(
        &self,
        graph: &NodeGraph<Self>,
        node_id: NodeId,
        cache: &mut EvalCache,
        ctx: &EvalContext,
    ) -> Result<(), EvalError>;

    /// 图级结果收集（样式图：viewer/组输出；应用图：组输出端口顺序合并）。
    fn collect_result(graph: &NodeGraph<Self>, cache: &EvalCache) -> Result<Self::Result, EvalError>;

    /// 样式引用节点（Reference / AppRef）引用的样式链 id；非引用节点返回 None。
    fn reference_id(&self) -> Option<NodeGraphId>;

    /// 原位改引用链（仅改 kind 内嵌 id + 由调用方标脏，不删节点）。
    /// 非引用节点返回 `EvalError::NotReferenceNode`。
    fn set_reference_id(&mut self, _new_id: NodeGraphId) -> Result<(), EvalError> {
        Err(EvalError::NotReferenceNode)
    }

    /// 非线性动画引用节点（`ApplicationNodeKind::AnimRef`）引用的动画片段 id；
    /// 其余节点返回 None。载荷是 `AnimDataId`，与 `reference_id` 的 `NodeGraphId`
    /// 不是同一种 id，所以单列一对方法。
    fn anim_ref_id(&self) -> Option<AnimDataId> {
        None
    }

    /// 原位改引用的动画片段（同 `set_reference_id`：仅改 kind 内嵌 id，不删节点）。
    fn set_anim_ref_id(&mut self, _new_id: AnimDataId) -> Result<(), EvalError> {
        Err(EvalError::NotReferenceNode)
    }
}
