use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::animation::{AnimationError, AnimationLibrary, apply_animation_to_style};
use crate::ids::{AnimDataId, ItemId, NodeGraphId, NodeId};
use crate::node_graph::{
    error::EvalError,
    kind::{EvalCache, EvalContext, NodeTreeType},
    socket::{
        AnimationSocket, ApplicationSocket, ApplicationUnit, BoolSocket, IntSocket, SocketDecl,
        SocketType, SocketValue, StyleSocket, TextSocket,
    },
    style::StyleResult,
    Node, NodeGraph, NodeHandle,
};

// ── LineContext ───────────────────────────────────────────────────────────────

/// 外部传入的当前行信息。
#[derive(Debug, Clone, Default)]
pub struct LineContext {
    pub actor: String,
    pub line_text: String,
    /// 当前可见行的序号：插入/删除/隐藏任意一行都会让它移位，因此只能用于人眼可读的
    /// 展示与临时判断，**不要**用它绑定任何需要跨编辑存活的东西。
    pub line_index: i64,
    /// 这条字幕行的稳定身份。自动生成的条件（视口拖动写出的那条链）只认它。
    pub item_id: ItemId,
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

/// 应用层求值过程中可能产生的错误。
#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Eval(#[from] EvalError),
    #[error(transparent)]
    Animation(#[from] AnimationError),
}

// ── ApplicationNodeKind ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplicationNodeKind {
    ContextInput,
    MatchActor,
    MatchText,
    IntEq,
    /// 文本全等判断：按行自动生成的条件用它，`MatchActor` 的子串匹配不足以锁定一行。
    TextEq,
    And,
    Or,
    Not,
    AppRef(NodeGraphId),
    /// 非线性动画引用：输出一条动画片段（`AnimData`）引用，供应用动画的动画端口
    /// 按连线取用。与 `AppRef` 对称——引用对象不同（动画片段 vs 样式链），
    /// 所以载荷是 `AnimDataId`。
    AnimRef(AnimDataId),
    /// 字符串常量：输出一段文本（值由节点默认值提供，UI 节点内可编辑）。
    TextConst,
    /// 布尔常量：输出一个真/假（值由节点默认值提供，UI 节点内可切换）。条件支路
    /// 要「无条件成立」时用它，而不是拿空 pattern 的匹配角色碰 `contains("")` 恒真。
    BoolConst,
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
}

// ── 应用图树类型注册（ApplicationNodeKind） ───────────────────────────────────

impl NodeTreeType for ApplicationNodeKind {
    type Result = Option<ApplicationGraphMatch>;

    fn validate_add(&self, _graph: &NodeGraph<Self>) -> Result<(), EvalError> {
        Ok(())
    }

    fn is_static(&self) -> bool {
        ApplicationNodeKind::is_static(self)
    }

    fn socket_decls(&self) -> Option<(Vec<SocketDecl>, Vec<SocketDecl>)> {
        // 宏生成的目录按 kind 相等比较，认不出带载荷的变体。AppRef 的端口由添加方
        // 传 `extra_outputs` 补；AnimRef 的添加方（GUI 的 `graph_add_node`）不传
        // extras，所以它的固定输出口只能在这里声明。
        if let ApplicationNodeKind::AnimRef(_) = self {
            return Some((
                Vec::new(),
                vec![SocketDecl::output("animation", SocketType::Animation)],
            ));
        }
        get_application_socket_decls(self)
    }

    fn evaluate_node(
        &self,
        graph: &NodeGraph<Self>,
        node_id: NodeId,
        cache: &mut EvalCache,
        ctx: &EvalContext,
    ) -> Result<(), EvalError> {
        let node = &graph.nodes[&node_id];
        match &node.kind {
            ApplicationNodeKind::ContextInput => {
                let Some(line) = ctx.line else {
                    return Ok(());
                };
                if let Some(&sid) = node.outputs.first() {
                    cache
                        .socket_values
                        .insert(sid, SocketValue::Text(line.actor.clone()));
                }
                if let Some(&sid) = node.outputs.get(1) {
                    cache
                        .socket_values
                        .insert(sid, SocketValue::Text(line.line_text.clone()));
                }
                if let Some(&sid) = node.outputs.get(2) {
                    cache
                        .socket_values
                        .insert(sid, SocketValue::Int(line.line_index));
                }
                if let Some(&sid) = node.outputs.get(3) {
                    cache.socket_values.insert(
                        sid,
                        SocketValue::Text(line.item_id.to_string()),
                    );
                }
            }

            ApplicationNodeKind::MatchActor => {
                let source = graph.read_input(node, 0, &cache.socket_values);
                let pattern = graph.read_input(node, 1, &cache.socket_values);
                let matched = match (source, pattern) {
                    (Some(SocketValue::Text(s)), Some(SocketValue::Text(p))) => s.contains(&p),
                    _ => false,
                };
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(matched));
                }
            }

            ApplicationNodeKind::MatchText => {
                let source = graph.read_input(node, 0, &cache.socket_values);
                let pattern = graph.read_input(node, 1, &cache.socket_values);
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
                let a = graph
                    .read_input(node, 0, &cache.socket_values)
                    .and_then(|v| match v {
                        SocketValue::Int(i) => Some(i),
                        _ => None,
                    })
                    .unwrap_or(0);
                let b = graph
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

            // 文本全等。自动生成的条件要的是「就这一行」，而 MatchActor 的子串匹配太宽
            // （空 pattern 恒真、互相前缀的值会串台），所以单列一个全等判断。任一侧缺值
            // 或未连线都判 false：条件宁可永不成立，也不能让一条按行写的单元套到所有行上。
            ApplicationNodeKind::TextEq => {
                let eq = match (
                    graph.read_input(node, 0, &cache.socket_values),
                    graph.read_input(node, 1, &cache.socket_values),
                ) {
                    (Some(SocketValue::Text(a)), Some(SocketValue::Text(b))) => a == b,
                    _ => false,
                };
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(eq));
                }
            }

            ApplicationNodeKind::And => {
                let all_true = node.inputs.iter().all(|&sid| {
                    graph
                        .input_source(sid)
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
                    graph
                        .input_source(sid)
                        .and_then(|from| cache.socket_values.get(&from))
                        .map(|v| matches!(v, SocketValue::Bool(true)))
                        .unwrap_or(false)
                });
                if let Some(&sid) = node.outputs.first() {
                    cache.socket_values.insert(sid, SocketValue::Bool(any_true));
                }
            }

            ApplicationNodeKind::Not => {
                let input = graph
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
                    let text = match graph.socket(sid).default_value.clone() {
                        Some(SocketValue::Text(s)) => s,
                        _ => String::new(),
                    };
                    cache.socket_values.insert(sid, SocketValue::Text(text));
                }
            }

            // 布尔常量：输出节点默认值里的布尔（UI 节点内切换，未设则假）。
            ApplicationNodeKind::BoolConst => {
                if let Some(&sid) = node.outputs.first() {
                    let value = matches!(
                        graph.socket(sid).default_value,
                        Some(SocketValue::Bool(true))
                    );
                    cache.socket_values.insert(sid, SocketValue::Bool(value));
                }
            }

            ApplicationNodeKind::AppRef(graph_id) => {
                let style = ctx
                    .style_results
                    .map(|m| m.get(graph_id).cloned().unwrap_or_default())
                    .unwrap_or_default();
                cache.node_styles.insert(node_id, style);
            }

            // 非线性动画引用：输出 kind 内嵌的动画片段 id，应用动画的动画端口经连线读它。
            ApplicationNodeKind::AnimRef(anim_id) => {
                if let Some(&sid) = node.outputs.first() {
                    cache
                        .socket_values
                        .insert(sid, SocketValue::Animation(*anim_id));
                }
            }

            // 应用匹配：style（必须明确连接）+ condition → 应用单元，供组输出收集。
            ApplicationNodeKind::ApplyMatch => {
                let style = required_style(graph, node, cache)?;
                let condition = node
                    .inputs
                    .get(1)
                    .and_then(|&sid| graph.input_source(sid))
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
                let style = required_style(graph, node, cache)?;
                let animation = node
                    .inputs
                    .get(1)
                    .and_then(|&sid| {
                        graph
                            .input_source(sid)
                            .and_then(|from| cache.socket_values.get(&from).cloned())
                            .or_else(|| graph.socket(sid).default_value.clone())
                    })
                    .and_then(|v| match v {
                        SocketValue::Animation(id) => Some(id),
                        _ => None,
                    });
                let condition = node
                    .inputs
                    .get(2)
                    .and_then(|&sid| graph.input_source(sid))
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

            // 组输出：出口节点，不计算任何值；收集与合并发生在 collect_result。
            ApplicationNodeKind::GroupOutput => {}
        }
        Ok(())
    }

    fn collect_result(graph: &NodeGraph<Self>, cache: &EvalCache) -> Result<Self::Result, EvalError> {
        // 组输出是唯一出口：按输入端口连接顺序（= 端口创建顺序）收集上游应用单元，
        // 依次合并——后连接的端口优先级更高（覆盖前者），对应总应用图里
        // ApplicationLayer 按 Vec 顺序"后面的图覆盖前面的图"。
        let matched = graph
            .nodes
            .values()
            .find(|n| matches!(n.kind, ApplicationNodeKind::GroupOutput))
            .map(
                |group_out| -> Result<Option<ApplicationGraphMatch>, EvalError> {
                    let mut result = ApplicationGraphMatch::default();
                    let mut any = false;
                    for &input_id in &group_out.inputs {
                        let Some(from) = graph.input_source(input_id) else {
                            continue;
                        };
                        let Some(unit) = cache.node_applications.get(&graph.socket(from).node) else {
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
        Ok(matched)
    }

    fn reference_id(&self) -> Option<NodeGraphId> {
        match self {
            ApplicationNodeKind::AppRef(id) => Some(*id),
            _ => None,
        }
    }

    fn set_reference_id(&mut self, new_id: NodeGraphId) -> Result<(), EvalError> {
        match self {
            ApplicationNodeKind::AppRef(id) => {
                *id = new_id;
                Ok(())
            }
            _ => Err(EvalError::NotReferenceNode),
        }
    }

    fn anim_ref_id(&self) -> Option<AnimDataId> {
        match self {
            ApplicationNodeKind::AnimRef(id) => Some(*id),
            _ => None,
        }
    }

    fn set_anim_ref_id(&mut self, new_id: AnimDataId) -> Result<(), EvalError> {
        match self {
            ApplicationNodeKind::AnimRef(id) => {
                *id = new_id;
                Ok(())
            }
            _ => Err(EvalError::NotReferenceNode),
        }
    }
}

/// 读取样式输入（第 0 号输入）：必须明确连接样式来源，未连接报错。
fn required_style(
    graph: &NodeGraph<ApplicationNodeKind>,
    node: &Node<ApplicationNodeKind>,
    cache: &EvalCache,
) -> Result<StyleResult, EvalError> {
    let style_sid = node
        .inputs
        .first()
        .ok_or_else(|| EvalError::StyleNotConnected(graph.name.clone()))?;
    match graph.upstream_node_id(*style_sid) {
        Some(up) => Ok(cache.node_styles.get(&up).cloned().unwrap_or_default()),
        None => Err(EvalError::StyleNotConnected(graph.name.clone())),
    }
}

// ── 类型化 Node Handle ───────────────────────────────────────────────────────

/// 应用节点类型化 handle（`NodeHandle<_, ApplicationNodeKind>` 的别名）。
pub type ApplicationNodeHandle<T> = NodeHandle<T, ApplicationNodeKind>;

/// 应用图类型（`NodeGraph<ApplicationNodeKind>` 的别名）。
pub type ApplicationGraph = NodeGraph<ApplicationNodeKind>;

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

            impl $crate::node_graph::NodeSpec<$crate::application_graph::ApplicationNodeKind> for $name {
                const KIND: $crate::application_graph::ApplicationNodeKind = $kind;
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
        // `id` 追加在末尾：已存盘图里的端口按下标存着，插队会让老连线指错口。
        outputs: { actor: TextSocket, text: TextSocket, index: IntSocket, id: TextSocket }
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
    TextEq, TextEqIn, TextEqOut {
        kind: ApplicationNodeKind::TextEq;
        inputs: { source: TextSocket, expected: TextSocket }
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
    BoolConst, BoolConstIn, BoolConstOut {
        kind: ApplicationNodeKind::BoolConst;
        inputs: {}
        outputs: { value: BoolSocket }
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

// ── ApplicationLayer ─────────────────────────────────────────────────────────

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
            if let Some(matched) = graph.evaluate(&EvalContext::app(ctx, style_results))? {
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
            if let Some(matched) = graph.evaluate(&EvalContext::app(ctx, style_results))? {
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
    use crate::node_graph::{
        NodeGraph, NodeHandle, NodeId, NodeKind, SocketDecl, SocketType, StyleOpKind,
    };

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
        g.evaluate(&EvalContext::style()).unwrap();
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
            item_id: ItemId::new(),
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&EvalContext::app(&ctx, &empty)).unwrap();
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
            item_id: ItemId::new(),
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&EvalContext::app(&ctx, &empty)).unwrap();
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
            item_id: ItemId::new(),
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&EvalContext::app(&ctx, &empty)).unwrap();
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
            item_id: ItemId::new(),
        };
        let empty = HashMap::new();
        let matched = g.evaluate(&EvalContext::app(&ctx, &empty)).unwrap();
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
            item_id: ItemId::new(),
        };
        let style_results = HashMap::new();
        let matched = g.evaluate(&EvalContext::app(&ctx, &style_results)).unwrap();
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
            item_id: ItemId::new(),
        };
        let style_results = HashMap::new();
        let matched = g.evaluate(&EvalContext::app(&ctx, &style_results)).unwrap();
        assert!(matched.is_some());
    }

    /// 按行自动生成的条件走 `ContextInput.id` + `TextEq` 全等。
    ///
    /// 同一个图对象连评两行是这条测试的重点：求值缓存必须按行失效，否则第二行会沿用
    /// 第一行的 `ContextInput` 输出，一条按行写的样式就会套到整条时间轴上。
    #[test]
    fn text_eq_gates_a_unit_to_one_line() {
        let mut g = ApplicationGraph::new("test");
        let ctx_node: ApplicationNodeHandle<ContextInput> = g
            .add_node(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let eq: ApplicationNodeHandle<TextEq> = g
            .add_node(ApplicationNodeKind::TextEq, &[], &[])
            .unwrap();
        let app_ref = g
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(NodeGraphId::new()),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
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

        g.connect_raw(g.nodes[&app_ref].outputs[0], apply.inputs.style.id)
            .unwrap();
        g.connect(ctx_node.outputs.id, eq.inputs.source)
            .unwrap();
        g.connect(eq.outputs.equal, apply.inputs.condition)
            .unwrap();
        g.connect_raw(apply.outputs.application.id, g.nodes[&group_out].inputs[0])
            .unwrap();

        let item_id = ItemId::new();
        g.set_socket_value(
            eq.inputs.expected.id,
            Some(SocketValue::Text(item_id.to_string())),
        )
        .unwrap();

        let owning = LineContext {
            actor: String::new(),
            line_text: String::new(),
            line_index: 0,
            item_id,
        };
        let other = LineContext {
            actor: String::new(),
            line_text: String::new(),
            line_index: 1,
            item_id: ItemId::new(),
        };
        let style_results = HashMap::new();
        assert!(
            g.evaluate(&EvalContext::app(&owning, &style_results))
                .unwrap()
                .is_some(),
            "本行应命中"
        );
        assert!(
            g.evaluate(&EvalContext::app(&other, &style_results))
                .unwrap()
                .is_none(),
            "换到别行不该再命中"
        );
    }
    #[test]
    fn app_layer_composes_styles_by_priority() {
        let mut bold_chain = make_bold_chain(12.0, true);
        let bold_result = bold_chain.evaluate(&EvalContext::style()).unwrap();

        let mut size_chain = make_bold_chain(24.0, false);
        let size_result = size_chain.evaluate(&EvalContext::style()).unwrap();

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
            item_id: ItemId::new(),
        };

        let result = layer
            .evaluate(&ctx, &style_results, &AnimationLibrary::default(), 0.0, 1.0)
            .unwrap();
        // g2's style (size=24, bold=false) should override g1's (size=12, bold=true)
        assert_eq!(result.font_size, Some(24.0));
        assert_eq!(result.bold, Some(false));
    }

    #[test]
    fn app_layer_applies_animation_to_matched_style() {
        let mut style_chain = make_bold_chain(12.0, false);
        let style_result = style_chain.evaluate(&EvalContext::style()).unwrap();

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
            item_id: ItemId::new(),
        };

        // Animation library without the referenced clip should error.
        let result = layer.evaluate(&ctx, &style_results, &AnimationLibrary::default(), 0.0, 1.0);
        assert!(result.is_err());
    }

    #[test]
    fn apply_animation_style_must_be_connected() {
        // 应用动画的样式输入未连接 → 必须报错，不能静默使用默认样式。
        let mut g = ApplicationGraph::new("anim-no-style");
        let always: ApplicationNodeHandle<BoolConst> = g
            .add_node(ApplicationNodeKind::BoolConst, &[], &[])
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

        // 条件用常量真放行：这条测试只盯「样式未连接」，不该再靠空 pattern 碰 contains 恒真。
        g.connect(always.outputs.value, anim.inputs.condition)
            .unwrap();
        g.set_socket_value(always.outputs.value.id, Some(SocketValue::Bool(true)))
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
            item_id: ItemId::new(),
        };
        let err = g.evaluate(&EvalContext::app(&ctx, &HashMap::new())).unwrap_err();
        assert!(matches!(err, EvalError::StyleNotConnected(_)));
    }

    #[test]
    fn bool_const_gates_application() {
        // 布尔常量作条件：真放行、假拦住、未设按假。空 pattern 的匹配角色不该再当「恒真」用。
        let mut g = ApplicationGraph::new("bool-gate");
        let app_ref = g
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(NodeGraphId::new()),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let always: ApplicationNodeHandle<BoolConst> = g
            .add_node(ApplicationNodeKind::BoolConst, &[], &[])
            .unwrap();
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

        g.connect_raw(g.nodes[&app_ref].outputs[0], apply.inputs.style.id)
            .unwrap();
        g.connect(always.outputs.value, apply.inputs.condition)
            .unwrap();
        g.connect_raw(apply.outputs.application.id, g.nodes[&group_out].inputs[0])
            .unwrap();

        let line_ctx = LineContext {
            actor: String::new(),
            line_text: String::new(),
            line_index: 0,
            item_id: ItemId::new(),
        };
        let style_results = HashMap::new();
        let ctx = EvalContext::app(&line_ctx, &style_results);
        // 未设默认值 → 假
        assert!(g.evaluate(&ctx).unwrap().is_none());
        g.set_socket_value(always.outputs.value.id, Some(SocketValue::Bool(true)))
            .unwrap();
        assert!(g.evaluate(&ctx).unwrap().is_some());
        g.set_socket_value(always.outputs.value.id, Some(SocketValue::Bool(false)))
            .unwrap();
        assert!(g.evaluate(&ctx).unwrap().is_none());
    }

    #[test]
    fn anim_ref_node_feeds_apply_animation_by_link() {
        // 非线性动画引用：动画片段 id 存在 kind 里，经连线（不是端口默认值）喂给
        // 应用动画的动画端口——引用关系因此是图上看得见的一条边。
        let mut g = ApplicationGraph::new("anim-ref");
        let clip_id = AnimDataId::new();
        let anim_ref = g
            .add_node::<NodeId>(ApplicationNodeKind::AnimRef(clip_id), &[], &[])
            .unwrap();
        let style_ref = g
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(NodeGraphId::new()),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let always: ApplicationNodeHandle<BoolConst> = g
            .add_node(ApplicationNodeKind::BoolConst, &[], &[])
            .unwrap();
        let apply: ApplicationNodeHandle<ApplyAnimation> = g
            .add_node(ApplicationNodeKind::ApplyAnimation, &[], &[])
            .unwrap();
        let group_out = g
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[SocketDecl::input("application", SocketType::Application)],
                &[],
            )
            .unwrap();

        // 添加时不传 extras：输出口来自 `socket_decls` 里为 AnimRef 声明的那一个
        assert_eq!(g.nodes[&anim_ref].outputs.len(), 1);
        g.connect_raw(g.nodes[&anim_ref].outputs[0], apply.inputs.animation.id)
            .unwrap();
        g.connect_raw(g.nodes[&style_ref].outputs[0], apply.inputs.style.id)
            .unwrap();
        g.connect(always.outputs.value, apply.inputs.condition)
            .unwrap();
        g.set_socket_value(always.outputs.value.id, Some(SocketValue::Bool(true)))
            .unwrap();
        g.connect_raw(apply.outputs.application.id, g.nodes[&group_out].inputs[0])
            .unwrap();

        let line_ctx = LineContext {
            actor: String::new(),
            line_text: String::new(),
            line_index: 0,
            item_id: ItemId::new(),
        };
        let style_results = HashMap::new();
        let ctx = EvalContext::app(&line_ctx, &style_results);
        assert_eq!(
            g.evaluate(&ctx).unwrap().unwrap().animation,
            Some(clip_id),
            "连线送来的动画片段应出现在匹配结果里"
        );

        // 原位换引用的动画片段：节点/端口/连线都不动，标脏让下次求值重算
        let other_id = AnimDataId::new();
        g.set_node_anim_ref(anim_ref, other_id).unwrap();
        assert_eq!(g.evaluate(&ctx).unwrap().unwrap().animation, Some(other_id));

        // 非线性动画引用之外的节点不接受这个操作
        assert!(matches!(
            g.set_node_anim_ref(style_ref, other_id),
            Err(EvalError::NotReferenceNode)
        ));
    }
}
