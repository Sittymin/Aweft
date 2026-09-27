//! 节点图操作 Action：样式图/应用图统一的图结构变更入口。
//!
//! 走 main 标准管线 `Action → dispatch → tag → 投影`（A2 决策落地）：
//!  - reducer（`AppStore::dispatch_node_graph`）修改 model 权威数据 + 打 `Tag::NodeGraph`
//!  - 投影（op_node_graph_*）按变更类型路由后精确定位更新视图模型
//!
//! 图类型用 model 的 `NodeGraphKind`（Blender `bNodeTree.type` 对应物），
//! 由 callbacks 层从 Slint `GraphType` 转换。

use aweft_model::{
    application_graph::ApplicationNodeKind,
    ids::AnimDataId,
    node_graph::{ColorRole, NodeGraphKind, NodeKind, StyleOpKind, VecOpKind},
};

/// 节点图结构/值变更操作。
#[derive(Debug, Clone)]
pub enum NodeGraphAction {
    /// 在右键位置添加节点，返回新节点在 nodes 模型中的下标（选中用）。
    AddNode {
        graph_type: NodeGraphKind,
        template_index: usize,
        x: f32,
        y: f32,
    },
    /// 删除节点（一并断开相关连线）。
    DeleteNode {
        graph_type: NodeGraphKind,
        node_index: usize,
    },
    /// 端口拖拽松开：按拖拽语义完成断开/重连。
    SocketDragEnded {
        graph_type: NodeGraphKind,
        node_index: usize,
        socket_index: usize,
        is_input: bool,
        x: f32,
        y: f32,
    },
    /// 引用节点的下拉选择（两种引用共用）：样式链引用选样式链，非线性动画引用选
    /// 动画片段。写入方式是原位改 kind 内嵌 id，不删节点、不动端口与连线。
    MenuSelected {
        graph_type: NodeGraphKind,
        node_index: usize,
        menu_index: usize,
    },
    /// 端口预设下拉选择（匹配角色 pattern 的默认值）。
    SocketOptionSelected {
        graph_type: NodeGraphKind,
        node_index: usize,
        socket_index: usize,
        option_index: usize,
    },
    /// 角色下拉"新建角色"：创建角色并设端口默认值。
    CreateActor {
        graph_type: NodeGraphKind,
        node_index: usize,
        socket_index: usize,
        name: String,
    },
    /// 内嵌文本输入框提交（字符串常量输出）：设端口默认值。
    SocketInputChanged {
        graph_type: NodeGraphKind,
        node_index: usize,
        socket_index: usize,
        text: String,
    },
    /// 端口行内嵌数字框提交（数值输入口）：把 `field` 分量写回端口默认值。
    /// 文本在 store 侧解析，解析不了的提交不动数据。
    SocketNumberChanged {
        graph_type: NodeGraphKind,
        node_index: usize,
        socket_index: usize,
        field: usize,
        text: String,
    },
    /// 端口行内嵌真/假切换（布尔常量输出）：翻转端口默认值。
    SocketBoolToggled {
        graph_type: NodeGraphKind,
        node_index: usize,
        socket_index: usize,
    },
    /// 新建一条默认链并切换到它。
    NewChain { graph_type: NodeGraphKind },
}

// ── 右键菜单模板 ──────────────────────────────────────────────────────────────

/// 样式图右键菜单模板（C5：补齐 node_kind_title 支持的全部样式节点）。
pub fn style_templates() -> Vec<NodeKind> {
    vec![
        NodeKind::StyleSource,
        NodeKind::StyleOp(StyleOpKind::SetFontName),
        NodeKind::StyleOp(StyleOpKind::SetFontSize),
        NodeKind::StyleOp(StyleOpKind::SetBold),
        NodeKind::StyleOp(StyleOpKind::SetItalic),
        NodeKind::StyleOp(StyleOpKind::SetUnderline),
        NodeKind::StyleOp(StyleOpKind::SetStrikeout),
        NodeKind::StyleOp(StyleOpKind::SetColor(ColorRole::Primary)),
        NodeKind::StyleOp(StyleOpKind::SetColor(ColorRole::Secondary)),
        NodeKind::StyleOp(StyleOpKind::SetColor(ColorRole::Outline)),
        NodeKind::StyleOp(StyleOpKind::SetColor(ColorRole::Back)),
        NodeKind::StyleOp(StyleOpKind::SetColor(ColorRole::All)),
        NodeKind::StyleOp(StyleOpKind::SetAlpha(ColorRole::Primary)),
        NodeKind::StyleOp(StyleOpKind::SetAlpha(ColorRole::Secondary)),
        NodeKind::StyleOp(StyleOpKind::SetAlpha(ColorRole::Outline)),
        NodeKind::StyleOp(StyleOpKind::SetAlpha(ColorRole::Back)),
        NodeKind::StyleOp(StyleOpKind::SetAlpha(ColorRole::All)),
        NodeKind::StyleOp(StyleOpKind::SetBorder),
        NodeKind::StyleOp(StyleOpKind::SetShadow),
        NodeKind::StyleOp(StyleOpKind::SetFontScale),
        NodeKind::StyleOp(StyleOpKind::SetFontRotation),
        NodeKind::StyleOp(StyleOpKind::SetFactor),
        NodeKind::StyleOp(StyleOpKind::SetBlur),
        NodeKind::StyleOp(StyleOpKind::SetBlurEdges),
        NodeKind::StyleOp(StyleOpKind::SetPosition),
        NodeKind::StyleOp(StyleOpKind::SetOrigin),
        NodeKind::StyleOp(StyleOpKind::SetClipRect),
        NodeKind::VecOp(VecOpKind::CombineVec2),
        NodeKind::VecOp(VecOpKind::SeparateVec2),
        NodeKind::VecOp(VecOpKind::CombineVec3),
        NodeKind::VecOp(VecOpKind::SeparateVec3),
        NodeKind::MergeStyle,
        NodeKind::GroupInput,
        NodeKind::GroupOutput,
        NodeKind::Viewer,
    ]
}

/// 应用图右键菜单模板。
pub fn app_templates() -> Vec<ApplicationNodeKind> {
    vec![
        ApplicationNodeKind::ContextInput,
        ApplicationNodeKind::MatchActor,
        ApplicationNodeKind::MatchText,
        ApplicationNodeKind::IntEq,
        ApplicationNodeKind::TextEq,
        ApplicationNodeKind::And,
        ApplicationNodeKind::Or,
        ApplicationNodeKind::Not,
        ApplicationNodeKind::TextConst,
        ApplicationNodeKind::BoolConst,
        // 占位 AnimDataId：`store::node_graph::resolve_app_payload` 在添加时换成
        // 库里按名排序的第一个动画片段（本函数拿不到库数据）。
        ApplicationNodeKind::AnimRef(AnimDataId::new()),
        ApplicationNodeKind::ApplyMatch,
        ApplicationNodeKind::ApplyAnimation,
        ApplicationNodeKind::GroupOutput,
    ]
}
