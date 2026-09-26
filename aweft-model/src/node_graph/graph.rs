use std::collections::{HashMap, HashSet};

use palette::Srgba;
use serde::{Deserialize, Serialize};

use crate::ids::{NodeGraphId, NodeId, SocketId};

use super::error::EvalError;
use super::kind::{EvalCache, EvalContext, NodeTreeType};
use super::math::{Vec2, Vec3};
use super::node::{ColorRole, FromNode, Node, NodeKind, StyleOpKind, VecOpKind, get_socket_decls};
use super::socket::{InputSocket, Link, Socket, SocketDecl, SocketRef, SocketValue};
use super::style::StyleResult;

/// 通用节点图：拓扑结构对所有树类型统一（Blender `bNodeTree` 对应物）。
/// 差异（节点目录/合法性/求值/UI）由 `K: NodeTreeType` 树类型承担。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "K: Serialize", deserialize = "K: Deserialize<'de>"))]
pub struct NodeGraph<K: NodeTreeType = NodeKind> {
    pub id: NodeGraphId,
    pub name: String,
    pub enabled: bool,
    pub nodes: HashMap<NodeId, Node<K>>,
    pub sockets: HashMap<SocketId, Socket>,
    pub links: Vec<Link>,
    input_link_map: HashMap<SocketId, SocketId>,
    output_link_map: HashMap<SocketId, Vec<SocketId>>,
    #[serde(skip, default)]
    pub(crate) cache: Option<EvalCache>,
    #[serde(skip, default)]
    pub dirty: HashSet<NodeId>,
}

impl<K: NodeTreeType> NodeGraph<K> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: NodeGraphId::new(),
            name: name.into(),
            enabled: true,
            nodes: HashMap::new(),
            sockets: HashMap::new(),
            links: Vec::new(),
            input_link_map: HashMap::new(),
            output_link_map: HashMap::new(),
            cache: None,
            dirty: HashSet::new(),
        }
    }

    // ── 构建 ───────────────────────────────────────────────────────────────

    pub fn add_node<T: FromNode<K>>(
        &mut self,
        kind: K,
        extra_inputs: &[SocketDecl],
        extra_outputs: &[SocketDecl],
    ) -> Result<T, EvalError> {
        kind.validate_add(self)?;

        if kind.is_static() && (!extra_inputs.is_empty() || !extra_outputs.is_empty()) {
            return Err(EvalError::StaticNodeExtras);
        }

        let node_id = NodeId::new();
        let (input_decls, output_decls) = T::resolve_decls(&kind, extra_inputs, extra_outputs);

        let inputs: Vec<SocketId> = input_decls
            .iter()
            .map(|decl| {
                let sid = SocketId::new();
                self.sockets.insert(
                    sid,
                    Socket {
                        id: sid,
                        node: node_id,
                        data_type: decl.data_type,
                        name: decl.name.clone(),
                        default_value: decl.default_value.clone(),
                    },
                );
                sid
            })
            .collect();

        let outputs: Vec<SocketId> = output_decls
            .iter()
            .map(|decl| {
                let sid = SocketId::new();
                self.sockets.insert(
                    sid,
                    Socket {
                        id: sid,
                        node: node_id,
                        data_type: decl.data_type,
                        name: decl.name.clone(),
                        default_value: None,
                    },
                );
                sid
            })
            .collect();

        self.nodes.insert(
            node_id,
            Node {
                id: node_id,
                kind,
                inputs,
                outputs,
            },
        );
        self.dirty.insert(node_id);
        Ok(T::from_node(node_id, self))
    }

    // ── 连线 ───────────────────────────────────────────────────────────────

    /// 运行时连线。适合动态场景，不需要提前知道 socket 类型。
    pub fn connect_raw(&mut self, from: SocketId, to: SocketId) -> Result<(), EvalError> {
        let from_type = self.socket(from).data_type;
        let to_type = self.socket(to).data_type;
        if from_type != to_type {
            return Err(EvalError::TypeMismatch {
                expected: from_type,
                got: to_type,
            });
        }

        if let Some(old_from) = self.input_link_map.remove(&to) {
            if let Some(outputs) = self.output_link_map.get_mut(&old_from) {
                outputs.retain(|&s| s != to);
            }
            self.links.retain(|l| l.to != to);
        }

        self.links.push(Link { from, to });
        self.input_link_map.insert(to, from);
        self.output_link_map.entry(from).or_default().push(to);

        let dest_node = self.socket(to).node;
        self.mark_dirty(dest_node);
        Ok(())
    }

    /// 类型安全的连线。编译期保证两端类型相同。
    pub fn connect<T>(&mut self, from: SocketRef<T>, to: SocketRef<T>) -> Result<(), EvalError> {
        self.connect_raw(from.id, to.id)
    }

    // ── 断线 ───────────────────────────────────────────────────────────────

    pub fn disconnect(&mut self, from: SocketId, to: SocketId) {
        if self.input_link_map.get(&to) != Some(&from) {
            return;
        }
        self.input_link_map.remove(&to);
        self.links.retain(|l| l.from != from || l.to != to);
        if let Some(outputs) = self.output_link_map.get_mut(&from) {
            outputs.retain(|&s| s != to);
        }
        let dest_node = self.socket(to).node;
        self.mark_dirty(dest_node);
    }

    pub fn disconnect_input(&mut self, input_socket_id: SocketId) {
        if let Some(from) = self.input_link_map.remove(&input_socket_id) {
            self.links.retain(|l| l.to != input_socket_id);
            if let Some(outputs) = self.output_link_map.get_mut(&from) {
                outputs.retain(|&s| s != input_socket_id);
            }
            let dest_node = self.socket(input_socket_id).node;
            self.mark_dirty(dest_node);
        }
    }

    pub fn disconnect_output(&mut self, output_socket_id: SocketId) {
        if let Some(tos) = self.output_link_map.remove(&output_socket_id) {
            for &to in &tos {
                self.input_link_map.remove(&to);
                self.links.retain(|l| l.to != to);
                let dest_node = self.socket(to).node;
                self.mark_dirty(dest_node);
            }
        }
    }

    pub fn disconnect_node_inputs(&mut self, node_id: NodeId) {
        if let Some(node) = self.nodes.get(&node_id) {
            let inputs: Vec<SocketId> = node.inputs.clone();
            for input_id in inputs {
                self.disconnect_input(input_id);
            }
        }
    }

    pub fn disconnect_node_outputs(&mut self, node_id: NodeId) {
        if let Some(node) = self.nodes.get(&node_id) {
            let outputs: Vec<SocketId> = node.outputs.clone();
            for output_id in outputs {
                self.disconnect_output(output_id);
            }
        }
    }

    pub fn disconnect_node_all(&mut self, node_id: NodeId) {
        self.disconnect_node_outputs(node_id);
        self.disconnect_node_inputs(node_id);
    }

    pub(crate) fn socket(&self, socket_id: SocketId) -> &Socket {
        self.sockets
            .get(&socket_id)
            .unwrap_or_else(|| panic!("socket {socket_id} not found in graph {}", self.id))
    }

    pub(crate) fn input_source(&self, socket_id: SocketId) -> Option<SocketId> {
        self.input_link_map.get(&socket_id).copied()
    }

    pub fn output_targets(&self, socket_id: SocketId) -> Option<&[SocketId]> {
        self.output_link_map.get(&socket_id).map(|v| v.as_slice())
    }

    pub(crate) fn upstream_node_id(&self, socket_id: SocketId) -> Option<NodeId> {
        self.input_source(socket_id)
            .map(|from| self.socket(from).node)
    }

    pub(crate) fn read_input(
        &self,
        node: &Node<K>,
        idx: usize,
        socket_values: &HashMap<SocketId, SocketValue>,
    ) -> Option<SocketValue> {
        let &sid = node.inputs.get(idx)?;
        if let Some(from) = self.input_source(sid) {
            if let Some(val) = socket_values.get(&from) {
                return Some(val.clone());
            }
            return self.socket(from).default_value.clone();
        }
        self.socket(sid).default_value.clone()
    }

    // ── Socket 设值 ────────────────────────────────────────────────────────

    pub fn set_socket_value(
        &mut self,
        socket_id: SocketId,
        value: Option<SocketValue>,
    ) -> Result<(), EvalError> {
        let socket = self
            .sockets
            .get(&socket_id)
            .ok_or(EvalError::MissingSocket(socket_id))?;
        if let Some(ref v) = value {
            let expected = socket.data_type;
            let got = v.data_type();
            if got != expected {
                return Err(EvalError::TypeMismatch { expected, got });
            }
        }
        let node_id = socket.node;
        self.sockets.get_mut(&socket_id).unwrap().default_value = value;
        self.mark_dirty(node_id);
        Ok(())
    }

    pub fn set_value<T: InputSocket>(
        &mut self,
        socket: SocketRef<T>,
        value: T::Value,
    ) -> Result<(), EvalError> {
        self.set_socket_value(socket.id, Some(T::to_socket_value(value)))
    }

    pub fn clear_value<T>(&mut self, socket: SocketRef<T>) -> Result<(), EvalError> {
        self.set_socket_value(socket.id, None)
    }

    // ── 引用链设置 ─────────────────────────────────────────────────────────

    /// 原位改引用节点的引用链（仅改 kind 内嵌 id + 标脏，不删节点）。
    pub fn set_node_reference(
        &mut self,
        node_id: NodeId,
        new_id: NodeGraphId,
    ) -> Result<(), EvalError> {
        {
            let node = self
                .nodes
                .get_mut(&node_id)
                .ok_or(EvalError::MissingNode(node_id))?;
            node.kind.set_reference_id(new_id)?;
        }
        self.mark_dirty(node_id);
        Ok(())
    }

    // ── 删除节点 ───────────────────────────────────────────────────────────

    pub fn remove_node(&mut self, node_id: NodeId) {
        self.disconnect_node_all(node_id);

        if let Some(node) = self.nodes.remove(&node_id) {
            for input_id in &node.inputs {
                self.sockets.remove(input_id);
            }
            for output_id in &node.outputs {
                self.sockets.remove(output_id);
            }
        }

        if let Some(cache) = &mut self.cache {
            cache.node_styles.remove(&node_id);
            cache
                .socket_values
                .retain(|sid, _| self.sockets.contains_key(sid));
            cache.node_applications.remove(&node_id);
        }
        self.dirty.remove(&node_id);
    }

    // ── 脏标记 ─────────────────────────────────────────────────────────────

    fn mark_dirty(&mut self, node_id: NodeId) {
        self.propagate_dirty(node_id);
    }

    fn propagate_dirty(&mut self, node_id: NodeId) {
        if !self.dirty.insert(node_id) {
            return;
        }
        let downstream: Vec<NodeId> = self
            .nodes
            .get(&node_id)
            .map(|node| {
                node.outputs
                    .iter()
                    .flat_map(|&sid| {
                        self.output_link_map
                            .get(&sid)
                            .into_iter()
                            .flatten()
                            .filter_map(|&to_sid| self.sockets.get(&to_sid).map(|s| s.node))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for down in downstream {
            self.propagate_dirty(down);
        }
    }

    // ── 求值 ───────────────────────────────────────────────────────────────

    /// 求值：遍历脏节点（拓扑序），再按树类型收集图级结果。
    pub fn evaluate(&mut self, ctx: &EvalContext) -> Result<K::Result, EvalError> {
        if self.cache.is_none() {
            self.cache = Some(EvalCache::default());
            if self.dirty.is_empty() {
                self.dirty = self.nodes.keys().copied().collect();
            }
        }

        let mut cache = self.cache.take().unwrap();
        let dirty_nodes: Vec<NodeId> = self.dirty.iter().copied().collect();
        let mut visiting = HashSet::new();

        for node_id in dirty_nodes {
            self.ensure_node_evaluated(node_id, &mut cache, &mut visiting, ctx)?;
        }

        let result = K::collect_result(self, &cache)?;
        self.cache = Some(cache);
        Ok(result)
    }

    fn ensure_node_evaluated(
        &mut self,
        node_id: NodeId,
        cache: &mut EvalCache,
        visiting: &mut HashSet<NodeId>,
        ctx: &EvalContext,
    ) -> Result<(), EvalError> {
        if !self.dirty.contains(&node_id) {
            return Ok(());
        }

        if !visiting.insert(node_id) {
            return Err(EvalError::CyclicGraph(self.id));
        }

        let upstream_nodes: Vec<NodeId> = self.nodes[&node_id]
            .inputs
            .iter()
            .filter_map(|&input_id| {
                self.input_source(input_id)
                    .map(|from| self.socket(from).node)
            })
            .collect();

        for upstream_node in upstream_nodes {
            self.ensure_node_evaluated(upstream_node, cache, visiting, ctx)?;
        }

        self.evaluate_node(node_id, cache, ctx)?;

        visiting.remove(&node_id);
        self.dirty.remove(&node_id);
        Ok(())
    }

    fn evaluate_node(
        &self,
        node_id: NodeId,
        cache: &mut EvalCache,
        ctx: &EvalContext,
    ) -> Result<(), EvalError> {
        let node = &self.nodes[&node_id];
        node.kind.evaluate_node(self, node_id, cache, ctx)
    }
}

// ── 样式图树类型注册（NodeKind） ───────────────────────────────────────────────

impl NodeTreeType for NodeKind {
    type Result = StyleResult;

    fn validate_add(&self, graph: &NodeGraph<Self>) -> Result<(), EvalError> {
        // 样式图只允许一个 Viewer。
        if matches!(self, NodeKind::Viewer)
            && graph
                .nodes
                .values()
                .any(|n| matches!(n.kind, NodeKind::Viewer))
        {
            return Err(EvalError::DuplicateViewer);
        }
        Ok(())
    }

    fn is_static(&self) -> bool {
        NodeKind::is_static(self)
    }

    fn socket_decls(&self) -> Option<(Vec<SocketDecl>, Vec<SocketDecl>)> {
        get_socket_decls(self)
    }

    fn evaluate_node(
        &self,
        graph: &NodeGraph<Self>,
        node_id: NodeId,
        cache: &mut EvalCache,
        _ctx: &EvalContext,
    ) -> Result<(), EvalError> {
        let node = &graph.nodes[&node_id];
        match &node.kind {
            NodeKind::StyleSource => {
                cache.node_styles.insert(node_id, StyleResult::default());
            }

            kind @ NodeKind::VecOp(_) => {
                eval_vec_op(graph, node, kind, &mut cache.socket_values)?;
            }

            kind @ NodeKind::StyleOp(_) => {
                let style = graph
                    .upstream_node_id(node.inputs[0])
                    .and_then(|up| cache.node_styles.get(&up).cloned())
                    .unwrap_or_default();
                let result = apply_style_op(graph, node, kind, style, &cache.socket_values)?;
                cache.node_styles.insert(node_id, result);
            }

            NodeKind::MergeStyle => {
                let upstream_ids: Vec<NodeId> = node
                    .inputs
                    .iter()
                    .filter_map(|&sid| graph.upstream_node_id(sid))
                    .collect();
                let merged = upstream_ids
                    .iter()
                    .filter_map(|up| cache.node_styles.get(up).cloned())
                    .fold(StyleResult::default(), |a, b| a.merge_over(b));
                cache.node_styles.insert(node_id, merged);
            }

            NodeKind::GroupOutput | NodeKind::Viewer => {
                let result = node
                    .inputs
                    .first()
                    .and_then(|&sid| graph.upstream_node_id(sid))
                    .and_then(|up| cache.node_styles.get(&up).cloned())
                    .unwrap_or_default();
                cache.node_styles.insert(node_id, result);
            }

            NodeKind::GroupInput | NodeKind::Reference(_) => {}
        }
        Ok(())
    }

    fn collect_result(
        graph: &NodeGraph<Self>,
        cache: &EvalCache,
    ) -> Result<Self::Result, EvalError> {
        let result = if let Some(viewer_id) = graph
            .nodes
            .iter()
            .find(|(_, n)| matches!(n.kind, NodeKind::Viewer))
            .map(|(&id, _)| id)
        {
            cache.node_styles.get(&viewer_id).cloned()
        } else if let Some(output_id) = graph
            .nodes
            .iter()
            .find(|(_, n)| matches!(n.kind, NodeKind::GroupOutput))
            .map(|(&id, _)| id)
        {
            cache.node_styles.get(&output_id).cloned()
        } else {
            Some(StyleResult::default())
        };
        Ok(result.unwrap_or_default())
    }

    fn reference_id(&self) -> Option<NodeGraphId> {
        match self {
            NodeKind::Reference(id) => Some(*id),
            _ => None,
        }
    }

    fn set_reference_id(&mut self, new_id: NodeGraphId) -> Result<(), EvalError> {
        match self {
            NodeKind::Reference(id) => {
                *id = new_id;
                Ok(())
            }
            _ => Err(EvalError::NotReferenceNode),
        }
    }
}

fn eval_vec_op(
    graph: &NodeGraph<NodeKind>,
    node: &Node<NodeKind>,
    kind: &NodeKind,
    socket_values: &mut HashMap<SocketId, SocketValue>,
) -> Result<(), EvalError> {
    let NodeKind::VecOp(op) = kind else {
        unreachable!()
    };
    match op {
        VecOpKind::SeparateVec2 => {
            if let Some(SocketValue::Vec2 { x, y }) = graph.read_input(node, 0, socket_values) {
                if let Some(&sid) = node.outputs.first() {
                    socket_values.insert(sid, SocketValue::Float(x));
                }
                if let Some(&sid) = node.outputs.get(1) {
                    socket_values.insert(sid, SocketValue::Float(y));
                }
            }
        }
        VecOpKind::CombineVec2 => {
            let x = graph
                .read_input(node, 0, socket_values)
                .and_then(|v| {
                    if let SocketValue::Float(f) = v {
                        Some(f)
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0);
            let y = graph
                .read_input(node, 1, socket_values)
                .and_then(|v| {
                    if let SocketValue::Float(f) = v {
                        Some(f)
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0);
            if let Some(&sid) = node.outputs.first() {
                socket_values.insert(sid, SocketValue::Vec2 { x, y });
            }
        }
        VecOpKind::SeparateVec3 => {
            if let Some(SocketValue::Vec3 { x, y, z }) = graph.read_input(node, 0, socket_values) {
                for (i, val) in [x, y, z].into_iter().enumerate() {
                    if let Some(&sid) = node.outputs.get(i) {
                        socket_values.insert(sid, SocketValue::Float(val));
                    }
                }
            }
        }
        VecOpKind::CombineVec3 => {
            let get_f = |idx: usize, sv: &HashMap<SocketId, SocketValue>| -> f64 {
                graph
                    .read_input(node, idx, sv)
                    .and_then(|v| {
                        if let SocketValue::Float(f) = v {
                            Some(f)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0)
            };
            let (x, y, z) = (
                get_f(0, socket_values),
                get_f(1, socket_values),
                get_f(2, socket_values),
            );
            if let Some(&sid) = node.outputs.first() {
                socket_values.insert(sid, SocketValue::Vec3 { x, y, z });
            }
        }
    }
    Ok(())
}

fn apply_style_op(
    graph: &NodeGraph<NodeKind>,
    node: &Node<NodeKind>,
    kind: &NodeKind,
    mut style: StyleResult,
    socket_values: &HashMap<SocketId, SocketValue>,
) -> Result<StyleResult, EvalError> {
    let NodeKind::StyleOp(op) = kind else {
        unreachable!()
    };

    let read = |idx: usize| -> Option<SocketValue> { graph.read_input(node, idx, socket_values) };

    let read_f64 = |idx: usize| -> Option<f64> {
        match read(idx) {
            Some(SocketValue::Float(f)) => Some(f),
            _ => None,
        }
    };
    let read_bool = |idx: usize| -> Option<bool> {
        match read(idx) {
            Some(SocketValue::Bool(b)) => Some(b),
            _ => None,
        }
    };
    let read_text = |idx: usize| -> Option<String> {
        match read(idx) {
            Some(SocketValue::Text(t)) => Some(t),
            _ => None,
        }
    };
    let read_color = |idx: usize| -> Option<Srgba> {
        match read(idx) {
            Some(SocketValue::Color(c)) => Some(c),
            _ => None,
        }
    };
    let read_vec2 = |idx: usize| -> Option<Vec2> {
        match read(idx) {
            Some(SocketValue::Vec2 { x, y }) => Some(Vec2 { x, y }),
            _ => None,
        }
    };
    let read_vec3 = |idx: usize| -> Option<Vec3> {
        match read(idx) {
            Some(SocketValue::Vec3 { x, y, z }) => Some(Vec3 { x, y, z }),
            _ => None,
        }
    };
    let read_int = |idx: usize| -> Option<u32> {
        match read(idx) {
            Some(SocketValue::Int(i)) => Some(i as u32),
            _ => None,
        }
    };

    match op {
        StyleOpKind::SetFontName => style.font_name = read_text(1),
        StyleOpKind::SetFontSize => style.font_size = read_f64(1),
        StyleOpKind::SetBold => style.bold = read_bool(1),
        StyleOpKind::SetItalic => style.italic = read_bool(1),
        StyleOpKind::SetUnderline => style.underline = read_bool(1),
        StyleOpKind::SetStrikeout => style.strikeout = read_bool(1),

        StyleOpKind::SetColor(role) => {
            if let Some(c) = read_color(1) {
                match role {
                    ColorRole::Primary => style.color_primary = Some(c),
                    ColorRole::Secondary => style.color_secondary = Some(c),
                    ColorRole::Outline => style.color_outline = Some(c),
                    ColorRole::Back => style.color_back = Some(c),
                    ColorRole::All => {
                        style.color_primary = Some(c);
                        style.color_secondary = Some(c);
                        style.color_outline = Some(c);
                        style.color_back = Some(c);
                    }
                }
            }
        }

        StyleOpKind::SetAlpha(role) => {
            if let Some(alpha) = read_f64(1) {
                let set = |c: &mut Option<Srgba>| {
                    let base = c.take().unwrap_or_default();
                    *c = Some(Srgba::new(
                        base.color.red,
                        base.color.green,
                        base.color.blue,
                        alpha as f32,
                    ));
                };
                match role {
                    ColorRole::Primary => set(&mut style.color_primary),
                    ColorRole::Secondary => set(&mut style.color_secondary),
                    ColorRole::Outline => set(&mut style.color_outline),
                    ColorRole::Back => set(&mut style.color_back),
                    ColorRole::All => {
                        set(&mut style.color_primary);
                        set(&mut style.color_secondary);
                        set(&mut style.color_outline);
                        set(&mut style.color_back);
                    }
                }
            }
        }

        StyleOpKind::SetBorder => style.border = read_vec2(1),
        StyleOpKind::SetShadow => style.shadow = read_vec2(1),
        StyleOpKind::SetFontScale => style.font_scale = read_vec2(1),
        StyleOpKind::SetFontRotation => style.font_rotation = read_vec3(1),
        StyleOpKind::SetFactor => style.factor = read_vec2(1),
        StyleOpKind::SetBlur => style.blur = read_f64(1),
        StyleOpKind::SetBlurEdges => style.blur_edges = read_int(1),
        StyleOpKind::SetPosition => style.position = read_vec2(1),
        StyleOpKind::SetOrigin => style.origin = read_vec2(1),
        StyleOpKind::SetClipRect => {
            style.clip_rect = match read(1) {
                Some(SocketValue::ClipRect(r)) => Some(r.normalized()),
                _ => None,
            };
        }
    }
    Ok(style)
}
