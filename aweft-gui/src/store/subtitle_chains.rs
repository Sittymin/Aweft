//! 视口里的可视化操作落在链上的那一份数据。
//!
//! 字幕位置的唯一真相是两条链：一条**视口样式链**（`样式源 → 设位置 → 组输出`，写的是
//! 位置基线，单位排版像素，位置动画在它之上相加）供出这份样式，一条**视口应用链**
//! （`上下文(id) → 文本全等(本行) → 应用 → 组输出`）把它只贴到那一行上。视口拖动因此不往
//! 行上写任何旁路字段，节点图编辑器里看到的就是拖动本身。
//!
//! 生成物**不靠名字也不靠额外标记找回，靠结构找回**：应用链里 `expected == 行身份` 的那个
//! `文本全等`，它的输出门控着一个 `应用` 节点的条件口，那个 `应用` 的样式口来自一个
//! `样式链引用`，引用的样式链里第一个 `设位置` 就是要改的端口。用户给这两条链改名、往里面
//! 再加节点都不影响找回。

use anyhow::Result;
use aweft_model::{
    application_graph::{
        ApplicationGraph, ApplicationNodeHandle, ApplicationNodeKind, ApplyMatch, ContextInput,
        TextEq,
    },
    ids::{ItemId, NodeGraphId, NodeId, SocketId},
    node_graph::{
        Node, NodeGraph, NodeHandle, NodeKind, SetPosition, SocketDecl, SocketType, SocketValue,
        StyleOpKind,
    },
    project::Project,
    sequence::ItemContent,
};

/// 视口链的名字（样式链与应用链同名，分别出现在两个菜单里）。
const VIEWPORT_CHAIN: &str = "视口位置";

/// 链名里带的行文本长度上限：菜单宽度有限，整段字幕塞进名字会挤爆。
const NAME_TEXT_CHARS: usize = 12;

/// 一行的位置在模型里的落点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinePosition {
    /// 承载位置的样式链
    pub(crate) style_chain: NodeGraphId,
    /// 要写的 `设位置` 节点（打精确 tag 时定位到这个节点的一行）
    pub(crate) position_node: NodeId,
    /// `设位置` 的 position 输入口（位置基线，排版像素）
    pub(crate) position_socket: SocketId,
}

/// 找回某行已有的视口位置；这行没在视口里拖过就 `None`。
pub(crate) fn find_line_position(project: &Project, item_id: ItemId) -> Option<LinePosition> {
    let key = item_id.to_string();
    for app in project.style_layer.graphs.iter() {
        for eq in app.nodes.values() {
            if !matches!(eq.kind, ApplicationNodeKind::TextEq) {
                continue;
            }
            // `文本全等` 的第二个输入是 expected：视口链在这里写着这一行的身份
            let Some(&expected) = eq.inputs.get(1) else {
                continue;
            };
            let Some(socket) = app.sockets.get(&expected) else {
                continue;
            };
            if !matches!(socket.default_value.as_ref(), Some(SocketValue::Text(t)) if *t == key) {
                continue;
            }
            let Some(&out) = eq.outputs.first() else {
                continue;
            };
            let Some(apply) = gated_apply(app, out) else {
                continue;
            };
            let Some(style_chain) = referenced_style_chain(app, apply) else {
                continue;
            };
            let Some((position_node, position_socket)) = position_socket_in(project, style_chain)
            else {
                continue;
            };
            return Some(LinePosition {
                style_chain,
                position_node,
                position_socket,
            });
        }
    }
    None
}

/// 把一行的位置**基线**平移 `dx`/`dy`（排版像素）。需要时先生成这一行的视口链对。
///
/// 写增量而不写绝对锚点：这份基线是位置动画相加的起点，平移它整条 `\move` 轨迹跟着走；
/// 送绝对坐标会把按下那一瞬动画恰好走到的偏移烤进基线。端口还没有值就从 0 起算。
///
/// 第二个返回值是 `true` 表示新建了链——调用方据此决定要不要重派生链菜单。
pub(crate) fn shift_line_position(
    project: &mut Project,
    item_id: ItemId,
    dx: f64,
    dy: f64,
) -> Result<(LinePosition, bool)> {
    let (placement, created) = match find_line_position(project, item_id) {
        Some(found) => (found, false),
        None => (create_line_position(project, item_id)?, true),
    };
    let chain = project
        .library
        .node_graphs
        .get_mut(&placement.style_chain)
        .ok_or_else(|| anyhow::anyhow!("视口样式链 {0:?} 不见了", placement.style_chain))?;
    let (base_x, base_y) = match chain
        .sockets
        .get(&placement.position_socket)
        .and_then(|socket| socket.default_value.as_ref())
    {
        Some(SocketValue::Vec2 { x, y }) => (*x, *y),
        _ => (0.0, 0.0),
    };
    chain.set_socket_value(
        placement.position_socket,
        Some(SocketValue::Vec2 {
            x: base_x + dx,
            y: base_y + dy,
        }),
    )?;
    Ok((placement, created))
}

/// `文本全等` 的输出所门控的那个 `应用` 节点（必须是它的条件输入口，即第 2 口）。
fn gated_apply(app: &ApplicationGraph, out: SocketId) -> Option<&Node<ApplicationNodeKind>> {
    app.links
        .iter()
        .filter(|link| link.from == out)
        .find_map(|link| condition_input(app, link.to))
}

/// `socket` 是某个 `应用` 节点的条件输入口时，返回那个节点。
fn condition_input(app: &ApplicationGraph, socket: SocketId) -> Option<&Node<ApplicationNodeKind>> {
    let node = app.nodes.get(&app.sockets.get(&socket)?.node)?;
    let gates =
        matches!(node.kind, ApplicationNodeKind::ApplyMatch) && node.inputs.get(1) == Some(&socket);
    gates.then_some(node)
}

/// `应用` 节点的样式输入口（第 1 口）来自哪条样式链的引用。
fn referenced_style_chain(
    app: &ApplicationGraph,
    apply: &Node<ApplicationNodeKind>,
) -> Option<NodeGraphId> {
    let style_socket = *apply.inputs.first()?;
    let from = app.links.iter().find(|link| link.to == style_socket)?.from;
    let node = app.nodes.get(&app.sockets.get(&from)?.node)?;
    match node.kind {
        ApplicationNodeKind::AppRef(id) => Some(id),
        _ => None,
    }
}

/// 样式链里第一个 `设位置` 节点及其 position 输入口（第 2 口）。
fn position_socket_in(project: &Project, style_chain: NodeGraphId) -> Option<(NodeId, SocketId)> {
    let chain = project.library.node_graphs.get(&style_chain)?;
    let node = chain
        .nodes
        .values()
        .find(|node| matches!(node.kind, NodeKind::StyleOp(StyleOpKind::SetPosition)))?;
    Some((node.id, *node.inputs.get(1)?))
}

/// 生成这一行的视口链对（先样式链后应用链，应用链追加在末尾 = 最高优先级）。
///
/// 端口位置不写进 model：节点画布坐标属编辑器状态，装载时按连线拓扑排（`place_unpositioned`）。
fn create_line_position(project: &mut Project, item_id: ItemId) -> Result<LinePosition> {
    let name = chain_name(project, item_id);

    let mut style = NodeGraph::new(name.clone());
    let source = style.add_node::<NodeId>(NodeKind::StyleSource, &[], &[])?;
    let set_position: NodeHandle<SetPosition> =
        style.add_node(NodeKind::StyleOp(StyleOpKind::SetPosition), &[], &[])?;
    let style_out = style.add_node::<NodeId>(NodeKind::GroupOutput, &[], &[])?;
    style.connect_raw(
        style.nodes[&source].outputs[0],
        set_position.inputs.style.id,
    )?;
    style.connect_raw(
        set_position.outputs.style.id,
        style.nodes[&style_out].inputs[0],
    )?;
    let style_chain = style.id;
    project.library.node_graphs.insert(style_chain, style);

    let mut app = ApplicationGraph::new(name);
    let context: ApplicationNodeHandle<ContextInput> =
        app.add_node(ApplicationNodeKind::ContextInput, &[], &[])?;
    let eq: ApplicationNodeHandle<TextEq> = app.add_node(ApplicationNodeKind::TextEq, &[], &[])?;
    let style_ref = app.add_node::<NodeId>(
        ApplicationNodeKind::AppRef(style_chain),
        &[],
        &[SocketDecl::output("style", SocketType::Style)],
    )?;
    let apply: ApplicationNodeHandle<ApplyMatch> =
        app.add_node(ApplicationNodeKind::ApplyMatch, &[], &[])?;
    let app_out = app.add_node::<NodeId>(
        ApplicationNodeKind::GroupOutput,
        &[SocketDecl::input("application", SocketType::Application)],
        &[],
    )?;
    // 上下文第 4 个输出口是行身份（`ContextInput.id`）：拖动写出的链只认它。
    // 行序号会在插入/删除/隐藏后移位，绑序号会让这条链贴到别的行上。
    app.connect(context.outputs.id, eq.inputs.source)?;
    app.connect(eq.outputs.equal, apply.inputs.condition)?;
    app.connect_raw(app.nodes[&style_ref].outputs[0], apply.inputs.style.id)?;
    app.connect_raw(apply.outputs.application.id, app.nodes[&app_out].inputs[0])?;
    app.set_socket_value(
        eq.inputs.expected.id,
        Some(SocketValue::Text(item_id.to_string())),
    )?;
    project.style_layer.graphs.push(app);

    let (position_node, position_socket) =
        position_socket_in(project, style_chain).expect("刚建好的样式链里就有设位置节点");
    Ok(LinePosition {
        style_chain,
        position_node,
        position_socket,
    })
}

/// 链名：`视口位置 · 行文本前若干字`。拿不到行文本（不是事件行）时只留前缀。
fn chain_name(project: &Project, item_id: ItemId) -> String {
    let text = match project.sequence.get(item_id).map(|item| &item.content) {
        Some(ItemContent::Event(event)) => event.text.clone(),
        _ => String::new(),
    };
    let head: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(NAME_TEXT_CHARS)
        .collect();
    if head.is_empty() {
        VIEWPORT_CHAIN.to_string()
    } else {
        format!("{VIEWPORT_CHAIN} · {head}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aweft_model::sequence::{EventData, ItemLocation};
    use chrono::TimeDelta;

    fn event(start_ms: i64, end_ms: i64, text: &str) -> EventData {
        EventData {
            start_time: TimeDelta::milliseconds(start_ms),
            end_time: TimeDelta::milliseconds(end_ms),
            actor: None,
            text: text.to_string(),
            segment_timing: None,
            style: String::new(),
        }
    }

    fn project_with_two_lines() -> (Project, ItemId, ItemId) {
        let mut project = Project::new();
        let first = project
            .sequence
            .create_event(event(0, 1_000, "第一行"), ItemLocation::Root(0))
            .unwrap();
        let second = project
            .sequence
            .create_event(event(0, 1_000, "第二行"), ItemLocation::Root(1))
            .unwrap();
        (project, first, second)
    }

    /// 读回视口样式链 `设位置` 端口上的基线。
    fn baseline(project: &Project, placement: &LinePosition) -> (f64, f64) {
        let value = project.library.node_graphs[&placement.style_chain].sockets
            [&placement.position_socket]
            .default_value
            .clone();
        match value {
            Some(SocketValue::Vec2 { x, y }) => (x, y),
            other => panic!("设位置端口应写入 Vec2，实际 {other:?}"),
        }
    }

    #[test]
    fn a_drag_creates_one_style_chain_and_one_app_chain() {
        let (mut project, first, second) = project_with_two_lines();
        let (placement, created) = shift_line_position(&mut project, first, 300.0, 400.0).unwrap();
        assert!(created);
        assert_eq!(project.library.node_graphs.len(), 1);
        assert_eq!(project.style_layer.graphs.len(), 1);

        // 再拖同一行：不新建，基线在上一笔之上接着累加
        let (again, created) = shift_line_position(&mut project, first, 500.0, 600.0).unwrap();
        assert!(!created);
        assert_eq!(again, placement);
        assert_eq!(project.library.node_graphs.len(), 1);
        assert_eq!(project.style_layer.graphs.len(), 1);
        assert_eq!(baseline(&project, &placement), (800.0, 1000.0));

        // 另一行是自己的那一对，互不干扰
        let other = shift_line_position(&mut project, second, 100.0, 200.0)
            .unwrap()
            .0;
        assert_ne!(other.style_chain, placement.style_chain);
        assert_eq!(project.library.node_graphs.len(), 2);
        assert_eq!(project.style_layer.graphs.len(), 2);
        assert_eq!(baseline(&project, &other), (100.0, 200.0));
    }

    /// 端口起算点是 0：没拖过的行第一笔拖动就是它自己的基线，不留旧绝对值的痕迹。
    #[test]
    fn the_first_drag_starts_from_zero() {
        let (mut project, first, _) = project_with_two_lines();
        let (placement, _) = shift_line_position(&mut project, first, -40.0, 12.5).unwrap();
        assert_eq!(baseline(&project, &placement), (-40.0, 12.5));
    }

    /// 找回靠结构，不靠名字：用户改名之后仍旧是同一条链。
    #[test]
    fn renaming_the_chains_does_not_lose_the_binding() {
        let (mut project, first, _) = project_with_two_lines();
        let placement = shift_line_position(&mut project, first, 300.0, 400.0)
            .unwrap()
            .0;
        for chain in project.library.node_graphs.values_mut() {
            chain.name = "我改过名了".into();
        }
        for app in project.style_layer.graphs.iter_mut() {
            app.name = "我也改过名了".into();
        }
        assert_eq!(find_line_position(&project, first), Some(placement));
        let (_, created) = shift_line_position(&mut project, first, 1.0, 2.0).unwrap();
        assert!(!created, "改名后不该再生一对链");
    }

    /// 没拖过的行找不回东西。
    #[test]
    fn an_undragged_line_has_no_position() {
        let (mut project, first, second) = project_with_two_lines();
        shift_line_position(&mut project, first, 300.0, 400.0).unwrap();
        assert_eq!(find_line_position(&project, second), None);
        assert!(find_line_position(&project, first).is_some());
    }

    /// 链名带行文本：用户在两个菜单里看到的是同一句话。
    #[test]
    fn chains_are_named_after_the_line_text() {
        let (mut project, first, _) = project_with_two_lines();
        let placement = shift_line_position(&mut project, first, 0.0, 0.0)
            .unwrap()
            .0;
        assert_eq!(
            project.library.node_graphs[&placement.style_chain].name,
            "视口位置 · 第一行"
        );
        assert_eq!(project.style_layer.graphs[0].name, "视口位置 · 第一行");
    }
}
