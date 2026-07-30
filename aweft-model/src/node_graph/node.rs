use serde::{Deserialize, Serialize};

use crate::ids::{NodeGraphId, NodeId, SocketId};

use super::socket::{
    BoolSocket, ClipRectSocket, FloatSocket, Socket, SocketDecl, StyleSocket, Vec2Socket,
    Vec3Socket,
};

// ── ColorRole ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorRole {
    Primary,
    Secondary,
    Outline,
    Back,
    All,
}

// ── 节点类型 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VecOpKind {
    CombineVec2,
    SeparateVec2,
    CombineVec3,
    SeparateVec3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StyleOpKind {
    SetFontName,
    SetFontSize,
    SetBold,
    SetItalic,
    SetUnderline,
    SetStrikeout,
    SetColor(ColorRole),
    SetAlpha(ColorRole),
    SetBorder,
    SetShadow,
    SetFontScale,
    SetFontRotation,
    SetFactor,
    SetBlur,
    SetBlurEdges,
    SetPosition,
    SetOrigin,
    SetClipRect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    StyleSource,
    VecOp(VecOpKind),
    StyleOp(StyleOpKind),
    MergeStyle,
    GroupInput,
    GroupOutput,
    Viewer,
    Reference(NodeGraphId),
}

// ── Socket 声明查询 ────────────────────────────────────────────────────────────

impl NodeKind {
    /// 返回节点的基础 socket 接口声明。
    ///
    /// 对于完全静态的节点（StyleSource / VecOp / StyleOp / Viewer），该声明即最终接口；
    /// 对于半动态节点（MergeStyle / GroupInput / GroupOutput），声明提供至少一个输入或输出，
    /// 调用方可在 `add_node` 中通过 extra_inputs / extra_outputs 追加更多端口。
    /// Reference 返回 None，其接口完全由被引用图决定。
    /// 节点是否为完全静态节点（不允许通过 extra 追加端口）。
    pub fn is_static(&self) -> bool {
        matches!(
            self,
            NodeKind::StyleSource | NodeKind::VecOp(_) | NodeKind::StyleOp(_) | NodeKind::Viewer
        )
    }
}

// ── 节点实例 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub inputs: Vec<SocketId>,
    pub outputs: Vec<SocketId>,
}

impl GraphNode {
    pub fn input<'a>(
        &self,
        idx: usize,
        sockets: &'a std::collections::HashMap<SocketId, Socket>,
    ) -> Option<&'a Socket> {
        self.inputs.get(idx).and_then(|id| sockets.get(id))
    }

    pub fn output<'a>(
        &self,
        idx: usize,
        sockets: &'a std::collections::HashMap<SocketId, Socket>,
    ) -> Option<&'a Socket> {
        self.outputs.get(idx).and_then(|id| sockets.get(id))
    }
}

// ── 类型化 Node Handle ────────────────────────────────────────────────────────

/// 节点的类型级规范。每种节点对应一个实现者，宏据此批量生成。
pub trait NodeSpec {
    const KIND: NodeKind;
    type Inputs;
    type Outputs;
    fn build_inputs(node_id: NodeId, sockets: &[SocketId]) -> Self::Inputs;
    fn build_outputs(node_id: NodeId, sockets: &[SocketId]) -> Self::Outputs;
    fn input_decls() -> Vec<SocketDecl>;
    fn output_decls() -> Vec<SocketDecl>;
}

pub struct NodeHandle<T: NodeSpec> {
    pub id: NodeId,
    pub inputs: T::Inputs,
    pub outputs: T::Outputs,
}

impl<T: NodeSpec> NodeHandle<T> {
    pub fn to_id(self) -> NodeId {
        self.id
    }
}

/// 决定 `add_node` 返回 `NodeId` 还是 `NodeHandle<T>`，
/// 同时提供 socket 声明解析路径。
pub trait FromNode {
    fn from_node(node_id: NodeId, graph: &super::graph::NodeGraph) -> Self;
    fn resolve_decls(
        kind: &NodeKind,
        extra_inputs: &[SocketDecl],
        extra_outputs: &[SocketDecl],
    ) -> (Vec<SocketDecl>, Vec<SocketDecl>);
}

impl FromNode for NodeId {
    fn from_node(node_id: NodeId, _graph: &super::graph::NodeGraph) -> Self {
        node_id
    }
    fn resolve_decls(
        kind: &NodeKind,
        extra_inputs: &[SocketDecl],
        extra_outputs: &[SocketDecl],
    ) -> (Vec<SocketDecl>, Vec<SocketDecl>) {
        let (base_i, base_o) = get_socket_decls(kind).unwrap_or_default();
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

impl<T: NodeSpec> FromNode for NodeHandle<T> {
    fn from_node(node_id: NodeId, graph: &super::graph::NodeGraph) -> Self {
        let node = &graph.nodes[&node_id];
        NodeHandle {
            id: node_id,
            inputs: T::build_inputs(node_id, &node.inputs),
            outputs: T::build_outputs(node_id, &node.outputs),
        }
    }
    fn resolve_decls(
        _kind: &NodeKind,
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

// ── 节点定义宏 ────────────────────────────────────────────────────────────────

/// 顶层宏：一次性声明所有节点，统一生成类型级规范和运行时 socket 声明。
#[macro_export]
macro_rules! define_nodes {
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

            impl $crate::node_graph::NodeSpec for $name {
                const KIND: $crate::node_graph::NodeKind = $kind;
                type Inputs = $inputs;
                type Outputs = $outputs;

                #[allow(unused_variables, unused_mut, unused_assignments)]
                fn build_inputs(
                    _node_id: $crate::node_graph::NodeId,
                    sockets: &[$crate::node_graph::SocketId],
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
                    _node_id: $crate::node_graph::NodeId,
                    sockets: &[$crate::node_graph::SocketId],
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

        /// 运行时根据 `NodeKind` 查找 socket 声明（供 untyped `add_node::<NodeId>` 路径使用）。
        pub fn get_socket_decls(
            kind: &$crate::node_graph::NodeKind,
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

// ── 通过宏定义所有具体节点 ────────────────────────────────────────────────────

define_nodes! {
    StyleSource, StyleSourceInputs, StyleSourceOutputs {
        kind: NodeKind::StyleSource;
        inputs: {}
        outputs: { style: StyleSocket }
    }
    SetFontSize, SetFontSizeInputs, SetFontSizeOutputs {
        kind: NodeKind::StyleOp(StyleOpKind::SetFontSize);
        inputs: { style: StyleSocket, size: FloatSocket }
        outputs: { style: StyleSocket }
    }
    SetBold, SetBoldInputs, SetBoldOutputs {
        kind: NodeKind::StyleOp(StyleOpKind::SetBold);
        inputs: { style: StyleSocket, bold: BoolSocket }
        outputs: { style: StyleSocket }
    }
    SetPosition, SetPositionInputs, SetPositionOutputs {
        kind: NodeKind::StyleOp(StyleOpKind::SetPosition);
        inputs: { style: StyleSocket, position: Vec2Socket }
        outputs: { style: StyleSocket }
    }
    SetClipRect, SetClipRectInputs, SetClipRectOutputs {
        kind: NodeKind::StyleOp(StyleOpKind::SetClipRect);
        inputs: { style: StyleSocket, clip_rect: ClipRectSocket }
        outputs: { style: StyleSocket }
    }
    SetFontRotation, SetFontRotationInputs, SetFontRotationOutputs {
        kind: NodeKind::StyleOp(StyleOpKind::SetFontRotation);
        inputs: { style: StyleSocket, rotation: Vec3Socket }
        outputs: { style: StyleSocket }
    }
    CombineVec2, CombineVec2Inputs, CombineVec2Outputs {
        kind: NodeKind::VecOp(VecOpKind::CombineVec2);
        inputs: { x: FloatSocket, y: FloatSocket }
        outputs: { vector: Vec2Socket }
    }
    SeparateVec2, SeparateVec2Inputs, SeparateVec2Outputs {
        kind: NodeKind::VecOp(VecOpKind::SeparateVec2);
        inputs: { vector: Vec2Socket }
        outputs: { x: FloatSocket, y: FloatSocket }
    }
    CombineVec3, CombineVec3Inputs, CombineVec3Outputs {
        kind: NodeKind::VecOp(VecOpKind::CombineVec3);
        inputs: { x: FloatSocket, y: FloatSocket, z: FloatSocket }
        outputs: { vector: Vec3Socket }
    }
    SeparateVec3, SeparateVec3Inputs, SeparateVec3Outputs {
        kind: NodeKind::VecOp(VecOpKind::SeparateVec3);
        inputs: { vector: Vec3Socket }
        outputs: { x: FloatSocket, y: FloatSocket, z: FloatSocket }
    }
    Viewer, ViewerInputs, ViewerOutputs {
        kind: NodeKind::Viewer;
        inputs: { style: StyleSocket }
        outputs: {}
    }
    GroupOutput, GroupOutputInputs, GroupOutputOutputs {
        kind: NodeKind::GroupOutput;
        inputs: { style: StyleSocket }
        outputs: {}
    }
    GroupInput, GroupInputInputs, GroupInputOutputs {
        kind: NodeKind::GroupInput;
        inputs: {}
        outputs: { style: StyleSocket }
    }
    MergeStyle, MergeStyleInputs, MergeStyleOutputs {
        kind: NodeKind::MergeStyle;
        inputs: { style0: StyleSocket }
        outputs: { style: StyleSocket }
    }
}
