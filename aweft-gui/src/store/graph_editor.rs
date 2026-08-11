//! 节点图编辑器 Rust 侧：持有编辑器视图模型（节点/连线/位置）与所有图操作。
//!
//! 图结构数据（nodes/sockets/links）权威在 `aweft-model`；本模块只保存 UI 数据
//! （节点位置、模型下标顺序）并生成 Slint 的节点/连线视图模型。
//! 端口命中、连线重连等"遍历"逻辑原先在 Slint 内硬编码逐槽位展开，现统一在此用循环实现。

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use aweft_model::{
    application_graph::{ApplicationGraph, ApplicationGraphNode, ApplicationNodeKind},
    ids::{NodeGraphId, NodeId, SocketId},
    node_graph::{
        get_socket_decls, error::EvalError, ColorRole, GraphNode as ModelGraphNode, Link, NodeGraph,
        NodeKind, Socket, SocketDecl, SocketType, StyleOpKind,
    },
};
use slint::{Color, Model, ModelRc, SharedString, VecModel};

use crate::{
    bridge::types::{ContextMenuItem, GraphLink, GraphNode, NodeSocketData, NodeSocketType, SocketHit},
    store::AppStore,
};

// ── 端口命中/布局所需的主题量 ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
pub struct EditorMetrics {
    pub header: f32, // 节点标题栏高度
    pub row: f32,    // 端口行高
    pub radius: f32, // 端口命中半径（= socket-size）
}

// ── 视图模型 ──────────────────────────────────────────────────────────────────

/// 一个节点图类型的编辑器状态。
pub struct GraphEditorState {
    /// 节点视图模型（title/x/y/sockets，含 UI 位置数据）
    pub nodes: Rc<VecModel<GraphNode>>,
    /// 连线视图模型（端口均以 nodes 模型下标引用）
    pub links: Rc<VecModel<GraphLink>>,
    /// 画布右键菜单项（增加节点）
    pub menu_items: Rc<VecModel<ContextMenuItem>>,
    /// 链切换菜单项（对应 style_order / 应用链列表）
    pub chain_items: Rc<VecModel<ContextMenuItem>>,
    /// 当前编辑的链：
    ///  - 样式链：`style_order[current_chain]` 对应 `Project.library.node_graphs` 里的链（按 id 引用）
    ///  - 应用链：直接是 `Project.style_layer.graphs[current_chain]`（Vec 下标，即优先级序）
    pub current_chain: usize,
    /// 样式链显示顺序（UI 数据：模型侧为 id-keyed map，链切换器的顺序在这里派生）
    pub style_order: Vec<NodeGraphId>,
    /// 样式链名字（与 style_order 一一对应，用于样式引用节点下拉菜单）
    pub style_chain_names: Vec<String>,
    /// 角色名字列表（按名排序；匹配角色 pattern 的下拉候选，含"新建角色"尾项）
    pub actor_names: Vec<String>,
    /// 节点位置（UI 数据，世界坐标，逻辑像素）
    positions: HashMap<NodeId, (f32, f32)>,
    /// 节点模型顺序：下标 → NodeId（增删节点后重建）
    order: Vec<NodeId>,
    pub metrics: EditorMetrics,
}

impl Default for GraphEditorState {
    fn default() -> Self {
        Self {
            nodes: Rc::new(VecModel::default()),
            links: Rc::new(VecModel::default()),
            menu_items: Rc::new(VecModel::default()),
            chain_items: Rc::new(VecModel::default()),
            current_chain: 0,
            style_order: Vec::new(),
            style_chain_names: Vec::new(),
            actor_names: Vec::new(),
            positions: HashMap::new(),
            order: Vec::new(),
            metrics: EditorMetrics::default(),
        }
    }
}

impl GraphEditorState {
    /// 节点位置：有记录用记录，否则自动排布（横向平铺）
    fn pos(&self, id: NodeId, idx: usize) -> (f32, f32) {
        self.positions
            .get(&id)
            .copied()
            .unwrap_or((40.0 + idx as f32 * 220.0, 40.0))
    }

    /// 把图的当前结构同步到视图模型（顺序只增不减：已有节点不动，新节点追加，删掉的移除，
    /// 保证模型下标稳定，不随连线变化漂移）。
    /// 无位置记录的节点按"上游深度"自动布局（源在左、汇在右，同深度纵向堆叠）——
    /// 位置直接由图结构推导，与迭代顺序无关，因此默认布局确定且正确（样式源在最左、组输出在最右）。
    /// 位置一旦分配即持久化（UI 数据），之后拖拽/重连都不会让节点跳位。
    pub fn sync<G: GraphEditable>(&mut self, g: &G) {
        self.order.retain(|id| g.nodes().contains_key(id));
        for id in g.nodes().keys() {
            if !self.order.contains(id) {
                self.order.push(*id);
            }
        }
        let mut cache = HashMap::new();
        let mut visiting = HashSet::new();
        let mut depth_rows = HashMap::new();
        for &id in &self.order {
            self.positions.entry(id).or_insert_with(|| {
                let d = flow_depth(g, id, &mut cache, &mut visiting);
                let y = 40.0 + depth_rows.get(&d).copied().unwrap_or(0) as f32 * 60.0;
                *depth_rows.entry(d).or_insert(0) += 1;
                (40.0 + d as f32 * 220.0, y)
            });
        }

        let rows: Vec<GraphNode> = self
            .order
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                let n = g.node(id);
                let (x, y) = self.pos(id, i);
                // 端口全局行号：输入从顶部先排（0..），输出接着输入往下排
                // （输入在上、输出在下）
                let in_count = G::inputs(n).len();
                let mut sockets = Vec::with_capacity(in_count + G::outputs(n).len());
                for (row, &sid) in G::inputs(n).iter().enumerate() {
                    let s = g.socket(sid);
                    let connected = g.links().iter().any(|l| l.to == sid);
                    let is_actor = G::actor_dropdown(n, true, row);
                    let selected = if is_actor {
                        match &s.default_value {
                            Some(aweft_model::node_graph::SocketValue::Text(t)) => self
                                .actor_names
                                .iter()
                                .position(|a| a == t)
                                .map_or(-1, |i| i as i32),
                            _ => -1,
                        }
                    } else {
                        -1
                    };
                    sockets.push(NodeSocketData {
                        name: s.name.clone().into(),
                        is_input: true,
                        data_type: socket_type_to_slint(s.data_type),
                        row: row as i32,
                        menu_items: if is_actor {
                            actor_menu_items(&self.actor_names)
                        } else {
                            ModelRc::from(Rc::new(VecModel::default()))
                        },
                        selected,
                        connected,
                        input: false,
                        input_text: SharedString::default(),
                    });
                }
                for (row, &sid) in G::outputs(n).iter().enumerate() {
                    let s = g.socket(sid);
                    let connected = g.links().iter().any(|l| l.from == sid);
                    let is_editor = G::text_editor(n, false, row);
                    let input_text = if is_editor {
                        match &s.default_value {
                            Some(aweft_model::node_graph::SocketValue::Text(t)) => t.clone().into(),
                            _ => SharedString::default(),
                        }
                    } else {
                        SharedString::default()
                    };
                    sockets.push(NodeSocketData {
                        name: s.name.clone().into(),
                        is_input: false,
                        data_type: socket_type_to_slint(s.data_type),
                        row: in_count as i32 + row as i32,
                        menu_items: ModelRc::from(Rc::new(VecModel::default())),
                        selected: -1,
                        connected,
                        input: is_editor,
                        input_text,
                    });
                }
                // 样式引用节点：生成下拉菜单行数据（标签 + 样式链选项 + 当前选中）
                let (menu_label, menu_options, menu_selected) = match G::reference_chain_id(n) {
                    Some(ref_id) => {
                        let selected = self
                            .style_order
                            .iter()
                            .position(|&x| x == ref_id)
                            .map_or(-1, |i| i as i32);
                        let opts: Vec<SharedString> = self
                            .style_chain_names
                            .iter()
                            .cloned()
                            .map(SharedString::from)
                            .collect();
                        (
                            SharedString::from("样式链"),
                            ModelRc::from(Rc::new(VecModel::from(opts))),
                            selected,
                        )
                    }
                    None => (
                        SharedString::default(),
                        ModelRc::from(Rc::new(VecModel::default())),
                        0,
                    ),
                };
                GraphNode {
                    title: g.title(n).into(),
                    x,
                    y,
                    sockets: Rc::new(VecModel::from(sockets)).into(),
                    socket_rows: (G::inputs(n).len() + G::outputs(n).len()) as i32,
                    menu_label,
                    menu_options,
                    menu_selected,
                }
            })
            .collect();
        self.nodes.set_vec(rows);

        let links: Vec<GraphLink> = g
            .links()
            .iter()
            .filter_map(|l| {
                let fi = self.order.iter().position(|&id| id == g.socket(l.from).node)?;
                let ti = self.order.iter().position(|&id| id == g.socket(l.to).node)?;
                // 端口下标 = 全局行号（输入先排 0..，输出接着排 inputs.len()..）：
                // 输入端下标即 inputs 内下标；输出端加输入端口数量。
                let fs = G::inputs(g.node(self.order[fi])).len()
                    + G::outputs(g.node(self.order[fi]))
                        .iter()
                        .position(|&s| s == l.from)?;
                let ts = G::inputs(g.node(self.order[ti])).iter().position(|&s| s == l.to)?;
                Some(GraphLink {
                    from_node: fi as i32,
                    from_socket: fs as i32,
                    to_node: ti as i32,
                    to_socket: ts as i32,
                    data_type: socket_type_to_slint(g.socket(l.from).data_type),
                })
            })
            .collect();
        self.links.set_vec(links);
    }

    pub fn id_at(&self, index: usize) -> Option<NodeId> {
        self.order.get(index).copied()
    }

    /// 更新节点位置（仅 UI 数据，不改图结构）
    pub fn set_node_position(&mut self, index: usize, x: f32, y: f32) {
        if let Some(&id) = self.order.get(index) {
            self.positions.insert(id, (x, y));
            if let Some(mut row) = self.nodes.row_data(index) {
                row.x = x;
                row.y = y;
                self.nodes.set_row_data(index, row);
            }
        }
    }
}

// ── 图类型适配 ────────────────────────────────────────────────────────────────

/// 让两种图（NodeGraph / ApplicationGraph）都能喂给通用编辑器逻辑。
/// 全部通过公开字段访问模型数据，实现里仅做类型化转发。
pub trait GraphEditable: Sized {
    type Node;
    type Kind: Clone;

    fn nodes(&self) -> &HashMap<NodeId, Self::Node>;
    fn sockets(&self) -> &HashMap<SocketId, Socket>;
    fn links(&self) -> &[Link];
    fn node(&self, id: NodeId) -> &Self::Node;
    fn socket(&self, id: SocketId) -> &Socket;
    fn graph_add_node(&mut self, kind: Self::Kind) -> Result<NodeId, EvalError>;
    fn graph_connect(&mut self, from: SocketId, to: SocketId) -> Result<(), EvalError>;
    fn graph_disconnect_input(&mut self, sid: SocketId);
    fn graph_delete(&mut self, id: NodeId);
    /// 节点显示名
    fn title(&self, n: &Self::Node) -> &'static str;
    fn inputs(n: &Self::Node) -> &[SocketId];
    fn outputs(n: &Self::Node) -> &[SocketId];
    /// 节点类型的 socket 声明（用于菜单色点）
    fn kind_decls(kind: &Self::Kind) -> (Vec<SocketDecl>, Vec<SocketDecl>);
    /// 样式引用节点（Reference / AppRef）引用的样式链 id；非引用节点返回 None。
    fn reference_chain_id(n: &Self::Node) -> Option<NodeGraphId>;
    /// 该端口是否需要"角色下拉"（候选 = 当前所有 actor，含"新建角色"项）；
    /// 如匹配角色的 pattern 输入。idx 为端口在输入/输出列内的下标。
    fn actor_dropdown(n: &Self::Node, is_input: bool, idx: usize) -> bool;
    /// 该端口行是否内嵌文本输入框（如字符串常量的输出端口）。
    fn text_editor(n: &Self::Node, is_input: bool, idx: usize) -> bool;
}

impl GraphEditable for NodeGraph {
    type Node = ModelGraphNode;
    type Kind = NodeKind;

    fn nodes(&self) -> &HashMap<NodeId, ModelGraphNode> {
        &self.nodes
    }
    fn sockets(&self) -> &HashMap<SocketId, Socket> {
        &self.sockets
    }
    fn links(&self) -> &[Link] {
        &self.links
    }
    fn node(&self, id: NodeId) -> &ModelGraphNode {
        &self.nodes[&id]
    }
    fn socket(&self, id: SocketId) -> &Socket {
        &self.sockets[&id]
    }
    fn graph_add_node(&mut self, kind: NodeKind) -> Result<NodeId, EvalError> {
        NodeGraph::add_node(self, kind, &[], &[])
    }
    fn graph_connect(&mut self, from: SocketId, to: SocketId) -> Result<(), EvalError> {
        NodeGraph::connect_raw(self, from, to)
    }
    fn graph_disconnect_input(&mut self, sid: SocketId) {
        NodeGraph::disconnect_input(self, sid);
    }
    fn graph_delete(&mut self, id: NodeId) {
        NodeGraph::remove_node(self, id);
    }
    fn title(&self, n: &ModelGraphNode) -> &'static str {
        n.kind.title()
    }
    fn inputs(n: &ModelGraphNode) -> &[SocketId] {
        &n.inputs
    }
    fn outputs(n: &ModelGraphNode) -> &[SocketId] {
        &n.outputs
    }
    fn kind_decls(kind: &NodeKind) -> (Vec<SocketDecl>, Vec<SocketDecl>) {
        get_socket_decls(kind).unwrap_or_default()
    }
    fn reference_chain_id(n: &ModelGraphNode) -> Option<NodeGraphId> {
        match n.kind {
            NodeKind::Reference(id) => Some(id),
            _ => None,
        }
    }
    fn actor_dropdown(_n: &ModelGraphNode, _is_input: bool, _idx: usize) -> bool {
        false
    }
    fn text_editor(_n: &ModelGraphNode, _is_input: bool, _idx: usize) -> bool {
        false
    }
}

impl GraphEditable for ApplicationGraph {
    type Node = ApplicationGraphNode;
    type Kind = ApplicationNodeKind;

    fn nodes(&self) -> &HashMap<NodeId, ApplicationGraphNode> {
        &self.nodes
    }
    fn sockets(&self) -> &HashMap<SocketId, Socket> {
        &self.sockets
    }
    fn links(&self) -> &[Link] {
        &self.links
    }
    fn node(&self, id: NodeId) -> &ApplicationGraphNode {
        &self.nodes[&id]
    }
    fn socket(&self, id: SocketId) -> &Socket {
        &self.sockets[&id]
    }
    fn graph_add_node(&mut self, kind: ApplicationNodeKind) -> Result<NodeId, EvalError> {
        ApplicationGraph::add_node(self, kind, &[], &[])
    }
    fn graph_connect(&mut self, from: SocketId, to: SocketId) -> Result<(), EvalError> {
        ApplicationGraph::connect_raw(self, from, to)
    }
    fn graph_disconnect_input(&mut self, sid: SocketId) {
        ApplicationGraph::disconnect_input(self, sid);
    }
    fn graph_delete(&mut self, id: NodeId) {
        ApplicationGraph::remove_node(self, id);
    }
    fn title(&self, n: &ApplicationGraphNode) -> &'static str {
        n.kind.title()
    }
    fn inputs(n: &ApplicationGraphNode) -> &[SocketId] {
        &n.inputs
    }
    fn outputs(n: &ApplicationGraphNode) -> &[SocketId] {
        &n.outputs
    }
    fn kind_decls(kind: &ApplicationNodeKind) -> (Vec<SocketDecl>, Vec<SocketDecl>) {
        aweft_model::application_graph::get_application_socket_decls(kind).unwrap_or_default()
    }
    fn reference_chain_id(n: &ApplicationGraphNode) -> Option<NodeGraphId> {
        match n.kind {
            ApplicationNodeKind::AppRef(id) => Some(id),
            _ => None,
        }
    }
    fn actor_dropdown(n: &ApplicationGraphNode, is_input: bool, idx: usize) -> bool {
        // 匹配角色的 pattern 输入（第 1 个输入）用角色下拉
        is_input && idx == 1 && matches!(n.kind, ApplicationNodeKind::MatchActor)
    }
    fn text_editor(n: &ApplicationGraphNode, is_input: bool, idx: usize) -> bool {
        // 字符串常量的输出端口：无输入，端口行内嵌文本输入框
        !is_input && idx == 0 && matches!(n.kind, ApplicationNodeKind::TextConst)
    }
}

// ── 图操作（原 Slint 逐槽位展开逻辑，现用循环） ───────────────────────────────

/// 上游深度：经输入连线可达的最深上游层数（源节点 = 0）。带记忆与环保护。
fn flow_depth<G: GraphEditable>(
    g: &G,
    id: NodeId,
    cache: &mut HashMap<NodeId, usize>,
    visiting: &mut HashSet<NodeId>,
) -> usize {
    if let Some(&d) = cache.get(&id) {
        return d;
    }
    if !visiting.insert(id) {
        return 0; // 有环：按叶子处理，避免无限递归
    }
    let n = g.node(id);
    let d = G::inputs(n)
        .iter()
        .filter_map(|&sid| g.links().iter().find(|l| l.to == sid).map(|l| g.socket(l.from).node))
        .map(|up| flow_depth(g, up, cache, visiting) + 1)
        .max()
        .unwrap_or(0);
    visiting.remove(&id);
    cache.insert(id, d);
    d
}

/// 角色下拉菜单项：当前所有 actor + 末尾"新建角色"。
/// 返回 (menu-items, 有效角色数)；末尾"新建角色"项由调用方识别。
fn actor_menu_items(actor_names: &[String]) -> ModelRc<ContextMenuItem> {
    let mut items: Vec<ContextMenuItem> = actor_names
        .iter()
        .map(|n| ContextMenuItem {
            text: n.clone().into(),
            enabled: true,
            dot_color: Color::from_argb_u8(0, 0, 0, 0),
            separator_before: false,
        })
        .collect();
    items.push(ContextMenuItem {
        text: "+ 新建角色".into(),
        enabled: true,
        dot_color: Color::from_argb_u8(0, 0, 0, 0),
        separator_before: !items.is_empty(),
    });
    ModelRc::from(Rc::new(VecModel::from(items)))
}

/// 创建默认样式链：样式源 -> 组输出（新链/测试数据用）。
pub fn new_style_chain(name: impl Into<String>) -> NodeGraph {
    let mut g = NodeGraph::new(name);
    let src = g.add_node::<NodeId>(NodeKind::StyleSource, &[], &[]).unwrap();
    let out = g.add_node::<NodeId>(NodeKind::GroupOutput, &[], &[]).unwrap();
    g.connect_raw(g.nodes[&src].outputs[0], g.nodes[&out].inputs[0])
        .unwrap();
    g
}

/// 创建默认应用链：上下文输入 -> 匹配角色 -> 应用匹配 -> 组输出。
/// 应用匹配的样式必须明确连接——默认链引用一条样式链（`style_ref`），
/// 若调用方没有可引用的样式链则传 `None`（用户随后手动连接样式来源）。
/// `default_actor`：匹配角色 pattern 的默认角色名（未连字符串节点时用它匹配）。
pub fn new_app_chain(
    name: impl Into<String>,
    style_ref: Option<NodeGraphId>,
    default_actor: Option<&str>,
) -> ApplicationGraph {
    let mut g = ApplicationGraph::new(name);
    let ctx = g.add_node::<NodeId>(ApplicationNodeKind::ContextInput, &[], &[]).unwrap();
    let m = g.add_node::<NodeId>(ApplicationNodeKind::MatchActor, &[], &[]).unwrap();
    let ap = g.add_node::<NodeId>(ApplicationNodeKind::ApplyMatch, &[], &[]).unwrap();
    let out = g
        .add_node::<NodeId>(
            ApplicationNodeKind::GroupOutput,
            &[SocketDecl::input("application", SocketType::Application)],
            &[],
        )
        .unwrap();
    g.connect_raw(g.nodes[&ctx].outputs[0], g.nodes[&m].inputs[0])
        .unwrap();
    // 匹配角色的 pattern（第 1 输入）默认角色（实际连接数据，非硬编码预设）
    if let Some(actor) = default_actor {
        g.set_socket_value(
            g.nodes[&m].inputs[1],
            Some(aweft_model::node_graph::SocketValue::Text(actor.into())),
        )
        .unwrap();
    }
    g.connect_raw(g.nodes[&m].outputs[0], g.nodes[&ap].inputs[1])
        .unwrap();
    if let Some(style_id) = style_ref {
        let appref = g
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(style_id),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        g.connect_raw(g.nodes[&appref].outputs[0], g.nodes[&ap].inputs[0])
            .unwrap();
    }
    g.connect_raw(g.nodes[&ap].outputs[0], g.nodes[&out].inputs[0])
        .unwrap();
    g
}

/// 添加节点，返回其在 nodes 模型中的下标。
pub fn add_node<G: GraphEditable>(
    g: &mut G,
    st: &mut GraphEditorState,
    kind: G::Kind,
    x: f32,
    y: f32,
) -> usize {
    let id = g.graph_add_node(kind).expect("添加节点失败");
    st.positions.insert(id, (x, y));
    st.sync(g);
    st.order.iter().position(|&n| n == id).unwrap_or(0)
}

/// 删除节点（自动断开相关连线）。
pub fn delete_node<G: GraphEditable>(g: &mut G, st: &mut GraphEditorState, index: usize) {
    if let Some(id) = st.id_at(index) {
        g.graph_delete(id);
        st.positions.remove(&id);
        st.sync(g);
    }
}

/// 找喂给某输入端口的连线（模型下标）；无则 -1。拖输入端时用它隐藏该线。
pub fn find_link_for_input<G: GraphEditable>(
    g: &G,
    st: &GraphEditorState,
    node_index: usize,
    sock: usize,
) -> i32 {
    let Some(id) = st.id_at(node_index) else {
        return -1;
    };
    // sock 为输入端口全局行号（输入在前，等于 inputs 内下标）
    let Some(&sid) = G::inputs(g.node(id)).get(sock) else {
        return -1;
    };
    g.links()
        .iter()
        .position(|l| l.to == sid)
        .map_or(-1, |i| i as i32)
}

/// 找拖拽释放位置命中的、类型匹配的输入端口（世界坐标，方形判定，半径= socket-size）。
pub fn find_drop_socket<G: GraphEditable>(
    g: &G,
    st: &GraphEditorState,
    x: f32,
    y: f32,
    want: SocketType,
) -> SocketHit {
    for (i, &id) in st.order.iter().enumerate() {
        let n = g.node(id);
        let (nx, ny) = st.pos(id, i);
        for (si, &sid) in G::inputs(n).iter().enumerate() {
            let s = g.socket(sid);
            if s.data_type != want {
                continue;
            }
            // 输入端口在前，全局行号 = 输入内下标
            let cy = ny + st.metrics.header + si as f32 * st.metrics.row + st.metrics.row / 2.0;
            if (x - nx).abs() < st.metrics.radius && (y - cy).abs() < st.metrics.radius {
                return SocketHit {
                    found: true,
                    node: i as i32,
                    socket: si as i32,
                };
            }
        }
    }
    SocketHit {
        found: false,
        node: 0,
        socket: 0,
    }
}

/// 端口拖拽松开：按拖拽语义完成断开/重连。
///  - 拖输入端：源 = 该输入当前连线的输出端；命中新输入则把线搬过去，否则断开。
///  - 拖输出端：命中输入则连接（该输入原有连线被覆盖），否则丢弃。
pub fn socket_drag_ended<G: GraphEditable>(
    g: &mut G,
    st: &mut GraphEditorState,
    node_index: usize,
    sock: usize,
    is_input: bool,
    x: f32,
    y: f32,
) {
    let Some(id) = st.id_at(node_index) else {
        return;
    };
    let n = g.node(id);
    // 上报的端口下标 = 全局行号（输入先排 0..，输出接着排 inputs.len()..）
    let dragged = if is_input {
        G::inputs(n).get(sock).copied()
    } else {
        G::outputs(n)
            .get(sock.saturating_sub(G::inputs(n).len()))
            .copied()
    };
    let Some(dragged_sid) = dragged else {
        return;
    };
    let want = g.socket(dragged_sid).data_type;
    let hit = find_drop_socket(g, st, x, y, want);

    if is_input {
        // 拖回原位 = 取消（什么都不做）
        if hit.found && hit.node as usize == node_index && hit.socket as usize == sock {
            return;
        }
        let source = g.links().iter().find(|l| l.to == dragged_sid).map(|l| l.from);
        g.graph_disconnect_input(dragged_sid);
        if hit.found {
            if let (Some(src), Some(target)) = (source, target_socket(g, st, &hit)) {
                let _ = g.graph_connect(src, target);
            }
        }
    } else if hit.found {
        if let Some(target) = target_socket(g, st, &hit) {
            let _ = g.graph_connect(dragged_sid, target);
        }
    }
    st.sync(g);
}

fn target_socket<G: GraphEditable>(g: &G, st: &GraphEditorState, hit: &SocketHit) -> Option<SocketId> {
    if !hit.found {
        return None;
    }
    let id = st.id_at(hit.node as usize)?;
    // hit.socket 为输入端口全局行号（输入在前，等于 inputs 内下标）
    G::inputs(g.node(id)).get(hit.socket as usize).copied()
}

// ── 右键菜单模板 ──────────────────────────────────────────────────────────────

fn style_templates() -> Vec<NodeKind> {
    vec![
        NodeKind::StyleSource,
        NodeKind::StyleOp(StyleOpKind::SetFontSize),
        NodeKind::StyleOp(StyleOpKind::SetBold),
        NodeKind::StyleOp(StyleOpKind::SetItalic),
        NodeKind::StyleOp(StyleOpKind::SetColor(ColorRole::Primary)),
        NodeKind::StyleOp(StyleOpKind::SetBorder),
        NodeKind::StyleOp(StyleOpKind::SetShadow),
        NodeKind::StyleOp(StyleOpKind::SetBlur),
        NodeKind::StyleOp(StyleOpKind::SetPosition),
        NodeKind::StyleOp(StyleOpKind::SetClipRect),
        NodeKind::StyleOp(StyleOpKind::SetFontRotation),
        NodeKind::MergeStyle,
        NodeKind::GroupOutput,
    ]
}

fn app_templates() -> Vec<ApplicationNodeKind> {
    vec![
        ApplicationNodeKind::ContextInput,
        ApplicationNodeKind::MatchActor,
        ApplicationNodeKind::MatchText,
        ApplicationNodeKind::IntEq,
        ApplicationNodeKind::And,
        ApplicationNodeKind::Or,
        ApplicationNodeKind::Not,
        ApplicationNodeKind::TextConst,
        ApplicationNodeKind::ApplyMatch,
        ApplicationNodeKind::ApplyAnimation,
        ApplicationNodeKind::GroupOutput,
    ]
}

// ── 类型转换 ──────────────────────────────────────────────────────────────────

pub fn socket_type_to_slint(t: SocketType) -> NodeSocketType {
    match t {
        SocketType::Style => NodeSocketType::Style,
        SocketType::Float => NodeSocketType::Float,
        SocketType::Bool => NodeSocketType::Bool,
        SocketType::Color => NodeSocketType::Color,
        SocketType::Text => NodeSocketType::Text,
        SocketType::Int => NodeSocketType::Int,
        SocketType::Vec2 => NodeSocketType::Vec2,
        SocketType::Vec3 => NodeSocketType::Vec3,
        SocketType::ClipRect => NodeSocketType::ClipRect,
        SocketType::Animation => NodeSocketType::Animation,
        SocketType::Application => NodeSocketType::Application,
    }
}

pub fn socket_type_from_slint(t: NodeSocketType) -> SocketType {
    match t {
        NodeSocketType::Style => SocketType::Style,
        NodeSocketType::Float => SocketType::Float,
        NodeSocketType::Bool => SocketType::Bool,
        NodeSocketType::Color => SocketType::Color,
        NodeSocketType::Text => SocketType::Text,
        NodeSocketType::Int => SocketType::Int,
        NodeSocketType::Vec2 => SocketType::Vec2,
        NodeSocketType::Vec3 => SocketType::Vec3,
        NodeSocketType::ClipRect => SocketType::ClipRect,
        NodeSocketType::Animation => SocketType::Animation,
        NodeSocketType::Application => SocketType::Application,
    }
}

// ── AppStore 入口（图操作分发到两种图） ───────────────────────────────────────

impl AppStore {
    fn with_style<R>(&mut self, f: impl FnOnce(&mut NodeGraph, &mut GraphEditorState) -> R) -> Option<R> {
        let AppStore { state, ui_models, .. } = self;
        let id = *ui_models.style_graph.style_order.get(ui_models.style_graph.current_chain)?;
        let graph = state.project.library.node_graphs.get_mut(&id)?;
        Some(f(graph, &mut ui_models.style_graph))
    }

    fn with_app<R>(
        &mut self,
        f: impl FnOnce(&mut ApplicationGraph, &mut GraphEditorState) -> R,
    ) -> Option<R> {
        let AppStore { state, ui_models, .. } = self;
        let idx = ui_models.app_graph.current_chain;
        let graph = state.project.style_layer.graphs.get_mut(idx)?;
        Some(f(graph, &mut ui_models.app_graph))
    }

    fn with_style_ro<R>(&self, f: impl FnOnce(&NodeGraph, &GraphEditorState) -> R) -> Option<R> {
        let AppStore { state, ui_models, .. } = self;
        let id = *ui_models.style_graph.style_order.get(ui_models.style_graph.current_chain)?;
        let graph = state.project.library.node_graphs.get(&id)?;
        Some(f(graph, &ui_models.style_graph))
    }

    fn with_app_ro<R>(&self, f: impl FnOnce(&ApplicationGraph, &GraphEditorState) -> R) -> Option<R> {
        let AppStore { state, ui_models, .. } = self;
        let graph = state.project.style_layer.graphs.get(ui_models.app_graph.current_chain)?;
        Some(f(graph, &ui_models.app_graph))
    }

    pub fn set_node_position(&mut self, gt: crate::bridge::types::GraphType, index: usize, x: f32, y: f32) {
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                self.ui_models.style_graph.set_node_position(index, x, y)
            }
            crate::bridge::types::GraphType::AppGraph => {
                self.ui_models.app_graph.set_node_position(index, x, y)
            }
        }
    }

    pub fn add_node(
        &mut self,
        gt: crate::bridge::types::GraphType,
        template: usize,
        x: f32,
        y: f32,
    ) -> i32 {
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                let Some(kind) = style_templates().get(template).cloned() else {
                    return 0;
                };
                self.with_style(|g, st| add_node(g, st, kind.clone(), x, y))
                    .unwrap_or(0) as i32
            }
            crate::bridge::types::GraphType::AppGraph => {
                let Some(kind) = app_templates().get(template).cloned() else {
                    return 0;
                };
                self.with_app(|g, st| add_node(g, st, kind.clone(), x, y))
                    .unwrap_or(0) as i32
            }
        }
    }

    pub fn delete_node(&mut self, gt: crate::bridge::types::GraphType, index: usize) {
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                self.with_style(|g, st| delete_node(g, st, index));
            }
            crate::bridge::types::GraphType::AppGraph => {
                self.with_app(|g, st| delete_node(g, st, index));
            }
        }
    }

    /// 端口角色下拉选择：把该端口的默认值设为所选角色名（未连接时生效）。
    pub fn socket_option_selected(
        &mut self,
        gt: crate::bridge::types::GraphType,
        node: usize,
        socket_row: usize,
        option_index: usize,
    ) {
        // 候选 = 当前 actor 名字；末尾"新建角色"项由 UI 处理，不在此设置
        let actor_names = match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                self.ui_models.style_graph.actor_names.clone()
            }
            crate::bridge::types::GraphType::AppGraph => {
                self.ui_models.app_graph.actor_names.clone()
            }
        };
        let Some(name) = actor_names.get(option_index).cloned() else {
            return;
        };
        self.set_socket_text_default(gt, node, socket_row, &name);
    }

    /// 角色下拉"新建角色"：创建角色并把该端口默认值设为新角色名。
    pub fn create_actor(
        &mut self,
        gt: crate::bridge::types::GraphType,
        node: usize,
        socket_row: usize,
        name: SharedString,
    ) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        use aweft_model::library::Actor;
        use aweft_model::ids::ActorId;
        self.state_mut().project.library.insert_actor(Actor {
            id: ActorId::new(),
            name: name.clone(),
        });
        self.refresh_actors();
        self.set_socket_text_default(gt, node, socket_row, &name);
    }

    /// 把某输入端口默认值设为文本（角色名/字符串常量），并重新同步视图。
    pub fn set_socket_text_default(
        &mut self,
        gt: crate::bridge::types::GraphType,
        node: usize,
        socket_row: usize,
        text: &str,
    ) {
        let value = Some(aweft_model::node_graph::SocketValue::Text(text.to_string()));
        // socket_row 是全局行号（输入在前 0..inputs.len()，输出接着排）：
        // 对输入端口用 inputs，对输出端口（如字符串常量的 text）用 outputs。
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                self.with_style(|g, st| {
                    let Some(nid) = st.id_at(node) else { return };
                    let in_len = g.nodes[&nid].inputs.len();
                    let sid = if socket_row < in_len {
                        g.nodes[&nid].inputs.get(socket_row).copied()
                    } else {
                        g.nodes[&nid]
                            .outputs
                            .get(socket_row - in_len)
                            .copied()
                    };
                    if let Some(sid) = sid {
                        let _ = g.set_socket_value(sid, value.clone());
                    }
                    st.sync(g);
                });
            }
            crate::bridge::types::GraphType::AppGraph => {
                self.with_app(|g, st| {
                    let Some(nid) = st.id_at(node) else { return };
                    let in_len = g.nodes[&nid].inputs.len();
                    let sid = if socket_row < in_len {
                        g.nodes[&nid].inputs.get(socket_row).copied()
                    } else {
                        g.nodes[&nid]
                            .outputs
                            .get(socket_row - in_len)
                            .copied()
                    };
                    if let Some(sid) = sid {
                        let _ = g.set_socket_value(sid, value.clone());
                    }
                    st.sync(g);
                });
            }
        }
    }

    /// 刷新角色名字列表（按名排序，样式图/应用图共用）。
    pub fn refresh_actors(&mut self) {
        let mut names: Vec<String> = self
            .state
            .project
            .library
            .actors
            .values()
            .map(|a| a.name.clone())
            .collect();
        names.sort();
        self.ui_models.style_graph.actor_names = names.clone();
        self.ui_models.app_graph.actor_names = names;
    }

    /// 样式引用节点下拉菜单选择样式链：把节点引用的样式链换成所选。
    pub fn menu_selected(&mut self, gt: crate::bridge::types::GraphType, node: usize, menu_index: usize) {
        let Some(style_id) = self.ui_models.style_graph.style_order.get(menu_index).copied() else {
            return;
        };
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                self.with_style(|g, st| {
                    if let Some(nid) = st.id_at(node) {
                        if let Some(n) = g.nodes.get_mut(&nid) {
                            n.kind = NodeKind::Reference(style_id);
                        }
                    }
                    st.sync(g);
                });
            }
            crate::bridge::types::GraphType::AppGraph => {
                self.with_app(|g, st| {
                    if let Some(nid) = st.id_at(node) {
                        if let Some(n) = g.nodes.get_mut(&nid) {
                            n.kind = ApplicationNodeKind::AppRef(style_id);
                        }
                    }
                    st.sync(g);
                });
            }
        }
    }

    pub fn find_link_for_input(
        &self,
        gt: crate::bridge::types::GraphType,
        node: usize,
        sock: usize,
    ) -> i32 {
        match gt {
            crate::bridge::types::GraphType::StyleGraph => self
                .with_style_ro(|g, st| find_link_for_input(g, st, node, sock))
                .unwrap_or(-1),
            crate::bridge::types::GraphType::AppGraph => self
                .with_app_ro(|g, st| find_link_for_input(g, st, node, sock))
                .unwrap_or(-1),
        }
    }

    pub fn find_drop_socket(
        &self,
        gt: crate::bridge::types::GraphType,
        x: f32,
        y: f32,
        dtype: NodeSocketType,
    ) -> SocketHit {
        let want = socket_type_from_slint(dtype);
        match gt {
            crate::bridge::types::GraphType::StyleGraph => self
                .with_style_ro(|g, st| find_drop_socket(g, st, x, y, want))
                .unwrap_or(SocketHit {
                    found: false,
                    node: 0,
                    socket: 0,
                }),
            crate::bridge::types::GraphType::AppGraph => self
                .with_app_ro(|g, st| find_drop_socket(g, st, x, y, want))
                .unwrap_or(SocketHit {
                    found: false,
                    node: 0,
                    socket: 0,
                }),
        }
    }

    pub fn socket_drag_ended(
        &mut self,
        gt: crate::bridge::types::GraphType,
        node: usize,
        sock: usize,
        is_input: bool,
        x: f32,
        y: f32,
    ) {
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                self.with_style(|g, st| socket_drag_ended(g, st, node, sock, is_input, x, y));
            }
            crate::bridge::types::GraphType::AppGraph => {
                self.with_app(|g, st| socket_drag_ended(g, st, node, sock, is_input, x, y));
            }
        }
    }

    pub fn load_chain(&mut self, gt: crate::bridge::types::GraphType, index: usize) {
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                self.ui_models.style_graph.current_chain = index;
                let AppStore { state, ui_models, .. } = self;
                let id = ui_models.style_graph.style_order.get(index).copied();
                if let Some(id) = id {
                    if let Some(graph) = state.project.library.node_graphs.get(&id) {
                        ui_models.style_graph.sync(graph);
                    }
                }
            }
            crate::bridge::types::GraphType::AppGraph => {
                self.ui_models.app_graph.current_chain = index;
                let AppStore { state, ui_models, .. } = self;
                if let Some(graph) = state.project.style_layer.graphs.get(index) {
                    ui_models.app_graph.sync(graph);
                }
            }
        }
    }

    /// 派生样式链切换菜单（模型侧为 id-keyed map，显示顺序按名字排序，属 UI 数据）。
    fn refresh_style_chain_menu(&mut self) {
        let AppStore { state, ui_models, .. } = self;
        let mut pairs: Vec<(NodeGraphId, String)> = state
            .project
            .library
            .node_graphs
            .iter()
            .map(|(id, g)| (*id, g.name.clone()))
            .collect();
        pairs.sort_by(|a, b| a.1.cmp(&b.1));
        ui_models.style_graph.style_order = pairs.iter().map(|(id, _)| *id).collect();
        // 应用图也要 style_order（sync 时用它算样式引用节点的当前选中下标）
        ui_models.app_graph.style_order = pairs.iter().map(|(id, _)| *id).collect();
        ui_models.style_graph.style_chain_names = pairs.iter().map(|(_, name)| name.clone()).collect();
        // 应用图里 AppRef 节点的下拉菜单也用同一份样式链名字（sync 时读 app_graph 状态）
        ui_models.app_graph.style_chain_names = pairs.iter().map(|(_, name)| name.clone()).collect();
        let items: Vec<ContextMenuItem> = pairs
            .into_iter()
            .map(|(_, name)| ContextMenuItem {
                text: name.into(),
                enabled: true,
                dot_color: Color::from_argb_u8(0, 0, 0, 0),
                separator_before: false,
            })
            .collect();
        ui_models.style_graph.chain_items.set_vec(items);
    }

    /// 派生应用链切换菜单（style_layer.graphs 本身为 Vec，即优先级序）。
    fn refresh_app_chain_menu(&mut self) {
        let AppStore { state, ui_models, .. } = self;
        let items: Vec<ContextMenuItem> = state
            .project
            .style_layer
            .graphs
            .iter()
            .map(|g| ContextMenuItem {
                text: g.name.clone().into(),
                enabled: true,
                dot_color: Color::from_argb_u8(0, 0, 0, 0),
                separator_before: false,
            })
            .collect();
        ui_models.app_graph.chain_items.set_vec(items);
    }

    /// 新建一条默认链并切换到它，返回新链在切换菜单中的下标。
    pub fn new_chain(&mut self, gt: crate::bridge::types::GraphType) -> i32 {
        match gt {
            crate::bridge::types::GraphType::StyleGraph => {
                let n = self.state.project.library.node_graphs.len() + 1;
                let graph = new_style_chain(format!("样式链 {n}"));
                let id = graph.id;
                self.state.project.library.node_graphs.insert(id, graph);
                self.refresh_style_chain_menu();
                let idx = self
                    .ui_models
                    .style_graph
                    .style_order
                    .iter()
                    .position(|&x| x == id)
                    .unwrap_or(0);
                self.load_chain(crate::bridge::types::GraphType::StyleGraph, idx);
                idx as i32
            }
            crate::bridge::types::GraphType::AppGraph => {
                let n = self.state.project.style_layer.graphs.len() + 1;
                // 默认引用第一条样式链作为应用匹配的样式来源（无样式链时留空手动连）
                let style_ref = self.ui_models.style_graph.style_order.first().copied();
                self.refresh_actors();
                let first_actor = self.ui_models.app_graph.actor_names.first().cloned();
                let graph = new_app_chain(
                    format!("应用链 {n}"),
                    style_ref,
                    first_actor.as_deref(),
                );
                self.state.project.style_layer.graphs.push(graph);
                self.refresh_app_chain_menu();
                let idx = self.state.project.style_layer.graphs.len() - 1;
                self.load_chain(crate::bridge::types::GraphType::AppGraph, idx);
                // 新建默认链也按固定四列布局
                self.layout_default_app_chain();
                idx as i32
            }
        }
    }

    /// 初始化两个编辑器：建链菜单/右键菜单，绑定视图模型到 Slint 全局，加载第 0 条链。
    pub fn initialize_graph_editors(&mut self, instance: &crate::bridge::types::MainWindow) {
        use slint::ComponentHandle;

        let theme = instance.global::<crate::bridge::types::Theme>().get_data().node_editor;

        // 链菜单（显示顺序在 GUI 派生）
        self.refresh_style_chain_menu();
        self.refresh_app_chain_menu();
        // 角色下拉候选（匹配角色 pattern）
        self.refresh_actors();

        // 右键菜单（色点 = 节点主端口类型颜色）
        let style_menu: Vec<ContextMenuItem> = style_templates()
            .iter()
            .map(|k| {
                let dot = menu_dot_color_for(&theme, &NodeGraph::kind_decls(k));
                ContextMenuItem {
                    text: k.title().into(),
                    enabled: true,
                    dot_color: dot,
                    separator_before: k.title() == "组输出",
                }
            })
            .collect();
        let app_menu: Vec<ContextMenuItem> = app_templates()
            .iter()
            .map(|k| {
                let dot = menu_dot_color_for(&theme, &ApplicationGraph::kind_decls(k));
                ContextMenuItem {
                    text: k.title().into(),
                    enabled: true,
                    dot_color: dot,
                    separator_before: k.title() == "应用",
                }
            })
            .collect();
        self.ui_models.style_graph.menu_items.set_vec(style_menu);
        self.ui_models.app_graph.menu_items.set_vec(app_menu);

        // 绑定全局
        let state = instance.global::<crate::bridge::types::NodeGraphState>();
        state.set_style_chain_items(ModelRc::from(self.ui_models.style_graph.chain_items.clone()));
        state.set_app_chain_items(ModelRc::from(self.ui_models.app_graph.chain_items.clone()));
        state.set_style_current_chain(0);
        state.set_app_current_chain(0);
        state.set_style_nodes(ModelRc::from(self.ui_models.style_graph.nodes.clone()));
        state.set_style_links(ModelRc::from(self.ui_models.style_graph.links.clone()));
        state.set_style_menu_items(ModelRc::from(self.ui_models.style_graph.menu_items.clone()));
        state.set_app_nodes(ModelRc::from(self.ui_models.app_graph.nodes.clone()));
        state.set_app_links(ModelRc::from(self.ui_models.app_graph.links.clone()));
        state.set_app_menu_items(ModelRc::from(self.ui_models.app_graph.menu_items.clone()));

        self.load_chain(crate::bridge::types::GraphType::StyleGraph, 0);
        self.load_chain(crate::bridge::types::GraphType::AppGraph, 0);
        // 默认应用链固定四列布局（初始化时）
        self.layout_default_app_chain();
    }

    /// 默认应用链固定四列布局（用户要求）：
    ///   列1 上下文输入 | 列2 样式链引用(上)/匹配角色(下) | 列3 应用匹配 | 列4 组输出。
    /// 只对"当前应用链"设置，其余节点保留自动/已有位置。
    pub fn layout_default_app_chain(&mut self) {
        self.with_app(|g, st| {
            let mut pos: HashMap<NodeId, (f32, f32)> = HashMap::new();
            for (nid, n) in &g.nodes {
                let p = match &n.kind {
                    ApplicationNodeKind::ContextInput => (40.0, 40.0),
                    ApplicationNodeKind::AppRef(_) => (260.0, 40.0),
                    ApplicationNodeKind::MatchActor => (260.0, 120.0),
                    ApplicationNodeKind::ApplyMatch => (480.0, 40.0),
                    ApplicationNodeKind::GroupOutput => (700.0, 40.0),
                    _ => continue,
                };
                pos.insert(*nid, p);
            }
            st.positions = pos;
            st.sync(g);
        });
    }
}

fn menu_dot_color_for(
    theme: &crate::bridge::types::NodeEditorTheme,
    decls: &(Vec<SocketDecl>, Vec<SocketDecl>),
) -> Color {
    let ty = decls.1.first().or_else(|| decls.0.first()).map(|d| d.data_type);
    match ty {
        Some(SocketType::Float) => theme.socket_float_color,
        Some(SocketType::Text) => theme.socket_text_color,
        Some(SocketType::Int) => theme.socket_int_color,
        Some(SocketType::Bool) => theme.socket_bool_color,
        Some(SocketType::Color) => theme.socket_color_color,
        Some(SocketType::Vec2) => theme.socket_vec2_color,
        Some(SocketType::Vec3) => theme.socket_vec3_color,
        Some(SocketType::ClipRect) => theme.socket_clip_rect_color,
        Some(SocketType::Animation) => theme.socket_animation_color,
        _ => theme.socket_style_color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aweft_model::node_graph::NodeKind;
    use slint::Model;

    fn metrics() -> EditorMetrics {
        EditorMetrics {
            header: 24.0,
            row: 22.0,
            radius: 14.0,
        }
    }

    // 节点 i 的第 sock 个输入端口中心（世界坐标）：node 左缘 + 端口全局行号偏移。
    // 行号从 sockets 视图模型取（输出先排，输入在输出之后）。
    fn input_center(st: &GraphEditorState, node: usize, sock: usize) -> (f32, f32) {
        let (nx, ny) = st.nodes.row_data(node).map(|r| (r.x, r.y)).unwrap();
        let sockets = st.nodes.row_data(node).unwrap().sockets;
        let mut in_idx = 0usize;
        let mut row = sock as i32;
        for i in 0..sockets.row_count() {
            let s = sockets.row_data(i).unwrap();
            if s.is_input {
                if in_idx == sock {
                    row = s.row;
                    break;
                }
                in_idx += 1;
            }
        }
        (nx, ny + st.metrics.header + row as f32 * st.metrics.row + st.metrics.row / 2.0)
    }

    // 按标题查节点下标（自动布局顺序与插入序无关，用标题定位更稳）
    fn idx_by_title(st: &GraphEditorState, title: &str) -> usize {
        (0..st.nodes.row_count())
            .find(|&i| st.nodes.row_data(i).unwrap().title.as_str() == title)
            .unwrap()
    }

    #[test]
    fn add_node_returns_index_and_stores_position() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();

        let a = add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        let b = add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        assert!(a < st.nodes.row_count());
        assert!(b < st.nodes.row_count());
        assert_eq!(st.nodes.row_count(), 2);
        assert_eq!(st.links.row_count(), 0);

        // 拖动节点更新位置（UI 数据，不改图）
        let src = idx_by_title(&st, "样式源");
        st.set_node_position(src, 100.0, 200.0);
        assert_eq!(st.nodes.row_data(src).unwrap().x, 100.0);
        assert_eq!(st.nodes.row_data(src).unwrap().y, 200.0);
        assert_eq!(g.nodes.len(), 2);
    }

    #[test]
    fn output_drag_connects_on_drop() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);

        assert_eq!(st.links.row_count(), 1);
        let link = st.links.row_data(0).unwrap();
        assert_eq!(link.from_node, src as i32);
        assert_eq!(link.from_socket, 0);
        assert_eq!(link.to_node, out as i32);
        assert_eq!(link.to_socket, 0);
        assert_eq!(g.links.len(), 1);
    }

    #[test]
    fn output_drag_to_empty_discards() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        let src = idx_by_title(&st, "样式源");

        socket_drag_ended(&mut g, &mut st, src, 0, false, 5000.0, 5000.0);
        assert_eq!(st.links.row_count(), 0);
    }

    #[test]
    fn input_drag_to_empty_disconnects() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        assert_eq!(st.links.row_count(), 1);

        // 拖输入端到空白：断开
        socket_drag_ended(&mut g, &mut st, out, 0, true, 5000.0, 5000.0);
        assert_eq!(st.links.row_count(), 0);
        assert_eq!(g.links.len(), 0);
    }

    #[test]
    fn input_drag_moves_connection_to_other_node() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        add_node(&mut g, &mut st, NodeKind::MergeStyle, 500.0, 40.0);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");
        let merge = idx_by_title(&st, "合并样式");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        assert_eq!(st.links.row_count(), 1);

        // 把 组输出 的输入搬到 合并样式 的输入（源保持不变）
        let (nx, ny) = input_center(&st, merge, 0);
        socket_drag_ended(&mut g, &mut st, out, 0, true, nx, ny);

        assert_eq!(st.links.row_count(), 1);
        let link = st.links.row_data(0).unwrap();
        let from_row = st.nodes.row_data(link.from_node as usize).unwrap();
        let to_row = st.nodes.row_data(link.to_node as usize).unwrap();
        assert_eq!(from_row.title.as_str(), "样式源");
        assert_eq!(to_row.title.as_str(), "合并样式");
        assert_eq!(g.links.len(), 1);
    }

    #[test]
    fn delete_node_removes_links_and_rebuilds_indices() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        assert_eq!(st.links.row_count(), 1);

        delete_node(&mut g, &mut st, src);
        assert_eq!(st.nodes.row_count(), 1);
        assert_eq!(st.links.row_count(), 0);
    }

    #[test]
    fn find_link_for_input_returns_model_index() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);

        assert_eq!(find_link_for_input(&g, &st, out, 0), 0);
        assert_eq!(find_link_for_input(&g, &st, src, 0), -1);
    }

    #[test]
    fn default_chain_layout_source_left() {
        // 默认样式链（无位置记录）自动布局：样式源在最左、组输出在最右，且确定
        let g = new_style_chain("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        st.sync(&g);

        assert_eq!(st.nodes.row_count(), 2);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");
        assert_eq!(st.nodes.row_data(src).unwrap().x, 40.0);
        assert_eq!(st.nodes.row_data(out).unwrap().x, 260.0);
        assert!(st.nodes.row_data(src).unwrap().x < st.nodes.row_data(out).unwrap().x);
        assert_eq!(st.links.row_count(), 1);
    }

    #[test]
    fn default_app_chain_layout_source_left() {
        let g = new_app_chain("t", Some(NodeGraphId::new()), None);
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        st.sync(&g);

        assert_eq!(st.nodes.row_count(), 5);
        let ctx = idx_by_title(&st, "上下文输入");
        let apply = idx_by_title(&st, "应用匹配");
        let out = idx_by_title(&st, "组输出");
        assert_eq!(st.nodes.row_data(ctx).unwrap().x, 40.0);
        assert_eq!(st.nodes.row_data(apply).unwrap().x, 480.0);
        assert!(st.nodes.row_data(apply).unwrap().x < st.nodes.row_data(out).unwrap().x);
        assert_eq!(st.links.row_count(), 4);
    }

    #[test]
    fn sync_is_deterministic_across_switches() {
        // 切换链（加载另一条图再回来）后布局与顺序都保持不变
        let g1 = new_style_chain("a");
        let g2 = new_app_chain("b", None, None);
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        st.sync(&g1);
        let before: Vec<(String, f32, f32)> = (0..st.nodes.row_count())
            .map(|i| {
                let r = st.nodes.row_data(i).unwrap();
                (r.title.as_str().to_string(), r.x, r.y)
            })
            .collect();
        st.sync(&g2);
        st.sync(&g1);
        let after: Vec<(String, f32, f32)> = (0..st.nodes.row_count())
            .map(|i| {
                let r = st.nodes.row_data(i).unwrap();
                (r.title.as_str().to_string(), r.x, r.y)
            })
            .collect();
        assert_eq!(before, after);
    }
}
