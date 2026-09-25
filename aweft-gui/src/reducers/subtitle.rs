use anyhow::Result;

use crate::depsgraph::{NodeGraphChange, Tag};

/// 字幕域 reducer（`impl AppStore`）：视口里对某一行的操作。
///
/// 位置写进自动生成的视口链（`store::subtitle_chains`），打的仍是节点图那两种 tag：
/// 端口值变化 + 链列表变化。字幕脚本投影已经订阅这两个 `TagKind`，因此拖动落地后
/// 会照原路重编脚本，这里不另开一条通路。
impl crate::store::AppStore {
    pub(crate) fn dispatch_subtitle(
        &mut self,
        action: crate::actions::subtitle::SubtitleAction,
    ) -> Result<Vec<Tag>> {
        use crate::actions::subtitle::SubtitleAction;
        use aweft_model::node_graph::NodeGraphKind;

        match action {
            SubtitleAction::ShiftLinePosition { item_id, dx, dy } => {
                let (placement, created) = crate::store::subtitle_chains::shift_line_position(
                    self.state.project.project_mut(),
                    item_id,
                    dx,
                    dy,
                )?;
                let mut tags = Vec::new();
                if created {
                    // 长出了新链：两侧菜单都要重派生（不切走用户正在编辑的那条）
                    tags.push(NodeGraphChange::ChainListChanged.into_tag(NodeGraphKind::Style));
                    tags.push(NodeGraphChange::ChainListChanged.into_tag(NodeGraphKind::App));
                }
                tags.push(
                    NodeGraphChange::SocketValueChanged {
                        node_id: placement.position_node,
                        socket_id: placement.position_socket,
                    }
                    .into_tag(NodeGraphKind::Style),
                );
                Ok(tags)
            }
        }
    }
}
