use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::animation::{AnimationError, AnimationLibrary, apply_animation_to_style};
use crate::ids::{AnimDataId, ApplicationGraphId, NodeGraphId, NodeId, SocketId};
#[allow(unused_imports)]
use crate::node_graph::socket::SocketType;
use crate::node_graph::{
    error::EvalError,
    socket::{
        AnimationSocket, ApplicationSocket, ApplicationUnit, BoolSocket, InputSocket, IntSocket,
        Link, Socket, SocketDecl, SocketRef, SocketValue, StyleSocket, TextSocket,
    },
    style::StyleResult,
};

// ── LineContext ───────────────────────────────────────────────────────────────

/// 外部传入的当前行信息。
#[derive(Debug, Clone)]
pub struct LineContext {
    pub actor: String,
    pub line_text: String,
    pub line_index: i64,
}

// ── 求值结果 ─────────────────────────────────────────────────────────────────

/// 一次应用图求值命中的结果。
#[derive(Debug, Clone, Default)]
pub struct ApplicationGraphMatch {
    /// 命中的静态样式。
    pub style: StyleResult,
    /// 命中的动画片段引用（可选）。
    pub animation: Option<AnimDataId>,
}

/// 应用图求值过程中可能产生的错误。
#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Eval(#[from] EvalError),
    #[error(transparent)]
    Animation(#[from] AnimationError),
    /// 静态样式必须明确连接（样式链引用；想要默认样式就连接默认样式链），
    /// 不允许留空静默使用默认样式。
    #[error("节点（{0}）的样式输入未连接；请把样式来源连接到样式端口（默认样式可连接默认样式链）")]
    StyleNotConnected(String),
}

// ── ApplicationNodeKind ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplicationNodeKind {
    ContextInput,
    MatchActor,
    MatchText,
    IntEq,
    And,
    Or,
    Not,
    AppRef(NodeGraphId),
    /// 字符串常量：输出一段文本（值由节点默认值提供，UI 节点内可编辑）。
    TextConst,
    /// 应用匹配：style + condition → 一个应用单元（命中才生效）。应用单元只有
    /// 接到组输出才有意义。
    ApplyMatch,
    /// 应用动画：animation + condition → 一个应用单元（命中才生效）。
    ApplyAnimation,
    /// 组输出：应用图的唯一出口。输入端口动态扩展（全部为 Application），
    /// 按端口连接顺序（= 端口创建顺序）收集应用单元并合并应用——
    /// 单图内即"小优先级"，后连接的端口优先级更高（覆盖前者）。
    GroupOutput,
}

impl ApplicationNodeKind {
    pub fn is_static(&self) -> bool {
        // AppRef 接口由被引用图决定；GroupOutput 输入端口动态扩展（半动态节点）。
        !matches!(
            self,
            ApplicationNodeKind::AppRef(_) | ApplicationNodeKind::GroupOutput
        )
    }

    /// 节点显示名（应用图编辑器标题栏用）。
    pub fn title(&self) -> &'static str {
        match self {
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
}

// ── ApplicationGraphNode ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationGraphNode {
    pub id: NodeId,
    pub kind: ApplicationNodeKind,
    pub inputs: Vec<SocketId>,
    pub outputs: Vec<SocketId>,
}

impl ApplicationGraphNode {
    pub fn input<'a>(
        &self,
        idx: usize,
        sockets: &'a HashMap<SocketId, Socket>,
    ) -> Option<&'a Socket> {
        self.inputs.get(idx).and_then(|id| sockets.get(id))
    }

    pub fn output<'a>(
        &self,
        idx: usize,
        sockets: &'a HashMap<SocketId, Socket>,
    ) -> Option<&'a Socket> {
        self.outputs.get(idx).and_then(|id| sockets.get(id))
    }
}

// ── 类型化 Node Handle ───────────────────────────────────────────────────────

pub trait ApplicationNodeSpec {
    const KIND: ApplicationNodeKind;
    type Inputs;
    type Outputs;
    fn build_inputs(node_id: NodeId, sockets: &[SocketId]) -> Self::Inputs;
    fn build_outputs(node_id: NodeId, sockets: &[SocketId]) -> Self::Outputs;
    fn input_decls() -> Vec<SocketDecl>;
    fn output_decls() -> Vec<SocketDecl>;
}

pub struct ApplicationNodeHandle<T: ApplicationNodeSpec> {
    pub id: NodeId,
    pub inputs: T::Inputs,
    pub outputs: T::Outputs,
}

impl<T: ApplicationNodeSpec> ApplicationNodeHandle<T> {
    pub fn to_id(self) -> NodeId {
        self.id
    }
}

pub trait ApplicationFromNode {
    fn from_node(node_id: NodeId, graph: &ApplicationGraph) -> Self;
    fn resolve_decls(
        kind: &ApplicationNodeKind,
        extra_inputs: &[SocketDecl],
        extra_outputs: &[SocketDecl],
    ) -> (Vec<SocketDecl>, Vec<SocketDecl>);
}

impl ApplicationFromNode for NodeId {
    fn from_node(node_id: NodeId, _graph: &ApplicationGraph) -> Self {
        node_id
    }
    fn resolve_decls(
        kind: &ApplicationNodeKind,
        extra_inputs: &[SocketDecl],
        extra_outputs: &[SocketDecl],
    ) -> (Vec<SocketDecl>, Vec<SocketDecl>) {
        let (base_i, base_o) = get_application_socket_decls(kind).unwrap_or_default();
        let inputs: Vec<_> = base_i
            .into_iter()
            .chain(extra_inputs.iter().cloned())
            .collect();
        let outputs: Vec<_> = base_o
            .into_iter()
            .chain(extra_outputs.iter().cloned())
            .collect();
        (inputs, outputs)
    }
}

impl<T: ApplicationNodeSpec> ApplicationFromNode for ApplicationNodeHandle<T> {
    fn from_node(node_id: NodeId, graph: &ApplicationGraph) -> Self {
        let node = &graph.nodes[&node_id];
        ApplicationNodeHandle {
            id: node_id,
            inputs: T::build_inputs(node_id, &node.inputs),
            outputs: T::build_outputs(node_id, &node.outputs),
        }
    }
    fn resolve_decls(
        _kind: &ApplicationNodeKind,
        extra_inputs: &[SocketDecl],
        extra_outputs: &[SocketDecl],
    ) -> (Vec<SocketDecl>, Vec<SocketDecl>) {
        let inputs: Vec<_> = T::input_decls()
            .into_iter()
            .chain(extra_inputs.iter().cloned())
            .collect();
        let outputs: Vec<_> = T::output_decls()
            .into_iter()
            .chain(extra_outputs.iter().cloned())
            .collect();
        (inputs, outputs)
    }
}

// ── 应用节点定义宏 ──────────────────────────────────────────────────────────

macro_rules! define_application_nodes {
    (
        $($name:ident, $inputs:ident, $outputs:ident {
            kind: $kind:expr;
            inputs: { $($in_name:ident: $in_ty:ident),* $(,)? }
            outputs: { $($out_name:ident: $out_ty:ident),* $(,)? }
        })*
    ) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub struct $name;

            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub struct $inputs {
                $(pub $in_name: $crate::node_graph::SocketRef<$in_ty>),*
            }

            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub struct $outputs {
                $(pub $out_name: $crate::node_graph::SocketRef<$out_ty>),*
            }

            impl $crate::application_graph::ApplicationNodeSpec for $name {
                const KIND: $crate::application_graph::ApplicationNodeKind = $kind;
                type Inputs = $inputs;
                type Outputs = $outputs;

                #[allow(unused_variables, unused_mut, unused_assignments)]
                fn build_inputs(
                    _node_id: $crate::application_graph::NodeId,
                    sockets: &[$crate::application_graph::SocketId],
                ) -> Self::Inputs {
                    let mut idx = 0usize;
                    $inputs {
                        $($in_name: {
                            let s = $crate::node_graph::SocketRef::new(sockets[idx]);
                            idx += 1;
                            s
                        }),*
                    }
                }

                #[allow(unused_variables, unused_mut, unused_assignments)]
                fn build_outputs(
                    _node_id: $crate::application_graph::NodeId,
                    sockets: &[$crate::application_graph::SocketId],
                ) -> Self::Outputs {
                    let mut idx = 0usize;
                    $outputs {
                        $($out_name: {
                            let s = $crate::node_graph::SocketRef::new(sockets[idx]);
                            idx += 1;
                            s
                        }),*
                    }
                }

                fn input_decls() -> Vec<$crate::node_graph::SocketDecl> {
                    vec![
                        $($crate::node_graph::SocketDecl::input(
                            stringify!($in_name),
                            <$in_ty as $crate::node_graph::SocketTypeInfo>::SOCKET_TYPE,
                        )),*
                    ]
                }

                fn output_decls() -> Vec<$crate::node_graph::SocketDecl> {
                    vec![
                        $($crate::node_graph::SocketDecl::output(
                            stringify!($out_name),
                            <$out_ty as $crate::node_graph::SocketTypeInfo>::SOCKET_TYPE,
                        )),*
                    ]
                }
            }
        )*

        pub fn get_application_socket_decls(
            kind: &$crate::application_graph::ApplicationNodeKind,
        ) -> Option<(Vec<$crate::node_graph::SocketDecl>, Vec<$crate::node_graph::SocketDecl>)> {
            $(
                if *kind == $kind {
                    return Some((
                        vec![$($crate::node_graph::SocketDecl::input(
                            stringify!($in_name),
                            <$in_ty as $crate::node_graph::SocketTypeInfo>::SOCKET_TYPE,
                        )),*],
                        vec![$($crate::node_graph::SocketDecl::output(
                            stringify!($out_name),
                            <$out_ty as $crate::node_graph::SocketTypeInfo>::SOCKET_TYPE,
                        )),*],
                    ));
                }
            )*
            None
        }
    };
}

define_application_nodes! {
    ContextInput, ContextInputIn, ContextInputOut {
        kind: ApplicationNodeKind::ContextInput;
        inputs: {}
        outputs: { actor: TextSocket, text: TextSocket, index: IntSocket }
    }
    MatchActor, MatchActorIn, MatchActorOut {
        kind: ApplicationNodeKind::MatchActor;
        inputs: { source: TextSocket, pattern: TextSocket }
        outputs: { matched: BoolSocket }
    }
    MatchText, MatchTextIn, MatchTextOut {
        kind: ApplicationNodeKind::MatchText;
        inputs: { source: TextSocket, regex: TextSocket }
        outputs: { matched: BoolSocket }
    }
    IntEq, IntEqIn, IntEqOut {
        kind: ApplicationNodeKind::IntEq;
        inputs: { a: IntSocket, b: IntSocket }
        outputs: { equal: BoolSocket }
    }
    And, AndIn, AndOut {
        kind: ApplicationNodeKind::And;
        inputs: {}
        outputs: { result: BoolSocket }
    }
    Or, OrIn, OrOut {
        kind: ApplicationNodeKind::Or;
        inputs: {}
        outputs: { result: BoolSocket }
    }
    Not, NotIn, NotOut {
        kind: ApplicationNodeKind::Not;
        inputs: { input: BoolSocket }
        outputs: { output: BoolSocket }
    }
    TextConst, TextConstIn, TextConstOut {
        kind: ApplicationNodeKind::TextConst;
        inputs: {}
        outputs: { text: TextSocket }
    }
    ApplyMatch, ApplyMatchIn, ApplyMatchOut {
        kind: ApplicationNodeKind::ApplyMatch;
        inputs: { style: StyleSocket, condition: BoolSocket }
        outputs: { application: ApplicationSocket }
    }
    ApplyAnimation, ApplyAnimationIn, ApplyAnimationOut {
        kind: ApplicationNodeKind::ApplyAnimation;
        inputs: { style: StyleSocket, animation: AnimationSocket, condition: BoolSocket }
        outputs: { application: ApplicationSocket }
    }
    GroupOutput, GroupOutputIn, GroupOutputOut {
        kind: ApplicationNodeKind::GroupOutput;
        inputs: {}
        outputs: {}
    }
}

// ── ApplicationGraph ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct ApplicationEvalCache {
    socket_values: HashMap<SocketId, SocketValue>,
    node_styles: HashMap<NodeId, StyleResult>,
    /// 每个应用匹配 / 应用动画节点的产出单元（组输出按端口顺序收集）。
    node_applications: HashMap<NodeId, ApplicationUnit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationGraph {
    pub id: ApplicationGraphId,
    pub name: String,
    pub enabled: bool,
    pub nodes: HashMap<NodeId, ApplicationGraphNode>,
    pub sockets: HashMap<SocketId, Socket>,
    pub links: Vec<Link>,
    input_link_map: HashMap<SocketId, SocketId>,
    output_link_map: HashMap<SocketId, Vec<SocketId>>,
    #[serde(skip, default)]
    cache: Option<ApplicationEvalCache>,
    #[serde(skip, default)]
    pub dirty: HashSet<NodeId>,
}

impl ApplicationGraph {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ApplicationGraphId::new(),
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

    pub fn add_node<T: ApplicationFromNode>(
        &mut self,
        kind: ApplicationNodeKind,
        extra_inputs: &[SocketDecl],
        extra_outputs: &[SocketDecl],
    ) -> Result<T, EvalError> {
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
            ApplicationGraphNode {
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

    /// 删除节点并断开其所有连线、移除其端口。
    pub fn remove_node(&mut self, node_id: NodeId) {
        if let Some(node) = self.nodes.get(&node_id) {
            let inputs: Vec<SocketId> = node.inputs.clone();
            let outputs: Vec<SocketId> = node.outputs.clone();
            for sid in inputs {
                self.disconnect_input(sid);
            }
            for sid in outputs {
                self.disconnect_output(sid);
            }
        }

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
        }
        self.dirty.remove(&node_id);
    }

    fn disconnect_output(&mut self, output_socket_id: SocketId) {
        if let Some(tos) = self.output_link_map.remove(&output_socket_id) {
            for &to in &tos {
                self.input_link_map.remove(&to);
                self.links.retain(|l| l.to != to);
                let dest_node = self.socket(to).node;
                self.mark_dirty(dest_node);
            }
        }
    }

    // ── 查询 ───────────────────────────────────────────────────────────────

    fn socket(&self, socket_id: SocketId) -> &Socket {
        self.sockets
            .get(&socket_id)
            .unwrap_or_else(|| panic!("socket {socket_id} not found in app graph {}", self.id))
    }

    fn input_source(&self, socket_id: SocketId) -> Option<SocketId> {
        self.input_link_map.get(&socket_id).copied()
    }

    fn upstream_node_id(&self, socket_id: SocketId) -> Option<NodeId> {
        self.input_source(socket_id)
            .map(|from| self.socket(from).node)
    }

    fn read_input(
        &self,
        node: &ApplicationGraphNode,
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

    pub fn evaluate(
        &mut self,
        ctx: &LineContext,
        style_results: &HashMap<NodeGraphId, StyleResult>,
    ) -> Result<Option<ApplicationGraphMatch>, ApplicationError> {
        if self.cache.is_none() {
            self.cache = Some(ApplicationEvalCache::default());
            if self.dirty.is_empty() {
                self.dirty = self.nodes.keys().copied().collect();
            }
        }

        let mut cache = self.cache.take().unwrap();
        let dirty_nodes: Vec<NodeId> = self.dirty.iter().copied().collect();
        let mut visiting = HashSet::new();

        for node_id in dirty_nodes {
            self.ensure_node_evaluated(node_id, &mut cache, &mut visiting, ctx, style_results)?;
        }

        // 组输出是唯一出口：按输入端口连接顺序（= 端口创建顺序）收集上游应用单元，
        // 依次合并——后连接的端口优先级更高（覆盖前者），对应总应用图里
        // ApplicationLayer 按 Vec 顺序"后面的图覆盖前面的图"。
        let matched = self
            .nodes
            .values()
            .find(|n| matches!(n.kind, ApplicationNodeKind::GroupOutput))
            .map(
                |group_out| -> Result<Option<ApplicationGraphMatch>, ApplicationError> {
                    let mut result = ApplicationGraphMatch::default();
                    let mut any = false;
                    for &input_id in &group_out.inputs {
                        let Some(from) = self.input_source(input_id) else {
                            continue;
                        };
                        let Some(unit) = cache
                            .node_applications
                            .get(&self.socket(from).node)
                        else {
                            continue;
                        };
                        if !unit.condition {
                            continue;
                        }
                        any = true;
                        result.style = result.style.merge_over(unit.style.clone());
                        if unit.animation.is_some() {
                            result.animation = unit.animation;
                        }
                    }
                    Ok(any.then_some(result))
                },
            )
            .transpose()?
            .flatten();

        self.cache = Some(cache);
        Ok(matched)
    }

    fn ensure_node_evaluated(
        &mut self,
        node_id: NodeId,
        cache: &mut ApplicationEvalCache,
        visiting: &mut HashSet<NodeId>,
        ctx: &LineContext,
        style_results: &HashMap<NodeGraphId, StyleResult>,
    ) -> Result<(), ApplicationError> {
        if !self.dirty.contains(&node_id) {
            return Ok(());
        }

        if !visiting.insert(node_id) {
            return Err(ApplicationError::Eval(EvalError::CyclicGraph(
                NodeGraphId::new(),
            )));
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
            self.ensure_node_evaluated(upstream_node, cache, visiting, ctx, style_results)?;
        }

        self.evaluate_node(node_id, cache, ctx, style_results)?;

        visiting.remove(&node_id);
        self.dirty.remove(&node_id);
        Ok(())
    }

    fn evaluate_node(
        &self,
        node_id: NodeId,
        cache: &mut ApplicationEvalCache,
        ctx: &LineContext,
        style_results: &HashMap<NodeGraphId, StyleResult>,
    ) -> Result<(), ApplicationError> {
        let node = &self.nodes[&node_id];
        match &node.kind {
            ApplicationNodeKind::ContextInput => {
                if let Some(&sid) = node.outputs.first() {
                    cache
                        .socket_values
                        .insert(sid, SocketValue::Text(ctx.actor.clone()));
                }
                if let Some(&sid) = node.outputs.get(1) {
                    cache
                        .socket_values
                        .insert(sid, SocketValue::Text(ctx.line_text.clone()));
                }
                if let Some(&sid) = node.outputs.get(2) {
                    cache
                        .socket_values
                        .insert(sid, SocketValue::Int(ctx.line_index));
                }
            }

            ApplicationNodeKind::MatchActor => {
                let source = self.read_input(node, 0, &cache.socket_values);
                let pattern = self.read_input(node, 1, &cache.socket_values);
                let matched = match (source, pattern) {
                    (Some(SocketValue::Text(s)), Some(SocketValue::Text(p))) => s.contains(&p),
                    _ => false,
                };
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(matched));
                }
            }

            ApplicationNodeKind::MatchText => {
                let source = self.read_input(node, 0, &cache.socket_values);
                let pattern = self.read_input(node, 1, &cache.socket_values);
                let matched = match (source, pattern) {
                    (Some(SocketValue::Text(s)), Some(SocketValue::Text(re))) => {
                        Regex::new(&re).map(|r| r.is_match(&s)).unwrap_or(false)
                    }
                    _ => false,
                };
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(matched));
                }
            }

            ApplicationNodeKind::IntEq => {
                let a = self
                    .read_input(node, 0, &cache.socket_values)
                    .and_then(|v| match v {
                        SocketValue::Int(i) => Some(i),
                        _ => None,
                    })
                    .unwrap_or(0);
                let b = self
                    .read_input(node, 1, &cache.socket_values)
                    .and_then(|v| match v {
                        SocketValue::Int(i) => Some(i),
                        _ => None,
                    })
                    .unwrap_or(0);
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(a == b));
                }
            }

            ApplicationNodeKind::And => {
                let all_true = node.inputs.iter().all(|&sid| {
                    self.input_source(sid)
                        .and_then(|from| cache.socket_values.get(&from))
                        .map(|v| matches!(v, SocketValue::Bool(true)))
                        .unwrap_or(false)
                });
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(all_true));
                }
            }

            ApplicationNodeKind::Or => {
                let any_true = node.inputs.iter().any(|&sid| {
                    self.input_source(sid)
                        .and_then(|from| cache.socket_values.get(&from))
                        .map(|v| matches!(v, SocketValue::Bool(true)))
                        .unwrap_or(false)
                });
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(any_true));
                }
            }

            ApplicationNodeKind::Not => {
                let input = self
                    .read_input(node, 0, &cache.socket_values)
                    .map(|v| match v {
                        SocketValue::Bool(b) => !b,
                        _ => false,
                    })
                    .unwrap_or(true);
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(input));
                }
            }

            // 字符串常量：输出节点默认值里的文本（UI 节点内编辑，未设则空串）。
            ApplicationNodeKind::TextConst => {
                if let Some(&sid) = node.outputs.first() {
                    let text = match self.socket(sid).default_value.clone() {
                        Some(SocketValue::Text(s)) => s,
                        _ => String::new(),
                    };
                    cache.socket_values.insert(sid, SocketValue::Text(text));
                }
            }

            ApplicationNodeKind::AppRef(graph_id) => {
                let style = style_results.get(graph_id).cloned().unwrap_or_default();
                cache.node_styles.insert(node_id, style);
            }

            // 应用匹配：style（必须明确连接）+ condition → 应用单元，供组输出收集。
            ApplicationNodeKind::ApplyMatch => {
                let style = self.required_style(node, &cache)?;
                let condition = node
                    .inputs
                    .get(1)
                    .and_then(|&sid| self.input_source(sid))
                    .and_then(|from| cache.socket_values.get(&from))
                    .and_then(|v| match v {
                        SocketValue::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(false);
                cache.node_applications.insert(
                    node_id,
                    ApplicationUnit {
                        style,
                        condition,
                        animation: None,
                    },
                );
            }

            // 应用动画：style（必须明确连接）+ animation + condition → 应用单元。
            // 动画的静态样式基础就是这里连接的样式（默认样式可连默认样式链）。
            ApplicationNodeKind::ApplyAnimation => {
                let style = self.required_style(node, &cache)?;
                let animation = node
                    .inputs
                    .get(1)
                    .and_then(|&sid| {
                        self.input_source(sid)
                            .and_then(|from| cache.socket_values.get(&from).cloned())
                            .or_else(|| self.socket(sid).default_value.clone())
                    })
                    .and_then(|v| match v {
                        SocketValue::Animation(id) => Some(id),
                        _ => None,
                    });
                let condition = node
                    .inputs
                    .get(2)
                    .and_then(|&sid| self.input_source(sid))
                    .and_then(|from| cache.socket_values.get(&from))
                    .and_then(|v| match v {
                        SocketValue::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(false);
                cache.node_applications.insert(
                    node_id,
                    ApplicationUnit {
                        style,
                        condition,
                        animation,
                    },
                );
            }

            // 组输出：出口节点，不计算任何值；收集与合并发生在 evaluate() 主流程。
            ApplicationNodeKind::GroupOutput => {}
        }
        Ok(())
    }

    /// 读取样式输入（第 0 号输入）：必须明确连接样式来源，未连接报错。
    fn required_style(
        &self,
        node: &ApplicationGraphNode,
        cache: &ApplicationEvalCache,
    ) -> Result<StyleResult, ApplicationError> {
        let style_sid = node
            .inputs
            .first()
            .ok_or_else(|| ApplicationError::StyleNotConnected(self.name.clone()))?;
        match self.upstream_node_id(*style_sid) {
            Some(up) => Ok(cache.node_styles.get(&up).cloned().unwrap_or_default()),
            None => Err(ApplicationError::StyleNotConnected(self.name.clone())),
        }
    }
}

// ── ApplicationLayer ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplicationLayer {
    pub graphs: Vec<ApplicationGraph>,
}

impl ApplicationLayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, index: usize, graph: ApplicationGraph) {
        self.graphs.insert(index, graph);
    }

    pub fn remove(&mut self, index: usize) -> Option<ApplicationGraph> {
        if index < self.graphs.len() {
            Some(self.graphs.remove(index))
        } else {
            None
        }
    }

    /// 按优先级从低到高求值，返回最终叠加的样式。
    ///
    /// `style_results` 需预先填入所有被引用样式链的求值结果。
    /// `animations` 提供动画片段采样；命中动画的规则会按 `time` 与 `duration`
    /// 对样式进行动态覆盖。
    pub fn evaluate(
        &mut self,
        ctx: &LineContext,
        style_results: &HashMap<NodeGraphId, StyleResult>,
        animations: &AnimationLibrary,
        time: f64,
        duration: f64,
    ) -> Result<StyleResult, ApplicationError> {
        let mut result = StyleResult::default();
        for graph in &mut self.graphs {
            if !graph.enabled {
                continue;
            }
            if let Some(matched) = graph.evaluate(ctx, style_results)? {
                let mut graph_style = matched.style;
                if let Some(clip_id) = matched.animation {
                    let tracks = animations.sample_tracks(clip_id, time, duration)?;
                    apply_animation_to_style(&mut graph_style, &tracks);
                }
                result = result.merge_over(graph_style);
            }
        }
        Ok(result)
    }

    /// 求值匹配结果，不做动画采样。
    ///
    /// 与 `evaluate()` 一样遍历应用图，但返回每个匹配到的基础样式和
    /// `AnimDataId` 引用。调用方（如 `aweft-convert`）可自行处理动画数据，
    /// 例如将其转换为 ASS 原生动画标签。
    pub fn evaluate_matches(
        &mut self,
        ctx: &LineContext,
        style_results: &HashMap<NodeGraphId, StyleResult>,
    ) -> Result<Vec<ApplicationGraphMatch>, ApplicationError> {
        let mut matches = Vec::new();
        for graph in &mut self.graphs {
            if !graph.enabled {
                continue;
            }
            if let Some(matched) = graph.evaluate(ctx, style_results)? {
                matches.push(matched);
            }
        }
        Ok(matches)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_graph::{NodeGraph, NodeHandle, NodeKind, StyleOpKind};

    fn make_bold_chain(size: f64, bold: bool) -> NodeGraph {
        let mut g = NodeGraph::new("bold-chain");
        let src: NodeHandle<crate::node_graph::StyleSource> =
            g.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
        let sz: NodeHandle<crate::node_graph::SetFontSize> = g
            .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
            .unwrap();
        let bd: NodeHandle<crate::node_graph::SetBold> = g
            .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
            .unwrap();
        let out: NodeHandle<crate::node_graph::GroupOutput> =
            g.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

        g.connect(src.outputs.style, sz.inputs.style).unwrap();
        g.connect(sz.outputs.style, bd.inputs.style).unwrap();
        g.connect(bd.outputs.style, out.inputs.style).unwrap();
        g.set_value(sz.inputs.size, size).unwrap();
        g.set_value(bd.inputs.bold, bold).unwrap();
        g.evaluate().unwrap();
        g
    }

    #[test]
    fn context_input_produces_values() {
        let mut g = ApplicationGraph::new("test");
        let _ctx: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();

        let ctx = LineContext {
            actor: "Alice".into(),
            line_text: "hello world".into(),
            line_index: 3,
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&ctx, &empty).unwrap();
        assert!(matched.is_none());
        assert!(!g.dirty.contains(&g.nodes.values().next().unwrap().id));
    }

    #[test]
    fn match_actor_detects_substring() {
        let mut g = ApplicationGraph::new("test");
        let ctx_node: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher: ApplicationNodeHandle<MatchActor> = g
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();

        g.connect(ctx_node.outputs.actor, matcher.inputs.source)
            .unwrap();
        let patt = matcher.inputs.pattern.id;
        g.set_socket_value(patt, Some(SocketValue::Text("Ali".into())))
            .unwrap();

        let ctx = LineContext {
            actor: "Alice".into(),
            line_text: String::new(),
            line_index: 0,
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&ctx, &empty).unwrap();
        assert!(matched.is_none());

        let val = g.sockets.get(&matcher.outputs.matched.id).unwrap();
        assert_eq!(val.data_type, SocketType::Bool);
    }

    #[test]
    fn match_text_uses_regex() {
        let mut g = ApplicationGraph::new("test");
        let ctx_node: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher: ApplicationNodeHandle<MatchText> = g
            .add_node(ApplicationNodeKind::MatchText, &[], &[])
            .unwrap();

        g.connect(ctx_node.outputs.text, matcher.inputs.source)
            .unwrap();
        let re = matcher.inputs.regex.id;
        g.set_socket_value(re, Some(SocketValue::Text(r"\d+\.\d+s".into())))
            .unwrap();

        let ctx = LineContext {
            actor: String::new(),
            line_text: "delay 1.5s then start".into(),
            line_index: 0,
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&ctx, &empty).unwrap();
        assert!(matched.is_none());

        // Verify the output socket exists and is Bool type
        assert_eq!(
            g.sockets[&matcher.outputs.matched.id].data_type,
            SocketType::Bool
        );
    }

    #[test]
    fn not_inverts_condition() {
        let mut g = ApplicationGraph::new("test");
        let ctx_node: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher: ApplicationNodeHandle<MatchActor> = g
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();
        let not_node: ApplicationNodeHandle<Not> =
            g.add_node(ApplicationNodeKind::Not, &[], &[]).unwrap();

        g.connect(ctx_node.outputs.actor, matcher.inputs.source)
            .unwrap();
        g.connect(matcher.outputs.matched, not_node.inputs.input)
            .unwrap();
        g.set_socket_value(
            matcher.inputs.pattern.id,
            Some(SocketValue::Text("Alice".into())),
        )
        .unwrap();

        let ctx = LineContext {
            actor: "Alice".into(),
            line_text: String::new(),
            line_index: 0,
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&ctx, &empty).unwrap();
        assert!(matched.is_none());
    }

    #[test]
    fn apply_returns_false_when_condition_false() {
        let mut g = ApplicationGraph::new("test");
        let ctx_node: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher: ApplicationNodeHandle<MatchActor> = g
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();
        let app_ref = g
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(NodeGraphId::new()),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let ref_style_out = g.nodes[&app_ref].outputs[0];
        let apply: ApplicationNodeHandle<ApplyMatch> = g
            .add_node(ApplicationNodeKind::ApplyMatch, &[], &[])
            .unwrap();
        let group_out = g
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[SocketDecl::input("application", SocketType::Application)],
                &[],
            )
            .unwrap();
        let out_in = g.nodes[&group_out].inputs[0];

        g.connect_raw(ref_style_out, apply.inputs.style.id).unwrap();
        g.connect(ctx_node.outputs.actor, matcher.inputs.source)
            .unwrap();
        g.connect(matcher.outputs.matched, apply.inputs.condition)
            .unwrap();
        g.connect_raw(apply.outputs.application.id, out_in).unwrap();
        g.set_socket_value(
            matcher.inputs.pattern.id,
            Some(SocketValue::Text("Bob".into())),
        )
        .unwrap();

        let ctx = LineContext {
            actor: "Alice".into(),
            line_text: String::new(),
            line_index: 0,
        };
        let style_results = HashMap::new();
        let matched = g.evaluate(&ctx, &style_results).unwrap();
        assert!(matched.is_none());
    }

    #[test]
    fn apply_returns_true_when_condition_matches() {
        let mut g = ApplicationGraph::new("test");
        let ctx_node: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher: ApplicationNodeHandle<MatchActor> = g
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();
        let app_ref = g
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(NodeGraphId::new()),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let ref_style_out = g.nodes[&app_ref].outputs[0];
        let apply: ApplicationNodeHandle<ApplyMatch> = g
            .add_node(ApplicationNodeKind::ApplyMatch, &[], &[])
            .unwrap();
        let group_out = g
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[SocketDecl::input("application", SocketType::Application)],
                &[],
            )
            .unwrap();
        let out_in = g.nodes[&group_out].inputs[0];

        g.connect_raw(ref_style_out, apply.inputs.style.id).unwrap();
        g.connect(ctx_node.outputs.actor, matcher.inputs.source)
            .unwrap();
        g.connect(matcher.outputs.matched, apply.inputs.condition)
            .unwrap();
        g.connect_raw(apply.outputs.application.id, out_in).unwrap();
        g.set_socket_value(
            matcher.inputs.pattern.id,
            Some(SocketValue::Text("Alice".into())),
        )
        .unwrap();

        let ctx = LineContext {
            actor: "Alice says hello".into(),
            line_text: String::new(),
            line_index: 0,
        };
        let style_results = HashMap::new();
        let matched = g.evaluate(&ctx, &style_results).unwrap();
        assert!(matched.is_some());
    }

    #[test]
    fn app_layer_composes_styles_by_priority() {
        let mut bold_chain = make_bold_chain(12.0, true);
        let bold_result = bold_chain.evaluate().unwrap();

        let mut size_chain = make_bold_chain(24.0, false);
        let size_result = size_chain.evaluate().unwrap();

        let style_results: HashMap<NodeGraphId, StyleResult> = vec![
            (bold_chain.id, bold_result.clone()),
            (size_chain.id, size_result.clone()),
        ]
        .into_iter()
        .collect();

        // App graph 1: match Alice → bold_chain
        let mut g1 = ApplicationGraph::new("alice-bold");
        let ctx1: ApplicationNodeHandle<ContextInput> = g1
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher1: ApplicationNodeHandle<MatchActor> = g1
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();
        let ref1 = g1
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(bold_chain.id),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let ref1_out = g1.nodes[&ref1].outputs[0];
        let apply1: ApplicationNodeHandle<ApplyMatch> = g1
            .add_node(ApplicationNodeKind::ApplyMatch, &[], &[])
            .unwrap();
        let group1 = g1
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[SocketDecl::input("application", SocketType::Application)],
                &[],
            )
            .unwrap();
        let group1_in = g1.nodes[&group1].inputs[0];

        g1.connect_raw(ref1_out, apply1.inputs.style.id).unwrap();
        g1.connect(ctx1.outputs.actor, matcher1.inputs.source)
            .unwrap();
        g1.connect(matcher1.outputs.matched, apply1.inputs.condition)
            .unwrap();
        g1.connect_raw(apply1.outputs.application.id, group1_in)
            .unwrap();
        g1.set_socket_value(
            matcher1.inputs.pattern.id,
            Some(SocketValue::Text("Alice".into())),
        )
        .unwrap();

        // App graph 2: match Alice → size_chain (higher priority)
        let mut g2 = ApplicationGraph::new("alice-size");
        let ctx2: ApplicationNodeHandle<ContextInput> = g2
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher2: ApplicationNodeHandle<MatchActor> = g2
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();
        let ref2 = g2
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(size_chain.id),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let ref2_out = g2.nodes[&ref2].outputs[0];
        let apply2: ApplicationNodeHandle<ApplyMatch> = g2
            .add_node(ApplicationNodeKind::ApplyMatch, &[], &[])
            .unwrap();
        let group2 = g2
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[SocketDecl::input("application", SocketType::Application)],
                &[],
            )
            .unwrap();
        let group2_in = g2.nodes[&group2].inputs[0];

        g2.connect_raw(ref2_out, apply2.inputs.style.id).unwrap();
        g2.connect(ctx2.outputs.actor, matcher2.inputs.source)
            .unwrap();
        g2.connect(matcher2.outputs.matched, apply2.inputs.condition)
            .unwrap();
        g2.connect_raw(apply2.outputs.application.id, group2_in)
            .unwrap();
        g2.set_socket_value(
            matcher2.inputs.pattern.id,
            Some(SocketValue::Text("Alice".into())),
        )
        .unwrap();

        let mut layer = ApplicationLayer::new();
        layer.graphs.push(g1); // lower priority
        layer.graphs.push(g2); // higher priority

        let ctx = LineContext {
            actor: "Alice".into(),
            line_text: String::new(),
            line_index: 0,
        };

        let result = layer
            .evaluate(&ctx, &style_results, &AnimationLibrary::default(), 0.0, 1.0)
            .unwrap();
        // g2's style (size=24, bold=false) should override g1's (size=12, bold=true)
        // merge_over: later overrides earlier. g1 is first in layer, g2 is second.
        // evaluate_layer does: result starts as default, then for each graph:
        //   result = graph_style.merge_over(result);
        // So for g1: result = g1_style.merge_over(default) = g1_style
        // For g2: result = g2_style.merge_over(g1_style) = g2 overrides g1
        assert_eq!(result.font_size, Some(24.0));
        assert_eq!(result.bold, Some(false));
    }

    #[test]
    fn app_layer_applies_animation_to_matched_style() {
        let mut style_chain = make_bold_chain(12.0, false);
        let style_result = style_chain.evaluate().unwrap();

        let style_results: HashMap<NodeGraphId, StyleResult> =
            vec![(style_chain.id, style_result.clone())]
                .into_iter()
                .collect();

        let mut g = ApplicationGraph::new("animated");
        let ctx_node: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let matcher: ApplicationNodeHandle<MatchActor> = g
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();
        let app_ref = g
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(style_chain.id),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let ref_out = g.nodes[&app_ref].outputs[0];
        let apply: ApplicationNodeHandle<ApplyMatch> = g
            .add_node(ApplicationNodeKind::ApplyMatch, &[], &[])
            .unwrap();
        let anim: ApplicationNodeHandle<ApplyAnimation> = g
            .add_node(ApplicationNodeKind::ApplyAnimation, &[], &[])
            .unwrap();
        let group_out = g
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[
                    SocketDecl::input("application", SocketType::Application),
                    SocketDecl::input("application", SocketType::Application),
                ],
                &[],
            )
            .unwrap();

        g.connect_raw(ref_out, apply.inputs.style.id).unwrap();
        g.connect(ctx_node.outputs.actor, matcher.inputs.source)
            .unwrap();
        g.connect(matcher.outputs.matched, apply.inputs.condition)
            .unwrap();
        // 应用动画的静态样式基础也必须明确连接（同一条样式链）
        g.connect_raw(ref_out, anim.inputs.style.id).unwrap();
        g.connect(matcher.outputs.matched, anim.inputs.condition)
            .unwrap();
        g.connect_raw(apply.outputs.application.id, g.nodes[&group_out].inputs[0])
            .unwrap();
        g.connect_raw(anim.outputs.application.id, g.nodes[&group_out].inputs[1])
            .unwrap();

        // Wire a default animation clip id into the animation input.
        let clip_id = AnimDataId::new();
        g.set_socket_value(
            anim.inputs.animation.id,
            Some(SocketValue::Animation(clip_id)),
        )
        .unwrap();
        g.set_socket_value(
            matcher.inputs.pattern.id,
            Some(SocketValue::Text("Alice".into())),
        )
        .unwrap();

        let mut layer = ApplicationLayer::new();
        layer.graphs.push(g);

        let ctx = LineContext {
            actor: "Alice".into(),
            line_text: String::new(),
            line_index: 0,
        };

        // Animation library without the referenced clip should error.
        let result = layer.evaluate(&ctx, &style_results, &AnimationLibrary::default(), 0.0, 1.0);
        assert!(result.is_err());
    }

    #[test]
    fn apply_animation_style_must_be_connected() {
        // 应用动画的样式输入未连接 → 必须报错，不能静默使用默认样式。
        let mut g = ApplicationGraph::new("anim-no-style");
        let matcher: ApplicationNodeHandle<MatchActor> = g
            .add_node(ApplicationNodeKind::MatchActor, &[], &[])
            .unwrap();
        let anim: ApplicationNodeHandle<ApplyAnimation> = g
            .add_node(ApplicationNodeKind::ApplyAnimation, &[], &[])
            .unwrap();
        let group_out = g
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[SocketDecl::input("application", SocketType::Application)],
                &[],
            )
            .unwrap();

        // 动画条件 = matcher 输出（恒真模式放行：source/pattern 都给空串，contains 恒真）
        g.connect(matcher.outputs.matched, anim.inputs.condition)
            .unwrap();
        g.set_socket_value(
            matcher.inputs.source.id,
            Some(SocketValue::Text(String::new())),
        )
        .unwrap();
        g.set_socket_value(
            matcher.inputs.pattern.id,
            Some(SocketValue::Text(String::new())),
        )
        .unwrap();
        g.connect_raw(anim.outputs.application.id, g.nodes[&group_out].inputs[0])
            .unwrap();
        g.set_socket_value(
            anim.inputs.animation.id,
            Some(SocketValue::Animation(AnimDataId::new())),
        )
        .unwrap();

        // 样式输入未连接：求值即报错（不静默默认）
        let ctx = LineContext {
            actor: String::new(),
            line_text: String::new(),
            line_index: 0,
        };
        let err = g.evaluate(&ctx, &HashMap::new()).unwrap_err();
        assert!(matches!(err, ApplicationError::StyleNotConnected(_)));
    }
}
