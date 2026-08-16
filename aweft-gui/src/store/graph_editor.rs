//! 节点图编辑器 Rust 侧：持有编辑器视图模型（节点/连线/位置）与所有图操作。
//!
//! 图结构数据（nodes/sockets/links）权威在 `aweft-model`；本模块只保存 UI 数据
//! （节点位置、模型下标顺序）并生成 Slint 的节点/连线视图模型。
//! 端口命中、连线重连等"遍历"逻辑原先在 Slint 内硬编码逐槽位展开，现统一在此用循环实现。
//!
//! 接入 main 的架构决策（A2）：图操作**自洽**（改 model + sync 视图模型），
//! 不走 `Action → dispatch → reducer → tag → depsgraph 投影`（无跨组件联动）。
//! 直接调用 AppStore 方法；结构变更后打 `Tag::NodeGraphChanged` 为未来联动留口。

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use aweft_model::{
    application_graph::{ApplicationGraph, ApplicationNodeKind},
    ids::{NodeGraphId, NodeId, SocketId},
    node_graph::{
        error::EvalError, ColorRole, Link, Node, NodeGraph, NodeGraphKind, NodeKind, NodeTreeType,
        Socket, SocketDecl, SocketType, SocketValue, StyleOpKind, VecOpKind,
    },
};
use slint::{Color, Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};

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
    pub node_width: f32, // 节点宽度（输出端口中心判定用）
}

// ── 视图模型 ──────────────────────────────────────────────────────────────────

/// 节点数据源模型：持投影构建的节点快照，实现 `slint::Model`。
/// `row_data` O(1) clone 快照；投影（op_node_graph_*）在权威图变化后更新快照并
/// `notify`（row_added/removed/changed 精确通知），Repeater 只增删改受影响实例。
#[derive(Clone)]
pub struct GraphNodesModel {
    rows: Rc<std::cell::RefCell<Vec<GraphNode>>>,
    notify: Rc<ModelNotify>,
}

impl GraphNodesModel {
    pub fn new() -> Self {
        Self {
            rows: Rc::new(std::cell::RefCell::new(Vec::new())),
            notify: Rc::new(ModelNotify::default()),
        }
    }

    /// 全量替换快照（初始化/切链用），`notify.reset()` 刷新所有行。
    pub fn set_rows(&self, rows: Vec<GraphNode>) {
        *self.rows.borrow_mut() = rows;
        self.notify.reset();
    }

    /// 单行替换（值变化），`row_changed` 精确通知。
    pub fn set_row(&self, index: usize, row: GraphNode) {
        let mut rows = self.rows.borrow_mut();
        if index < rows.len() {
            rows[index] = row;
        }
        drop(rows);
        self.notify.row_changed(index);
    }

    /// 追加一行（末尾新增），`row_added` 精确通知。
    pub fn push(&self, row: GraphNode) {
        let mut rows = self.rows.borrow_mut();
        rows.push(row);
        let idx = rows.len() - 1;
        drop(rows);
        self.notify.row_added(idx, 1);
    }

    /// 移除一行，`row_removed` 精确通知。
    pub fn remove(&self, index: usize) {
        let mut rows = self.rows.borrow_mut();
        if index < rows.len() {
            rows.remove(index);
        }
        drop(rows);
        self.notify.row_removed(index, 1);
    }

    pub fn notify_row_changed(&self, row: usize) {
        self.notify.row_changed(row);
    }
}

impl Default for GraphNodesModel {
    fn default() -> Self {
        Self::new()
    }
}

impl Model for GraphNodesModel {
    type Data = GraphNode;
    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }
    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.rows.borrow().get(row).cloned()
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        self.notify.as_ref()
    }
}

/// 连线数据源模型：持投影构建的连线快照，实现 `slint::Model`。
#[derive(Clone)]
pub struct GraphLinksModel {
    rows: Rc<std::cell::RefCell<Vec<GraphLink>>>,
    notify: Rc<ModelNotify>,
}

impl GraphLinksModel {
    pub fn new() -> Self {
        Self {
            rows: Rc::new(std::cell::RefCell::new(Vec::new())),
            notify: Rc::new(ModelNotify::default()),
        }
    }

    pub fn set_rows(&self, rows: Vec<GraphLink>) {
        *self.rows.borrow_mut() = rows;
        self.notify.reset();
    }

    /// 单行替换（节点删除后连线下标左移等），`row_changed` 精确通知。
    pub fn set_row(&self, index: usize, row: GraphLink) {
        let mut rows = self.rows.borrow_mut();
        if index < rows.len() {
            rows[index] = row;
        }
        drop(rows);
        self.notify.row_changed(index);
    }

    pub fn push(&self, row: GraphLink) {
        let mut rows = self.rows.borrow_mut();
        rows.push(row);
        let idx = rows.len() - 1;
        drop(rows);
        self.notify.row_added(idx, 1);
    }

    pub fn remove(&self, index: usize) {
        let mut rows = self.rows.borrow_mut();
        if index < rows.len() {
            rows.remove(index);
        }
        drop(rows);
        self.notify.row_removed(index, 1);
    }
}

impl Default for GraphLinksModel {
    fn default() -> Self {
        Self::new()
    }
}

impl Model for GraphLinksModel {
    type Data = GraphLink;
    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }
    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.rows.borrow().get(row).cloned()
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        self.notify.as_ref()
    }
}

/// 一个节点图类型的编辑器状态。
pub struct GraphEditorState {
    /// 节点数据源模型（title/x/y/sockets，含 UI 位置数据）
    pub nodes: Rc<GraphNodesModel>,
    /// 连线数据源模型（端口均以 nodes 模型下标引用）
    pub links: Rc<GraphLinksModel>,
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
            nodes: Rc::new(GraphNodesModel::new()),
            links: Rc::new(GraphLinksModel::new()),
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
    fn pos(&self, node_id: NodeId, order_index: usize) -> (f32, f32) {
        self.positions
            .get(&node_id)
            .copied()
            .unwrap_or((40.0 + order_index as f32 * 220.0, 40.0))
    }

    /// 把图的当前结构同步到视图模型（顺序只增不减：已有节点不动，新节点追加，删掉的移除，
    /// 保证模型下标稳定，不随连线变化漂移）。
    /// 无位置记录的节点按"上游深度"自动布局（源在左、汇在右，同深度纵向堆叠）——
    /// 位置直接由图结构推导，与迭代顺序无关，因此默认布局确定且正确（样式源在最左、组输出在最右）。
    /// 位置一旦分配即持久化（UI 数据），之后拖拽/重连都不会让节点跳位。
    ///
    /// 运行期变更走 `apply_change` 精确 diff（row_added/row_removed/row_changed）；
    /// `rebuild`（本函数所在，供初始化/切链）仍全量 set_rows。
    /// 构建单个节点的 Slint 快照（供投影对单个节点精确构建）。
    /// 需要 `editor` 的布局/角色/样式链上下文。
    pub fn build_node_at<G: GraphEditable>(&self, graph: &G, node_id: NodeId) -> Option<GraphNode> {
        let node = graph.node(node_id);
        let node_index = self.order.iter().position(|&id| id == node_id)?;
        let (x, y) = self.pos(node_id, node_index);
        // 端口 row = 列内下标：输入列/输出列各自从顶部独立排列（Blender 式），
        // 互不影响。渲染/命中/连线全部用列内 row。
        let input_count = G::inputs(node).len();
        let output_count = G::outputs(node).len();
        let mut sockets = Vec::with_capacity(input_count + output_count);
        for (input_row, &socket_id) in G::inputs(node).iter().enumerate() {
            let socket = graph.socket(socket_id);
            let connected = graph.links().iter().any(|link| link.to == socket_id);
            let is_actor = G::actor_dropdown(node, true, input_row);
            let selected = if is_actor {
                match &socket.default_value {
                    Some(aweft_model::node_graph::SocketValue::Text(text)) => self
                        .actor_names
                        .iter()
                        .position(|actor_name| actor_name == text)
                        .map_or(-1, |actor_index| actor_index as i32),
                    _ => -1,
                }
            } else {
                -1
            };
            sockets.push(NodeSocketData {
                name: socket.name.clone().into(),
                is_input: true,
                data_type: socket_type_to_slint(socket.data_type),
                row: input_row as i32,
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
        for (output_row, &socket_id) in G::outputs(node).iter().enumerate() {
            let socket = graph.socket(socket_id);
            let connected = graph.links().iter().any(|link| link.from == socket_id);
            let is_editor = G::text_editor(node, false, output_row);
            let input_text = if is_editor {
                match &socket.default_value {
                    Some(aweft_model::node_graph::SocketValue::Text(text)) => text.clone().into(),
                    _ => SharedString::default(),
                }
            } else {
                SharedString::default()
            };
            sockets.push(NodeSocketData {
                name: socket.name.clone().into(),
                is_input: false,
                data_type: socket_type_to_slint(socket.data_type),
                row: output_row as i32,
                menu_items: ModelRc::from(Rc::new(VecModel::default())),
                selected: -1,
                connected,
                input: is_editor,
                input_text,
            });
        }
        // 样式引用节点：生成下拉菜单行数据（标签 + 样式链选项 + 当前选中）
        let (menu_label, menu_options, menu_selected) = match G::reference_chain_id(node) {
            Some(ref_chain_id) => {
                let selected = self
                    .style_order
                    .iter()
                    .position(|&chain_id| chain_id == ref_chain_id)
                    .map_or(-1, |chain_index| chain_index as i32);
                let options: Vec<SharedString> = self
                    .style_chain_names
                    .iter()
                    .cloned()
                    .map(SharedString::from)
                    .collect();
                (
                    SharedString::from("样式链"),
                    ModelRc::from(Rc::new(VecModel::from(options))),
                    selected,
                )
            }
            None => (
                SharedString::default(),
                ModelRc::from(Rc::new(VecModel::default())),
                0,
            ),
        };
        Some(GraphNode {
            title: graph.title(node).into(),
            x,
            y,
            sockets: Rc::new(VecModel::from(sockets)).into(),
            input_count: input_count as i32,
            // 端口行数 = 两列各自独立从顶排，节点高度取两列较大者
            socket_rows: input_count.max(output_count) as i32,
            menu_label,
            menu_options,
            menu_selected,
        })
    }

    /// 构建全部连线快照（按当前 order 映射端口下标）。
    pub fn build_links<G: GraphEditable>(&self, graph: &G) -> Vec<GraphLink> {
        graph
            .links()
            .iter()
            .filter_map(|link| {
                let from_index =
                    self.order.iter().position(|&node_id| node_id == graph.socket(link.from).node)?;
                let to_index =
                    self.order.iter().position(|&node_id| node_id == graph.socket(link.to).node)?;
                // 端口下标 = 列内 row：from 是输出列内下标，to 是输入列内下标
                let from_socket = G::outputs(graph.node(self.order[from_index]))
                    .iter()
                    .position(|&socket_id| socket_id == link.from)?;
                let to_socket = G::inputs(graph.node(self.order[to_index]))
                    .iter()
                    .position(|&socket_id| socket_id == link.to)?;
                Some(GraphLink {
                    from_node: from_index as i32,
                    from_socket: from_socket as i32,
                    to_node: to_index as i32,
                    to_socket: to_socket as i32,
                    data_type: socket_type_to_slint(graph.socket(link.from).data_type),
                })
            })
            .collect()
    }

    /// 全量重建快照（初始化/切链用）：更新 order + 布局位置 + 节点/连线快照。
    /// 运行期精确更新走 `apply_change`，此处仅限切链/初始化。
    pub fn rebuild<G: GraphEditable>(&mut self, graph: &G) {
        self.order.retain(|node_id| graph.nodes().contains_key(node_id));
        for node_id in graph.nodes().keys() {
            if !self.order.contains(node_id) {
                self.order.push(*node_id);
            }
        }
        let mut depth_cache = HashMap::new();
        let mut visiting = HashSet::new();
        let mut depth_rows = HashMap::new();
        for &node_id in &self.order {
            self.positions.entry(node_id).or_insert_with(|| {
                let depth = flow_depth(graph, node_id, &mut depth_cache, &mut visiting);
                let y = 40.0 + depth_rows.get(&depth).copied().unwrap_or(0) as f32 * 60.0;
                *depth_rows.entry(depth).or_insert(0) += 1;
                (40.0 + depth as f32 * 220.0, y)
            });
        }
        let rows: Vec<GraphNode> = self
            .order
            .iter()
            .filter_map(|&node_id| self.build_node_at(graph, node_id))
            .collect();
        self.nodes.set_rows(rows);
        let links = self.build_links(graph);
        self.links.set_rows(links);
    }

    /// 应用一次精确变更（投影 op 调用）：按 change 局部更新快照 + notify。
    /// 节点增删走 row_added/row_removed 精确；连线变化重建连线 + 受影响节点行；
    /// 端口值变化单行更新；切链全量重建。
    pub fn apply_change<G: GraphEditable>(&mut self, graph: &G, change: &crate::depsgraph::NodeGraphChange) {
        use crate::depsgraph::NodeGraphChange;
        match change {
            NodeGraphChange::NodesInserted { node_ids, positions } => {
                for &node_id in node_ids {
                    if let Some(&pos) = positions.get(&node_id) {
                        self.positions.insert(node_id, pos);
                    }
                    if !self.order.contains(&node_id) {
                        self.order.push(node_id);
                    }
                    if let Some(node) = self.build_node_at(graph, node_id) {
                        // 新节点追加到末尾（位置由 positions 指定）
                        self.nodes.push(node);
                    }
                }
                // 新节点无连线、order 追加末尾 → 既有连线下标不变，连线模型无需更新
            }
            NodeGraphChange::NodesRemoved { node_ids } => {
                for &node_id in node_ids {
                    if let Some(idx) = self.order.iter().position(|&n| n == node_id) {
                        self.order.remove(idx);
                        self.positions.remove(&node_id);
                        self.nodes.remove(idx);
                        // 连线局部更新：移除该节点的连线行 + 之后下标左移
                        self.remove_node_links(idx);
                    }
                }
            }
            NodeGraphChange::LinksChanged { affected_nodes } => {
                // 连线局部更新：值 diff（节点下标不变，值唯一标识连线）——
                // 移除 old 中消失的行（row_removed）、追加 new 中新增的行（row_added）
                let new_links = self.build_links(graph);
                let old: Vec<GraphLink> = (0..self.links.row_count())
                    .map(|i| self.links.row_data(i).unwrap())
                    .collect();
                let to_remove: Vec<usize> =
                    (0..old.len()).filter(|&i| !new_links.contains(&old[i])).collect();
                for &i in to_remove.iter().rev() {
                    self.links.remove(i);
                }
                for link in &new_links {
                    if !old.contains(link) {
                        self.links.push(link.clone());
                    }
                }
                // 受影响节点行：connected 标记变化，只 row_changed 这些行
                for &node_id in affected_nodes {
                    if let Some(idx) = self.order.iter().position(|&n| n == node_id)
                        && let Some(node) = self.build_node_at(graph, node_id)
                    {
                        self.nodes.set_row(idx, node);
                    }
                }
            }
            NodeGraphChange::SocketValueChanged { node_id, .. } => {
                if let Some(idx) = self.order.iter().position(|&n| n == *node_id)
                    && let Some(node) = self.build_node_at(graph, *node_id)
                {
                    self.nodes.set_row(idx, node);
                }
            }
            NodeGraphChange::ReferenceChanged { node_id } => {
                // 换引用链：原位改值，节点不增删——单行重建（端口/高度/menu_selected）。
                if let Some(idx) = self.order.iter().position(|&n| n == *node_id)
                    && let Some(node) = self.build_node_at(graph, *node_id)
                {
                    self.nodes.set_row(idx, node);
                }
            }
            // 新建链：投影 op_node_graph_chain 已完成菜单派生/当前链设置，这里兜底重建。
            NodeGraphChange::ChainSwitched { .. } => self.rebuild(graph),
        }
    }

    pub fn id_at(&self, index: usize) -> Option<NodeId> {
        self.order.get(index).copied()
    }

    /// 节点删除后的连线局部更新：移除引用该节点下标的连线行（row_removed），
    /// 剩余行中 from_node/to_node 大于删除下标的左移 1（row_changed）。
    fn remove_node_links(&mut self, removed_idx: usize) {
        let to_remove: Vec<usize> = (0..self.links.row_count())
            .filter(|&i| {
                let r = self.links.row_data(i).unwrap();
                r.from_node as usize == removed_idx || r.to_node as usize == removed_idx
            })
            .collect();
        for &i in to_remove.iter().rev() {
            self.links.remove(i);
        }
        for i in 0..self.links.row_count() {
            let mut r = self.links.row_data(i).unwrap();
            let mut changed = false;
            if r.from_node as usize > removed_idx {
                r.from_node -= 1;
                changed = true;
            }
            if r.to_node as usize > removed_idx {
                r.to_node -= 1;
                changed = true;
            }
            if changed {
                self.links.set_row(i, r);
            }
        }
    }

    /// 更新节点位置（仅 UI 数据，不改图结构）
    pub fn set_node_position(&mut self, index: usize, x: f32, y: f32) {
        if let Some(&node_id) = self.order.get(index) {
            self.positions.insert(node_id, (x, y));
            if let Some(mut node_row) = self.nodes.row_data(index) {
                node_row.x = x;
                node_row.y = y;
                self.nodes.set_row(index, node_row);
            }
        }
    }

    /// 端口命中（本地坐标，相对节点左上角，逻辑像素）：返回 sockets 组合数组下标，-1 无。
    /// 替代 Slint 硬编码逐下标（C1 修复：半动态节点扩展端口 >4 时依旧可用）。
    ///
    /// Blender 式"最近距离"判定（参考 `node_find_indicated_socket`）：圆形命中半径
    /// = socket-size，多个端口同时命中时取距离最近者——方形判定在端口斜向接近时
    /// 会产生不匹配，且无法表达"选最近的"。
    pub fn find_socket_at<G: GraphEditable>(
        &self,
        graph: &G,
        node_index: usize,
        local_x: f32,
        local_y: f32,
    ) -> i32 {
        let Some(&node_id) = self.order.get(node_index) else {
            return -1;
        };
        let node = graph.node(node_id);
        let node_width = self.metrics.node_width;
        let header_height = self.metrics.header;
        let row_height = self.metrics.row;
        let radius = self.metrics.radius;
        let input_count = G::inputs(node).len();

        // 各端口中心：输入列在左缘外半伸（row 0..in），输出列在右缘外半伸（row 0..out）。
        // 用距离²（免开方），选半径内最近者。
        let mut best_socket = -1;
        let mut best_distance_sq = f32::MAX;
        let mut consider = |socket_index: usize, center_x: f32, center_y: f32| {
            let dx = local_x - center_x;
            let dy = local_y - center_y;
            let distance_sq = dx * dx + dy * dy;
            if distance_sq < radius * radius && distance_sq < best_distance_sq {
                best_socket = socket_index as i32;
                best_distance_sq = distance_sq;
            }
        };
        for (input_row, _) in G::inputs(node).iter().enumerate() {
            consider(
                input_row,
                0.0, // 输入圆点中心在节点左缘（半伸出），与渲染/拖拽命中一致
                header_height + input_row as f32 * row_height + row_height / 2.0,
            );
        }
        for (output_row, _) in G::outputs(node).iter().enumerate() {
            consider(
                input_count + output_row,
                node_width, // 输出圆点中心在节点右缘
                header_height + output_row as f32 * row_height + row_height / 2.0,
            );
        }
        best_socket
    }
}

// ── 图类型适配 ────────────────────────────────────────────────────────────────

/// 节点 UI 注册（GUI 侧，与 model 分离）：标题为本地化呈现，端口 UI 语义属交互层。
/// 每种树类型实现一次；`GraphEditable` 的泛型 impl 借此保持单实现。
pub trait NodeUi: NodeTreeType {
    /// 节点显示名。
    fn title(&self) -> &'static str;
    /// 该端口是否需要"角色下拉"（候选 = 当前所有 actor，含"新建角色"项）。
    /// idx 为端口在输入/输出列内的下标。
    fn actor_dropdown(&self, is_input: bool, idx: usize) -> bool;
    /// 该端口行是否内嵌文本输入框（如字符串常量的输出端口）。
    fn text_editor(&self, is_input: bool, idx: usize) -> bool;
}

impl NodeUi for NodeKind {
    fn title(&self) -> &'static str {
        node_kind_title(self)
    }
    fn actor_dropdown(&self, _is_input: bool, _idx: usize) -> bool {
        false
    }
    fn text_editor(&self, _is_input: bool, _idx: usize) -> bool {
        false
    }
}

impl NodeUi for ApplicationNodeKind {
    fn title(&self) -> &'static str {
        application_node_kind_title(self)
    }
    fn actor_dropdown(&self, is_input: bool, idx: usize) -> bool {
        // 匹配角色的 pattern 输入（第 1 个输入）用角色下拉
        is_input && idx == 1 && matches!(self, ApplicationNodeKind::MatchActor)
    }
    fn text_editor(&self, is_input: bool, idx: usize) -> bool {
        // 字符串常量的输出端口：无输入，端口行内嵌文本输入框
        !is_input && idx == 0 && matches!(self, ApplicationNodeKind::TextConst)
    }
}

/// 样式节点显示名（本地化呈现，属 GUI 层）。
fn node_kind_title(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::StyleSource => "样式源",
        NodeKind::VecOp(kind) => match kind {
            VecOpKind::CombineVec2 => "合并向量2D",
            VecOpKind::SeparateVec2 => "分离向量2D",
            VecOpKind::CombineVec3 => "合并向量3D",
            VecOpKind::SeparateVec3 => "分离向量3D",
        },
        NodeKind::StyleOp(kind) => match kind {
            StyleOpKind::SetFontName => "字体名称",
            StyleOpKind::SetFontSize => "字号",
            StyleOpKind::SetBold => "粗体",
            StyleOpKind::SetItalic => "斜体",
            StyleOpKind::SetUnderline => "下划线",
            StyleOpKind::SetStrikeout => "删除线",
            StyleOpKind::SetColor(role) => match role {
                ColorRole::Primary => "颜色-主色",
                ColorRole::Secondary => "颜色-副色",
                ColorRole::Outline => "颜色-描边",
                ColorRole::Back => "颜色-背景",
                ColorRole::All => "颜色-全部",
            },
            StyleOpKind::SetAlpha(role) => match role {
                ColorRole::Primary => "透明度-主色",
                ColorRole::Secondary => "透明度-副色",
                ColorRole::Outline => "透明度-描边",
                ColorRole::Back => "透明度-背景",
                ColorRole::All => "透明度-全部",
            },
            StyleOpKind::SetBorder => "描边",
            StyleOpKind::SetShadow => "阴影",
            StyleOpKind::SetFontScale => "字体缩放",
            StyleOpKind::SetFontRotation => "字体旋转",
            StyleOpKind::SetFactor => "系数",
            StyleOpKind::SetBlur => "模糊",
            StyleOpKind::SetBlurEdges => "边缘模糊",
            StyleOpKind::SetPosition => "位置",
            StyleOpKind::SetOrigin => "原点",
            StyleOpKind::SetClipRect => "裁剪区域",
        },
        NodeKind::MergeStyle => "合并样式",
        NodeKind::GroupInput => "组输入",
        NodeKind::GroupOutput => "组输出",
        NodeKind::Viewer => "查看器",
        NodeKind::Reference(_) => "样式引用",
    }
}

/// 应用节点显示名（本地化呈现，属 GUI 层）。
fn application_node_kind_title(kind: &ApplicationNodeKind) -> &'static str {
    match kind {
        ApplicationNodeKind::ContextInput => "上下文输入",
        ApplicationNodeKind::MatchActor => "匹配角色",
        ApplicationNodeKind::MatchText => "匹配文本",
        ApplicationNodeKind::IntEq => "整数相等",
        ApplicationNodeKind::And => "与",
        ApplicationNodeKind::Or => "或",
        ApplicationNodeKind::Not => "非",
        ApplicationNodeKind::AppRef(_) => "样式链引用",
        ApplicationNodeKind::TextConst => "字符串常量",
        ApplicationNodeKind::ApplyMatch => "应用匹配",
        ApplicationNodeKind::ApplyAnimation => "应用动画",
        ApplicationNodeKind::GroupOutput => "组输出",
    }
}

/// 让所有树类型图（样式 `NodeGraph<NodeKind>` / 应用 `NodeGraph<ApplicationNodeKind>`）
/// 都能喂给通用编辑器逻辑。实现仅做类型化转发，结构差异由 model 的 `NodeTreeType`
/// 注册表承担，UI 差异由 GUI 的 `NodeUi` 注册表承担。
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
    /// 设置端口默认值（返回模型约束错误）。
    fn graph_set_socket_value(
        &mut self,
        sid: SocketId,
        value: Option<SocketValue>,
    ) -> Result<(), EvalError>;
    /// 原位改引用节点的引用链（仅改 kind 内嵌 id，不删节点）。
    fn graph_set_reference(&mut self, node_id: NodeId, new_id: NodeGraphId) -> Result<(), EvalError>;
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

impl<K: NodeTreeType + NodeUi> GraphEditable for NodeGraph<K> {
    type Node = Node<K>;
    type Kind = K;

    fn nodes(&self) -> &HashMap<NodeId, Node<K>> {
        &self.nodes
    }
    fn sockets(&self) -> &HashMap<SocketId, Socket> {
        &self.sockets
    }
    fn links(&self) -> &[Link] {
        &self.links
    }
    fn node(&self, id: NodeId) -> &Node<K> {
        &self.nodes[&id]
    }
    fn socket(&self, id: SocketId) -> &Socket {
        &self.sockets[&id]
    }
    fn graph_add_node(&mut self, kind: K) -> Result<NodeId, EvalError> {
        NodeGraph::<K>::add_node::<NodeId>(self, kind, &[], &[])
    }
    fn graph_connect(&mut self, from: SocketId, to: SocketId) -> Result<(), EvalError> {
        NodeGraph::<K>::connect_raw(self, from, to)
    }
    fn graph_disconnect_input(&mut self, sid: SocketId) {
        NodeGraph::<K>::disconnect_input(self, sid);
    }
    fn graph_delete(&mut self, id: NodeId) {
        NodeGraph::<K>::remove_node(self, id);
    }
    fn graph_set_socket_value(
        &mut self,
        sid: SocketId,
        value: Option<SocketValue>,
    ) -> Result<(), EvalError> {
        NodeGraph::<K>::set_socket_value(self, sid, value)
    }
    fn graph_set_reference(
        &mut self,
        node_id: NodeId,
        new_id: NodeGraphId,
    ) -> Result<(), EvalError> {
        NodeGraph::<K>::set_node_reference(self, node_id, new_id)
    }
    fn title(&self, n: &Node<K>) -> &'static str {
        n.kind.title()
    }
    fn inputs(n: &Node<K>) -> &[SocketId] {
        &n.inputs
    }
    fn outputs(n: &Node<K>) -> &[SocketId] {
        &n.outputs
    }
    fn kind_decls(kind: &K) -> (Vec<SocketDecl>, Vec<SocketDecl>) {
        kind.socket_decls().unwrap_or_default()
    }
    fn reference_chain_id(n: &Node<K>) -> Option<NodeGraphId> {
        n.kind.reference_id()
    }
    fn actor_dropdown(n: &Node<K>, is_input: bool, idx: usize) -> bool {
        n.kind.actor_dropdown(is_input, idx)
    }
    fn text_editor(n: &Node<K>, is_input: bool, idx: usize) -> bool {
        n.kind.text_editor(is_input, idx)
    }
}

// ── 图操作（原 Slint 逐槽位展开逻辑，现用循环） ───────────────────────────────

/// 上游深度：经输入连线可达的最深上游层数（源节点 = 0）。带记忆与环保护。
fn flow_depth<G: GraphEditable>(
    graph: &G,
    node_id: NodeId,
    depth_cache: &mut HashMap<NodeId, usize>,
    visiting: &mut HashSet<NodeId>,
) -> usize {
    if let Some(&depth) = depth_cache.get(&node_id) {
        return depth;
    }
    if !visiting.insert(node_id) {
        return 0; // 有环：按叶子处理，避免无限递归
    }
    let node = graph.node(node_id);
    let depth = G::inputs(node)
        .iter()
        .filter_map(|&socket_id| {
            graph
                .links()
                .iter()
                .find(|link| link.to == socket_id)
                .map(|link| graph.socket(link.from).node)
        })
        .map(|upstream| flow_depth(graph, upstream, depth_cache, visiting) + 1)
        .max()
        .unwrap_or(0);
    visiting.remove(&node_id);
    depth_cache.insert(node_id, depth);
    depth
}

/// 角色下拉菜单项：当前所有 actor + 末尾"新建角色"。
/// 返回 (menu-items, 有效角色数)；末尾"新建角色"项由调用方识别。
fn actor_menu_items(actor_names: &[String]) -> ModelRc<ContextMenuItem> {
    let mut items: Vec<ContextMenuItem> = actor_names
        .iter()
        .map(|actor_name| ContextMenuItem {
            text: actor_name.clone().into(),
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
    let mut graph = NodeGraph::new(name);
    let source = graph
        .add_node::<NodeId>(NodeKind::StyleSource, &[], &[])
        .unwrap();
    let output = graph
        .add_node::<NodeId>(NodeKind::GroupOutput, &[], &[])
        .unwrap();
    graph
        .connect_raw(graph.nodes[&source].outputs[0], graph.nodes[&output].inputs[0])
        .unwrap();
    graph
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
    let mut graph = ApplicationGraph::new(name);
    let context = graph
        .add_node::<NodeId>(ApplicationNodeKind::ContextInput, &[], &[])
        .unwrap();
    let matcher = graph
        .add_node::<NodeId>(ApplicationNodeKind::MatchActor, &[], &[])
        .unwrap();
    let apply = graph
        .add_node::<NodeId>(ApplicationNodeKind::ApplyMatch, &[], &[])
        .unwrap();
    let output = graph
        .add_node::<NodeId>(
            ApplicationNodeKind::GroupOutput,
            &[SocketDecl::input("application", SocketType::Application)],
            &[],
        )
        .unwrap();
    graph
        .connect_raw(graph.nodes[&context].outputs[0], graph.nodes[&matcher].inputs[0])
        .unwrap();
    // 匹配角色的 pattern（第 1 输入）默认角色（实际连接数据，非硬编码预设）
    if let Some(actor) = default_actor {
        graph
            .set_socket_value(
                graph.nodes[&matcher].inputs[1],
                Some(aweft_model::node_graph::SocketValue::Text(actor.into())),
            )
            .unwrap();
    }
    graph
        .connect_raw(graph.nodes[&matcher].outputs[0], graph.nodes[&apply].inputs[1])
        .unwrap();
    if let Some(style_id) = style_ref {
        let app_ref = graph
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(style_id),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        graph
            .connect_raw(graph.nodes[&app_ref].outputs[0], graph.nodes[&apply].inputs[0])
            .unwrap();
    }
    graph
        .connect_raw(graph.nodes[&apply].outputs[0], graph.nodes[&output].inputs[0])
        .unwrap();
    graph
}

/// 添加节点到图（只改权威图，不 sync——投影 op 统一刷新视图）。
/// 返回新增节点 id（供投影精确定位）。
pub fn add_node<G: GraphEditable>(
    graph: &mut G,
    editor: &mut GraphEditorState,
    kind: G::Kind,
    x: f32,
    y: f32,
) -> Option<NodeId> {
    let Ok(node_id) = graph.graph_add_node(kind) else {
        log::warn!("节点图: 添加节点失败（模型约束）");
        return None;
    };
    editor.positions.insert(node_id, (x, y));
    Some(node_id)
}

/// 删除节点（自动断开相关连线）。只改权威图，不 sync。
pub fn delete_node<G: GraphEditable>(graph: &mut G, editor: &mut GraphEditorState, index: usize) -> Option<NodeId> {
    let node_id = editor.id_at(index)?;
    graph.graph_delete(node_id);
    editor.positions.remove(&node_id);
    Some(node_id)
}

/// 找喂给某输入端口的连线（模型下标）；无则 -1。拖输入端时用它隐藏该线。
pub fn find_link_for_input<G: GraphEditable>(
    graph: &G,
    editor: &GraphEditorState,
    node_index: usize,
    socket_index: usize,
) -> i32 {
    let Some(node_id) = editor.id_at(node_index) else {
        return -1;
    };
    // socket_index 为输入端口组合下标（输入在前，等于 inputs 内下标）
    let Some(&socket_id) = G::inputs(graph.node(node_id)).get(socket_index) else {
        return -1;
    };
    graph
        .links()
        .iter()
        .position(|link| link.to == socket_id)
        .map_or(-1, |link_index| link_index as i32)
}

/// 找拖拽释放位置命中的、类型匹配的输入端口（世界坐标）。
/// Blender 式"最近距离"判定（参考 `node_find_indicated_socket`）：圆形命中半径
/// = socket-size，多个候选端口同时命中时取距离最近者（拖拽释放应选最近的输入）。
pub fn find_drop_socket<G: GraphEditable>(
    graph: &G,
    editor: &GraphEditorState,
    x: f32,
    y: f32,
    want: SocketType,
) -> SocketHit {
    let radius = editor.metrics.radius;
    let mut best: Option<(f32, usize, usize)> = None;
    for (node_index, &node_id) in editor.order.iter().enumerate() {
        let node = graph.node(node_id);
        let (node_x, node_y) = editor.pos(node_id, node_index);
        for (input_row, &socket_id) in G::inputs(node).iter().enumerate() {
            let socket = graph.socket(socket_id);
            if socket.data_type != want {
                continue;
            }
            // 输入列内 row = input_row（输入列从节点顶部独立排）
            let center_y = node_y + editor.metrics.header
                + input_row as f32 * editor.metrics.row
                + editor.metrics.row / 2.0;
            let dx = x - node_x;
            let dy = y - center_y;
            let distance_sq = dx * dx + dy * dy;
            if distance_sq < radius * radius
                && best.is_none_or(|(best_distance, _, _)| distance_sq < best_distance)
            {
                best = Some((distance_sq, node_index, input_row));
            }
        }
    }
    match best {
        Some((_, node, socket)) => SocketHit {
            found: true,
            node: node as i32,
            socket: socket as i32,
        },
        None => SocketHit {
            found: false,
            node: 0,
            socket: 0,
        },
    }
}

/// 端口拖拽松开：按拖拽语义完成断开/重连。
///  - 拖输入端：源 = 该输入当前连线的输出端；命中新输入则把线搬过去，否则断开。
///  - 拖输出端：命中输入则连接（该输入原有连线被覆盖），否则丢弃。
///
/// 返回 connected 标记发生变化的节点（供投影只 row_changed 这些行，而非全量重建）。
pub fn socket_drag_ended<G: GraphEditable>(
    graph: &mut G,
    editor: &mut GraphEditorState,
    node_index: usize,
    socket_index: usize,
    is_input: bool,
    x: f32,
    y: f32,
) -> Vec<NodeId> {
    let Some(node_id) = editor.id_at(node_index) else {
        return Vec::new();
    };
    let node = graph.node(node_id);
    // socket_index 为组合数组下标（sockets 数组索引：输入 0..in，输出 in..in+out）
    let input_count = G::inputs(node).len();
    let dragged = if is_input {
        G::inputs(node).get(socket_index).copied()
    } else {
        G::outputs(node).get(socket_index - input_count).copied()
    };
    let Some(dragged_socket_id) = dragged else {
        return Vec::new();
    };
    let want = graph.socket(dragged_socket_id).data_type;
    let hit = find_drop_socket(graph, editor, x, y, want);

    // 受影响节点（connected 标记变化的端点）：拖拽节点 + 原连线源 + 命中目标。
    // 量小（≤3）且 set_row 幂等，不去重也可；重复只多一次幂等重建。
    let mut affected: Vec<NodeId> = vec![node_id];

    if is_input {
        // 拖回原位 = 取消（什么都不做）
        if hit.found && hit.node as usize == node_index && hit.socket as usize == socket_index {
            return Vec::new();
        }
        let source = graph
            .links()
            .iter()
            .find(|link| link.to == dragged_socket_id)
            .map(|link| link.from);
        graph.graph_disconnect_input(dragged_socket_id);
        if let Some(src_sid) = source {
            affected.push(graph.socket(src_sid).node);
        }
        if hit.found
            && let (Some(source), Some(target)) = (source, target_socket(graph, editor, &hit))
        {
            let _ = graph.graph_connect(source, target);
            affected.push(graph.socket(target).node);
        }
    } else if hit.found
        && let Some(target) = target_socket(graph, editor, &hit)
    {
        let _ = graph.graph_connect(dragged_socket_id, target);
        affected.push(graph.socket(target).node);
    }

    affected
}

fn target_socket<G: GraphEditable>(
    graph: &G,
    editor: &GraphEditorState,
    hit: &SocketHit,
) -> Option<SocketId> {
    if !hit.found {
        return None;
    }
    let node_id = editor.id_at(hit.node as usize)?;
    // hit.socket 为输入端口组合下标（输入在前，等于 inputs 内下标）
    G::inputs(graph.node(node_id)).get(hit.socket as usize).copied()
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

// ── AppStore 入口（图操作统一分发到当前编辑图） ───────────────────────────────

/// 当前编辑图访问句柄（可变）：携带具体树类型的图引用。
/// 图操作在这里按类型分发——`match` 只存在于本枚举与 `with_graph` 中，
/// 外部调用方一律按 `NodeGraphKind` 走统一入口（Blender `get_from_context` 对应物）。
pub(crate) enum GraphEditorAccess<'a> {
    Style(&'a mut NodeGraph<NodeKind>),
    App(&'a mut NodeGraph<ApplicationNodeKind>),
}

impl<'a> GraphEditorAccess<'a> {
    pub(crate) fn apply_change(
        self,
        editor: &mut GraphEditorState,
        change: &crate::depsgraph::NodeGraphChange,
    ) {
        match self {
            Self::Style(g) => editor.apply_change(g, change),
            Self::App(g) => editor.apply_change(g, change),
        }
    }

    pub(crate) fn add_node(
        self,
        editor: &mut GraphEditorState,
        template_index: usize,
        x: f32,
        y: f32,
    ) -> Option<NodeId> {
        match self {
            Self::Style(g) => {
                let kind =
                    crate::actions::node_graph::style_templates().get(template_index).cloned()?;
                add_node(g, editor, kind, x, y)
            }
            Self::App(g) => {
                let kind = crate::actions::node_graph::app_templates().get(template_index).cloned()?;
                add_node(g, editor, kind, x, y)
            }
        }
    }

    pub(crate) fn delete_node(
        self,
        editor: &mut GraphEditorState,
        node_index: usize,
    ) -> Option<NodeId> {
        match self {
            Self::Style(g) => delete_node(g, editor, node_index),
            Self::App(g) => delete_node(g, editor, node_index),
        }
    }

    pub(crate) fn socket_drag_ended(
        self,
        editor: &mut GraphEditorState,
        node_index: usize,
        socket_index: usize,
        is_input: bool,
        x: f32,
        y: f32,
    ) -> Vec<NodeId> {
        match self {
            Self::Style(g) => {
                socket_drag_ended(g, editor, node_index, socket_index, is_input, x, y)
            }
            Self::App(g) => {
                socket_drag_ended(g, editor, node_index, socket_index, is_input, x, y)
            }
        }
    }

    pub(crate) fn set_reference(
        self,
        editor: &mut GraphEditorState,
        node_index: usize,
        style_id: NodeGraphId,
    ) {
        match self {
            Self::Style(g) => set_reference(g, editor, node_index, style_id),
            Self::App(g) => set_reference(g, editor, node_index, style_id),
        }
    }

    pub(crate) fn set_socket_text_default(
        self,
        editor: &mut GraphEditorState,
        node_index: usize,
        socket_index: usize,
        text: &str,
    ) {
        match self {
            Self::Style(g) => set_socket_text_default(g, editor, node_index, socket_index, text),
            Self::App(g) => set_socket_text_default(g, editor, node_index, socket_index, text),
        }
    }

    pub(crate) fn socket_id_at(
        self,
        editor: &GraphEditorState,
        node_index: usize,
        socket_index: usize,
    ) -> Option<SocketId> {
        match self {
            Self::Style(g) => socket_id_at(g, editor, node_index, socket_index),
            Self::App(g) => socket_id_at(g, editor, node_index, socket_index),
        }
    }

    pub(crate) fn rebuild(self, editor: &mut GraphEditorState) {
        match self {
            Self::Style(g) => editor.rebuild(g),
            Self::App(g) => editor.rebuild(g),
        }
    }

    pub(crate) fn set_default_layout(&mut self, editor: &mut GraphEditorState) {
        if let Self::App(g) = self {
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
            editor.positions = pos;
        }
    }
}

/// 当前编辑图访问句柄（只读）：命中检测/找连线。
pub(crate) enum GraphEditorAccessRo<'a> {
    Style(&'a NodeGraph<NodeKind>),
    App(&'a NodeGraph<ApplicationNodeKind>),
}

impl<'a> GraphEditorAccessRo<'a> {
    pub(crate) fn find_socket_at(
        self,
        editor: &GraphEditorState,
        node: usize,
        x: f32,
        y: f32,
    ) -> i32 {
        match self {
            Self::Style(g) => editor.find_socket_at(g, node, x, y),
            Self::App(g) => editor.find_socket_at(g, node, x, y),
        }
    }

    pub(crate) fn find_link_for_input(
        self,
        editor: &GraphEditorState,
        node: usize,
        sock: usize,
    ) -> i32 {
        match self {
            Self::Style(g) => find_link_for_input(g, editor, node, sock),
            Self::App(g) => find_link_for_input(g, editor, node, sock),
        }
    }

    pub(crate) fn find_drop_socket(
        self,
        editor: &GraphEditorState,
        x: f32,
        y: f32,
        want: SocketType,
    ) -> SocketHit {
        match self {
            Self::Style(g) => find_drop_socket(g, editor, x, y, want),
            Self::App(g) => find_drop_socket(g, editor, x, y, want),
        }
    }
}

impl AppStore {
    /// 统一访问当前编辑图（Blender `get_from_context` 对应物）：按 kind 取编辑器 + 图。
    /// 唯一一处按图类型选择容器/访问路径的分支。
    pub(crate) fn with_graph<R>(
        &mut self,
        kind: NodeGraphKind,
        f: impl FnOnce(&mut GraphEditorState, GraphEditorAccess<'_>) -> R,
    ) -> Option<R> {
        match kind {
            NodeGraphKind::Style => {
                let AppStore { state, ui_models, .. } = self;
                let editor = &mut ui_models.style_graph;
                let id = *editor.style_order.get(editor.current_chain)?;
                let graph = state.project.library.node_graphs.get_mut(&id)?;
                Some(f(editor, GraphEditorAccess::Style(graph)))
            }
            NodeGraphKind::App => {
                let AppStore { state, ui_models, .. } = self;
                let editor = &mut ui_models.app_graph;
                let graph = state.project.style_layer.graphs.get_mut(editor.current_chain)?;
                Some(f(editor, GraphEditorAccess::App(graph)))
            }
        }
    }

    /// 只读访问当前编辑图。
    fn with_graph_ro<R>(
        &self,
        kind: NodeGraphKind,
        f: impl FnOnce(&GraphEditorState, GraphEditorAccessRo<'_>) -> R,
    ) -> Option<R> {
        match kind {
            NodeGraphKind::Style => {
                let AppStore { state, ui_models, .. } = self;
                let editor = &ui_models.style_graph;
                let id = *editor.style_order.get(editor.current_chain)?;
                let graph = state.project.library.node_graphs.get(&id)?;
                Some(f(editor, GraphEditorAccessRo::Style(graph)))
            }
            NodeGraphKind::App => {
                let AppStore { state, ui_models, .. } = self;
                let editor = &ui_models.app_graph;
                let graph = state.project.style_layer.graphs.get(editor.current_chain)?;
                Some(f(editor, GraphEditorAccessRo::App(graph)))
            }
        }
    }

    /// 端口命中（本地坐标）：C1 修复的 Rust 侧实现，与 find_drop_socket 同源。
    pub fn find_socket_at(&self, kind: NodeGraphKind, node: usize, x: f32, y: f32) -> i32 {
        self.with_graph_ro(kind, |editor, access| access.find_socket_at(editor, node, x, y))
            .unwrap_or(-1)
    }

    pub fn set_node_position(&mut self, kind: NodeGraphKind, index: usize, x: f32, y: f32) {
        match kind {
            NodeGraphKind::Style => {
                self.ui_models.style_graph.set_node_position(index, x, y)
            }
            NodeGraphKind::App => {
                self.ui_models.app_graph.set_node_position(index, x, y)
            }
        }
    }

    pub fn find_link_for_input(&self, kind: NodeGraphKind, node: usize, sock: usize) -> i32 {
        self.with_graph_ro(kind, |editor, access| {
            access.find_link_for_input(editor, node, sock)
        })
        .unwrap_or(-1)
    }

    pub fn find_drop_socket(
        &self,
        kind: NodeGraphKind,
        x: f32,
        y: f32,
        dtype: NodeSocketType,
    ) -> SocketHit {
        let want = socket_type_from_slint(dtype);
        self.with_graph_ro(kind, |editor, access| access.find_drop_socket(editor, x, y, want))
            .unwrap_or(SocketHit {
                found: false,
                node: 0,
                socket: 0,
            })
    }

    /// 加载指定链（切换当前编辑图）。
    pub fn load_chain(&mut self, kind: NodeGraphKind, index: usize) {
        match kind {
            NodeGraphKind::Style => self.ui_models.style_graph.current_chain = index,
            NodeGraphKind::App => self.ui_models.app_graph.current_chain = index,
        }
        self.with_graph(kind, |editor, access| access.rebuild(editor));
    }

    /// 默认应用链固定四列布局（初始化/新建时）。设 positions + rebuild（一次性初始化）。
    pub fn layout_default_app_chain(&mut self) {
        self.with_graph(NodeGraphKind::App, |editor, mut access| {
            access.set_default_layout(editor);
            access.rebuild(editor);
        });
    }

    /// 初始化两个编辑器：建链菜单/右键菜单，绑定视图模型到 Slint 全局，加载第 0 条链。
    pub fn initialize_graph_editors(&mut self, instance: &crate::bridge::types::MainWindow) {
        use slint::ComponentHandle;

        let theme = instance.global::<crate::bridge::types::Theme>().get_data().node_editor;

        // 编辑器几何量（端口命中/拖拽用）：从主题取，与 Slint 渲染一致。
        // 缺失则 find_socket_at 半径=0，拖端口端点会误判为「选中节点」。
        let metrics = EditorMetrics {
            header: theme.node_header_height,
            row: theme.node_socket_row_height,
            radius: theme.socket_size,
            node_width: theme.node_width,
        };
        self.ui_models.style_graph.metrics = metrics;
        self.ui_models.app_graph.metrics = metrics;

        // 链菜单（显示顺序在 GUI 派生）
        self.refresh_style_chain_menu();
        self.refresh_app_chain_menu();
        // 角色下拉候选（匹配角色 pattern）
        self.refresh_actors();

        // 右键菜单（色点 = 节点主端口类型颜色）
        let style_menu: Vec<ContextMenuItem> = crate::actions::node_graph::style_templates()
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
        let app_menu: Vec<ContextMenuItem> = crate::actions::node_graph::app_templates()
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

        self.load_chain(NodeGraphKind::Style, 0);
        self.load_chain(NodeGraphKind::App, 0);
        // 默认应用链固定四列布局（初始化时）
        self.layout_default_app_chain();
    }
}

// ── 写操作 dispatch 实现（被 AppStore::dispatch_node_graph 调用，改 model 不打 tag） ──

/// 添加节点（按 kind 选模板列表）。
pub fn add_node_dispatch(
    store: &mut AppStore,
    graph_type: NodeGraphKind,
    template_index: usize,
    x: f32,
    y: f32,
) -> Option<NodeId> {
    store
        .with_graph(graph_type, |editor, access| {
            access.add_node(editor, template_index, x, y)
        })
        .flatten()
}

pub fn delete_node_dispatch(
    store: &mut AppStore,
    graph_type: NodeGraphKind,
    node_index: usize,
) -> Option<NodeId> {
    store
        .with_graph(graph_type, |editor, access| access.delete_node(editor, node_index))
        .flatten()
}

pub fn socket_drag_ended_dispatch(
    store: &mut AppStore,
    graph_type: NodeGraphKind,
    node_index: usize,
    socket_index: usize,
    is_input: bool,
    x: f32,
    y: f32,
) -> Vec<NodeId> {
    store
        .with_graph(graph_type, |editor, access| {
            access.socket_drag_ended(editor, node_index, socket_index, is_input, x, y)
        })
        .unwrap_or_default()
}

pub fn menu_selected_dispatch(
    store: &mut AppStore,
    graph_type: NodeGraphKind,
    node_index: usize,
    menu_index: usize,
) {
    let Some(style_id) = store.ui_models.style_graph.style_order.get(menu_index).copied() else {
        return;
    };
    store.with_graph(graph_type, |editor, access| {
        access.set_reference(editor, node_index, style_id)
    });
}

pub fn socket_option_selected_dispatch(
    store: &mut AppStore,
    graph_type: NodeGraphKind,
    node_index: usize,
    socket_index: usize,
    option_index: usize,
) {
    let actor_names = match graph_type {
        NodeGraphKind::Style => store.ui_models.style_graph.actor_names.clone(),
        NodeGraphKind::App => store.ui_models.app_graph.actor_names.clone(),
    };
    let Some(name) = actor_names.get(option_index).cloned() else {
        return;
    };
    set_socket_text_default_dispatch(store, graph_type, node_index, socket_index, &name);
}

pub fn create_actor_dispatch(
    store: &mut AppStore,
    graph_type: NodeGraphKind,
    node_index: usize,
    socket_index: usize,
    name: &str,
) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    use aweft_model::library::Actor;
    use aweft_model::ids::ActorId;
    store.state_mut().project.library.insert_actor(Actor {
        id: ActorId::new(),
        name: name.clone(),
    });
    store.refresh_actors();
    set_socket_text_default_dispatch(store, graph_type, node_index, socket_index, &name);
}

pub fn set_socket_text_default_dispatch(
    store: &mut AppStore,
    graph_type: NodeGraphKind,
    node_index: usize,
    socket_index: usize,
    text: &str,
) {
    store.with_graph(graph_type, |editor, access| {
        access.set_socket_text_default(editor, node_index, socket_index, text)
    });
}

/// 新建链（reducer：只改 model，返回目标链下标）。链菜单/当前链/渲染全由投影完成。
pub fn new_chain_dispatch(store: &mut AppStore, graph_type: NodeGraphKind) -> usize {
    match graph_type {
        NodeGraphKind::Style => {
            let n = store.state.project.library.node_graphs.len() + 1;
            let graph = new_style_chain(format!("样式链 {n}"));
            let id = graph.id;
            store.state.project.library.node_graphs.insert(id, graph);
            // 目标下标 = 新链在"按名排序"列表中的位置（model 派生，与投影 style_order 一致）
            style_chain_position(store, id).unwrap_or(0)
        }
        NodeGraphKind::App => {
            // 没有样式链可引用时先建一条默认样式链，保证 AppRef 节点存在
            let style_ref = match first_style_chain_id(store) {
                Some(id) => Some(id),
                None => {
                    let n = store.state.project.library.node_graphs.len() + 1;
                    let graph = new_style_chain(format!("样式链 {n}"));
                    let id = graph.id;
                    store.state.project.library.node_graphs.insert(id, graph);
                    Some(id)
                }
            };
            let n = store.state.project.style_layer.graphs.len() + 1;
            // 默认角色：按名排序的第一个 actor（model 派生，不读 UI）
            let first_actor = store
                .state
                .project
                .library
                .actors
                .values()
                .map(|a| a.name.clone())
                .min();
            let graph = new_app_chain(format!("应用链 {n}"), style_ref, first_actor.as_deref());
            store.state.project.style_layer.graphs.push(graph);
            store.state.project.style_layer.graphs.len() - 1
        }
    }
}

/// 第一个样式链（按名排序，与投影 style_order 同款排序）；无则 None。
fn first_style_chain_id(store: &AppStore) -> Option<NodeGraphId> {
    store
        .state
        .project
        .library
        .node_graphs
        .values()
        .min_by(|a, b| a.name.cmp(&b.name))
        .map(|g| g.id)
}

/// 样式链 id 在"按名排序"列表中的位置（与投影 `refresh_style_chain_menu` 的排序一致）。
fn style_chain_position(store: &AppStore, id: NodeGraphId) -> Option<usize> {
    let mut pairs: Vec<(NodeGraphId, String)> = store
        .state
        .project
        .library
        .node_graphs
        .iter()
        .map(|(gid, g)| (*gid, g.name.clone()))
        .collect();
    pairs.sort_by(|a, b| a.1.cmp(&b.1));
    pairs.iter().position(|(gid, _)| *gid == id)
}

// ── 链菜单/角色刷新（供 dispatch 与 initialize 使用） ───────────────────────────

impl AppStore {
    /// 派生样式链/应用链菜单（投影在链列表变化时调用；reducer 只改 model）。
    pub(crate) fn refresh_chain_menus(&mut self) {
        self.refresh_style_chain_menu();
        self.refresh_app_chain_menu();
    }

    /// 设当前链（UI 状态；投影在 ChainSwitched 时设置，供 with_graph 定位目标图）。
    pub(crate) fn set_current_chain(&mut self, kind: NodeGraphKind, index: usize) {
        match kind {
            NodeGraphKind::Style => self.ui_models.style_graph.current_chain = index,
            NodeGraphKind::App => self.ui_models.app_graph.current_chain = index,
        }
    }

    /// 派生样式链切换菜单（模型侧为 id-keyed map，显示顺序按名字排序，属 UI 数据）。
    pub(crate) fn refresh_style_chain_menu(&mut self) {
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
        ui_models.style_graph.style_chain_names =
            pairs.iter().map(|(_, name)| name.clone()).collect();
        // 应用图里 AppRef 节点的下拉菜单也用同一份样式链名字（sync 时读 app_graph 状态）
        ui_models.app_graph.style_chain_names =
            pairs.iter().map(|(_, name)| name.clone()).collect();
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
    pub(crate) fn refresh_app_chain_menu(&mut self) {
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
}

// ── 通用图操作（G: GraphEditable，被 GraphEditorAccess 分发） ─────────────────

/// 设置端口文本默认值（字符串常量输出 / 角色下拉等）。
fn set_socket_text_default<G: GraphEditable>(
    graph: &mut G,
    editor: &mut GraphEditorState,
    node_index: usize,
    socket_index: usize,
    text: &str,
) {
    let value = Some(SocketValue::Text(text.to_string()));
    // socket_index 为组合数组下标（sockets 数组索引：输入 0..in，输出 in..in+out）
    let Some(nid) = editor.id_at(node_index) else {
        return;
    };
    let node = graph.node(nid);
    let in_len = G::inputs(node).len();
    let sid = if socket_index < in_len {
        G::inputs(node).get(socket_index).copied()
    } else {
        G::outputs(node).get(socket_index - in_len).copied()
    };
    if let Some(sid) = sid {
        let _ = graph.graph_set_socket_value(sid, value);
    }
}

/// 取指定图类型某端口的真实 SocketId。
fn socket_id_at<G: GraphEditable>(
    graph: &G,
    editor: &GraphEditorState,
    node_index: usize,
    socket_index: usize,
) -> Option<SocketId> {
    let nid = editor.id_at(node_index)?;
    let node = graph.node(nid);
    let in_len = G::inputs(node).len();
    if socket_index < in_len {
        G::inputs(node).get(socket_index).copied()
    } else {
        G::outputs(node).get(socket_index - in_len).copied()
    }
}

/// 原位改引用节点的引用链：改 kind 内嵌 id + 标脏（不删节点、不动端口/连线）。
/// 投影收到 `ReferenceChanged` 后单行重建该节点（menu_selected/端口/高度）。
fn set_reference<G: GraphEditable>(
    graph: &mut G,
    editor: &mut GraphEditorState,
    node_index: usize,
    style_id: NodeGraphId,
) {
    let Some(node_id) = editor.id_at(node_index) else {
        return;
    };
    let _ = graph.graph_set_reference(node_id, style_id);
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
        // 未知/无主端口类型：灰色兜底，不误用 style 色
        _ => theme.node_muted_text,
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
            node_width: 168.0,
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
        assert!(a.is_some());
        assert!(b.is_some());
        st.rebuild(&g);
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
        st.rebuild(&g);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        st.rebuild(&g);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        st.rebuild(&g);

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
        st.rebuild(&g);
        let src = idx_by_title(&st, "样式源");

        socket_drag_ended(&mut g, &mut st, src, 0, false, 5000.0, 5000.0);
        st.rebuild(&g);
        assert_eq!(st.links.row_count(), 0);
    }

    #[test]
    fn input_drag_to_empty_disconnects() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        st.rebuild(&g);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        st.rebuild(&g);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        st.rebuild(&g);
        assert_eq!(st.links.row_count(), 1);

        // 拖输入端到空白：断开
        socket_drag_ended(&mut g, &mut st, out, 0, true, 5000.0, 5000.0);
        st.rebuild(&g);
        assert_eq!(st.links.row_count(), 0);
        assert_eq!(g.links.len(), 0);
    }

    #[test]
    fn input_drag_moves_connection_to_other_node() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        st.rebuild(&g);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        st.rebuild(&g);
        add_node(&mut g, &mut st, NodeKind::MergeStyle, 500.0, 40.0);
        st.rebuild(&g);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");
        let merge = idx_by_title(&st, "合并样式");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        st.rebuild(&g);
        assert_eq!(st.links.row_count(), 1);

        // 把 组输出 的输入搬到 合并样式 的输入（源保持不变）
        let (nx, ny) = input_center(&st, merge, 0);
        socket_drag_ended(&mut g, &mut st, out, 0, true, nx, ny);
        st.rebuild(&g);

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
        st.rebuild(&g);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        st.rebuild(&g);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        st.rebuild(&g);
        assert_eq!(st.links.row_count(), 1);

        delete_node(&mut g, &mut st, src);
        st.rebuild(&g);
        assert_eq!(st.nodes.row_count(), 1);
        assert_eq!(st.links.row_count(), 0);
    }

    #[test]
    fn find_link_for_input_returns_model_index() {
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        add_node(&mut g, &mut st, NodeKind::StyleSource, 40.0, 40.0);
        st.rebuild(&g);
        add_node(&mut g, &mut st, NodeKind::GroupOutput, 260.0, 40.0);
        st.rebuild(&g);
        let src = idx_by_title(&st, "样式源");
        let out = idx_by_title(&st, "组输出");

        let (tx, ty) = input_center(&st, out, 0);
        socket_drag_ended(&mut g, &mut st, src, 0, false, tx, ty);
        st.rebuild(&g);

        assert_eq!(find_link_for_input(&g, &st, out, 0), 0);
        assert_eq!(find_link_for_input(&g, &st, src, 0), -1);
    }

    #[test]
    fn default_chain_layout_source_left() {
        // 默认样式链（无位置记录）自动布局：样式源在最左、组输出在最右，且确定
        let g = new_style_chain("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        st.rebuild(&g);

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
        st.rebuild(&g);

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
    fn find_socket_at_hits_socket_with_more_than_four_ports() {
        // C1 修复：端口命中 Rust 化，不再受 Slint 硬编码 4 上限约束。
        // 用 GroupOutput 节点（声明可扩展输入）构造 5 个输入端口验证。
        let mut g = NodeGraph::new("t");
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        // GroupOutput 输入数可由 add_node 的 inputs 声明控制
        let decls: Vec<SocketDecl> = (0..5)
            .map(|i| SocketDecl::input(format!("in{i}"), SocketType::Style))
            .collect();
        let id = g
            .add_node::<NodeId>(NodeKind::GroupOutput, &decls, &[])
            .unwrap();
        st.positions.insert(id, (40.0, 40.0));
        st.rebuild(&g);

        let node_idx = st.id_at(0).unwrap();
        assert_eq!(node_idx, id);
        // 第 5 个输入（组合数组下标 4，> 旧硬编码上限 3）：命中其中心（节点左缘 x=0）
        let row_h = st.metrics.row;
        let cy = st.metrics.header + 4.0 * row_h + row_h / 2.0;
        assert_eq!(st.find_socket_at(&g, 0, 0.0, cy), 4);
        // 节点主体中间（非端口）：未命中
        assert_eq!(st.find_socket_at(&g, 0, st.metrics.node_width / 2.0, cy), -1);
        // 越界节点：-1
        assert_eq!(st.find_socket_at(&g, 99, 0.0, 0.0), -1);
    }

    #[test]
    fn sync_is_deterministic_across_switches() {
        // 切换链（加载另一条图再回来）后布局与顺序都保持不变
        let g1 = new_style_chain("a");
        let g2 = new_app_chain("b", None, None);
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        st.rebuild(&g1);
        let before: Vec<(String, f32, f32)> = (0..st.nodes.row_count())
            .map(|i| {
                let r = st.nodes.row_data(i).unwrap();
                (r.title.as_str().to_string(), r.x, r.y)
            })
            .collect();
        st.rebuild(&g2);
        st.rebuild(&g1);
        let after: Vec<(String, f32, f32)> = (0..st.nodes.row_count())
            .map(|i| {
                let r = st.nodes.row_data(i).unwrap();
                (r.title.as_str().to_string(), r.x, r.y)
            })
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn switching_reference_updates_row_in_place() {
        // 回归：换引用链是原位改值，节点不删不增、order 不动、单行重建
        //（早期实现是删旧建新，导致 order 残留旧 id、新引用节点不渲染）。
        let mut g = NodeGraph::new("t");
        let nid = g
            .add_node::<NodeId>(NodeKind::Reference(NodeGraphId::new()), &[], &[])
            .unwrap();
        let mut st = GraphEditorState::default();
        st.metrics = metrics();
        st.positions.insert(nid, (40.0, 40.0));
        st.rebuild(&g);
        assert_eq!(st.nodes.row_count(), 1);

        // 换到另一条链：原位改 kind 内嵌 id + 单行刷新
        let new_id = NodeGraphId::new();
        st.style_order = vec![new_id];
        st.style_chain_names = vec!["目标链".into()];
        set_reference(&mut g, &mut st, 0, new_id);
        st.apply_change(
            &g,
            &crate::depsgraph::NodeGraphChange::ReferenceChanged { node_id: nid },
        );

        // 节点不消失、order 原位不变、引用 id 已改、下拉选中指向新链
        assert_eq!(st.nodes.row_count(), 1);
        assert_eq!(st.id_at(0), Some(nid));
        assert!(matches!(g.nodes[&nid].kind, NodeKind::Reference(id) if id == new_id));
        let row = st.nodes.row_data(0).unwrap();
        assert_eq!(row.menu_selected, 0);
    }
}
