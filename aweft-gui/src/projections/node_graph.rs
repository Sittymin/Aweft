//! NodeGraph 投影：按变更类型拆分为 4 个独立操作（结构/连线/端口/新建链），
//! 各自声明 `handles`（处理哪些 TagKind）与 `depends_on`（依赖谁）——
//! 路由与执行顺序由依赖图自动完成，操作内部不手动维护执行关系。
//!
//! 权威数据在 model（`Project.library.node_graphs` / `Project.style_layer.graphs`）。
//! reducer 修改权威数据 + 打精确 tag（携带操作信息），投影据此调 `NodeGraphState::apply_change`
//! 局部更新（row_added/row_removed/row_changed）。
//!
//! 树类型（`NodeGraphKind`）只用于选择当前编辑图——经 `AppStore::with_graph`（唯一访问器，
//! Blender `get_from_context` 对应物）统一获取。

use crate::depsgraph::{NodeGraphChange, Tag};
use crate::store::AppStore;

/// 节点图：结构变化（节点增删）。维护 `order`，是连线/端口操作的上游。
pub fn op_node_graph_structure(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        if let Tag::NodeGraph { kind, change } = tag
            && matches!(
                change,
                NodeGraphChange::NodesInserted { .. } | NodeGraphChange::NodesRemoved { .. }
            )
        {
            store.with_graph(*kind, |editor, access| access.apply_change(editor, change));
        }
    }
}

/// 节点图：连线变化（增删/重连）。依赖结构操作（连线按下标映射，order 由结构维护）。
pub fn op_node_graph_links(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        if let Tag::NodeGraph { kind, change } = tag
            && matches!(change, NodeGraphChange::LinksChanged { .. })
        {
            store.with_graph(*kind, |editor, access| access.apply_change(editor, change));
        }
    }
}

/// 节点图：端口值/引用变化（单行更新）。依赖结构操作（单行定位依赖 order）。
pub fn op_node_graph_socket(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        if let Tag::NodeGraph { kind, change } = tag
            && matches!(
                change,
                NodeGraphChange::SocketValueChanged { .. }
                    | NodeGraphChange::ReferenceChanged { .. }
            )
        {
            store.with_graph(*kind, |editor, access| access.apply_change(editor, change));
        }
    }
}

/// 节点图：链列表变化（新建链 / 自动生成或删掉链）。独立（自带菜单派生/当前链设置）。
pub fn op_node_graph_chain(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        if let Tag::NodeGraph { kind, change } = tag {
            match change {
                NodeGraphChange::ChainSwitched { target } => {
                    // 派生链菜单 + 设当前链 + 全量重建（位置按连线拓扑平铺）
                    store.refresh_chain_menus();
                    store.set_current_chain(*kind, *target);
                    store.with_graph(*kind, |editor, access| access.rebuild(editor));
                }
                // 视口拖动写出的链：菜单要长出新条目，但用户正在编辑的链不能被换掉
                NodeGraphChange::ChainListChanged => store.reload_chain_list(*kind),
                _ => {}
            }
        }
    }
}
