use anyhow::Result;

use crate::depsgraph::Tag;

/// 节点图域 reducer（`impl AppStore`）：修改 model + 打精确 `Tag::NodeGraph`
/// （投影据此局部更新）。
///
/// 图类型用 model 的 `NodeGraphKind`（树类型），不再区分 Style/App 的访问路径——
/// 全部经 `with_graph`（唯一访问器）路由，`dispatch` 本身与树类型无关。
impl crate::store::AppStore {
    /// 分发节点图操作：修改 model + 打精确 `Tag::NodeGraph`（投影据此局部更新）。
    pub(crate) fn dispatch_node_graph(
        &mut self,
        action: crate::actions::node_graph::NodeGraphAction,
    ) -> Result<Vec<Tag>> {
        use crate::actions::node_graph::NodeGraphAction;
        use crate::depsgraph::NodeGraphChange;

        // 每个分支返回 (kind, change)，dispatch 据此组装精确 tag
        let (kind, change) = match action {
            NodeGraphAction::AddNode {
                graph_type,
                template_index,
                x,
                y,
            } => {
                let node_id = crate::store::node_graph::add_node_dispatch(
                    self, graph_type, template_index, x, y,
                );
                let Some(node_id) = node_id else {
                    return Ok(vec![]);
                };
                let mut positions = std::collections::HashMap::new();
                positions.insert(node_id, (x, y));
                (
                    graph_type,
                    NodeGraphChange::NodesInserted {
                        node_ids: vec![node_id],
                        positions,
                    },
                )
            }
            NodeGraphAction::DeleteNode {
                graph_type,
                node_index,
            } => {
                let node_id =
                    crate::store::node_graph::delete_node_dispatch(self, graph_type, node_index);
                let Some(node_id) = node_id else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::NodesRemoved {
                        node_ids: vec![node_id],
                    },
                )
            }
            NodeGraphAction::SocketDragEnded {
                graph_type,
                node_index,
                socket_index,
                is_input,
                x,
                y,
            } => {
                let affected_nodes = crate::store::node_graph::socket_drag_ended_dispatch(
                    self, graph_type, node_index, socket_index, is_input, x, y,
                );
                (
                    graph_type,
                    NodeGraphChange::LinksChanged { affected_nodes },
                )
            }
            NodeGraphAction::MenuSelected {
                graph_type,
                node_index,
                menu_index,
            } => {
                crate::store::node_graph::menu_selected_dispatch(
                    self, graph_type, node_index, menu_index,
                );
                // 原位改引用链：单行重建该节点（不增删结构）
                let Some(node_id) = self
                    .with_graph(graph_type, |editor, _| editor.id_at(node_index))
                    .flatten()
                else {
                    return Ok(vec![]);
                };
                (graph_type, NodeGraphChange::ReferenceChanged { node_id })
            }
            NodeGraphAction::SocketOptionSelected {
                graph_type,
                node_index,
                socket_index,
                option_index,
            } => {
                crate::store::node_graph::socket_option_selected_dispatch(
                    self, graph_type, node_index, socket_index, option_index,
                );
                let Some((node_id, socket_id)) =
                    self.node_and_socket_id(graph_type, node_index, socket_index)
                else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::SocketValueChanged { node_id, socket_id },
                )
            }
            NodeGraphAction::CreateActor {
                graph_type,
                node_index,
                socket_index,
                name,
            } => {
                crate::store::node_graph::create_actor_dispatch(
                    self, graph_type, node_index, socket_index, &name,
                );
                let Some((node_id, socket_id)) =
                    self.node_and_socket_id(graph_type, node_index, socket_index)
                else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::SocketValueChanged { node_id, socket_id },
                )
            }
            NodeGraphAction::SocketInputChanged {
                graph_type,
                node_index,
                socket_index,
                text,
            } => {
                crate::store::node_graph::set_socket_text_default_dispatch(
                    self, graph_type, node_index, socket_index, &text,
                );
                let Some((node_id, socket_id)) =
                    self.node_and_socket_id(graph_type, node_index, socket_index)
                else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::SocketValueChanged { node_id, socket_id },
                )
            }
            NodeGraphAction::NewChain { graph_type } => {
                // reducer 只建链（model），返回目标下标；菜单/当前链/渲染由投影完成
                let target = crate::store::node_graph::new_chain_dispatch(self, graph_type);
                (graph_type, NodeGraphChange::ChainSwitched { target })
            }
        };
        Ok(vec![change.into_tag(kind)])
    }

    /// 取指定图类型某端口对应节点的真实 (NodeId, SocketId)。
    fn node_and_socket_id(
        &mut self,
        kind: crate::depsgraph::NodeGraphKind,
        node_index: usize,
        socket_index: usize,
    ) -> Option<(aweft_model::ids::NodeId, aweft_model::ids::SocketId)> {
        self.with_graph(kind, |editor, access| {
            let node_id = editor.id_at(node_index)?;
            let socket_id = access.socket_id_at(editor, node_index, socket_index)?;
            Some((node_id, socket_id))
        })
        .flatten()
    }
}
