use crate::actions::Action;
use crate::bridge::types::{MainWindow, NodeGraphCallbacks};

use crate::store::AppStore;
// winit 事件只在桌面后端存在：Android 特性集里 winit 后端被彻底排除
//（见 aweft-gui/Cargo.toml），slint::winit_030 模块在该目标上不存在。
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 设置节点图编辑器回调：图操作走 `Action::NodeGraph` → dispatch → tag → 投影。
/// 只读查询（命中检测/找连线）保持同步调用（拖拽需要立即结果）。
pub(super) fn setup_node_graph_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::actions::node_graph::NodeGraphAction;
    use crate::bridge::types::GraphType;
    use aweft_model::node_graph::NodeGraphKind;

    let graph_type_of = |slint_type: GraphType| -> NodeGraphKind {
        match slint_type {
            GraphType::StyleGraph => NodeGraphKind::Style,
            GraphType::AppGraph => NodeGraphKind::App,
        }
    };

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_at(move |graph_type, node_index, x, y| {
            s.borrow()
                .find_socket_at(graph_type_of(graph_type), node_index as usize, x, y)
        });

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_node_dragged(move |graph_type, node_index, x, y| {
            s.borrow_mut()
                .set_node_position(graph_type_of(graph_type), node_index as usize, x, y);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_add_node(
        move |graph_type, template_index, x, y| -> i32 {
            let action = Action::NodeGraph(NodeGraphAction::AddNode {
                graph_type: graph_type_of(graph_type),
                template_index: template_index as usize,
                x,
                y,
            });
            dispatch_and_render(&s, action, &weak);
            // 投影已 sync：新节点在 nodes 模型末尾
            let store_ref = s.borrow();
            let count = match graph_type {
                GraphType::StyleGraph => store_ref.ui_models().style_graph.nodes.row_count(),
                GraphType::AppGraph => store_ref.ui_models().app_graph.nodes.row_count(),
            };
            (count as i32) - 1
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_delete_node(move |graph_type, node_index| {
            let action = Action::NodeGraph(NodeGraphAction::DeleteNode {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_find_link_for_input(move |graph_type, node_index, socket_index| -> i32 {
            s.borrow().find_link_for_input(
                graph_type_of(graph_type),
                node_index as usize,
                socket_index as usize,
            )
        });

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_find_drop_socket(
        move |graph_type, x, y, data_type| -> crate::bridge::types::SocketHit {
            s.borrow()
                .find_drop_socket(graph_type_of(graph_type), x, y, data_type)
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_drag_ended(
            move |graph_type, node_index, socket_index, is_input, x, y| {
                let action = Action::NodeGraph(NodeGraphAction::SocketDragEnded {
                    graph_type: graph_type_of(graph_type),
                    node_index: node_index as usize,
                    socket_index: socket_index as usize,
                    is_input,
                    x,
                    y,
                });
                dispatch_and_render(&s, action, &weak);
            },
        );

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_load_chain(move |graph_type, index| {
            s.borrow_mut()
                .load_chain(graph_type_of(graph_type), index as usize);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_new_chain(move |graph_type| -> i32 {
            let action = Action::NodeGraph(NodeGraphAction::NewChain {
                graph_type: graph_type_of(graph_type),
            });
            dispatch_and_render(&s, action, &weak);
            // 投影已刷新链菜单：新链在菜单末尾
            let store_ref = s.borrow();
            let count = match graph_type {
                GraphType::StyleGraph => store_ref.ui_models().style_graph.chain_items.row_count(),
                GraphType::AppGraph => store_ref.ui_models().app_graph.chain_items.row_count(),
            };
            (count as i32) - 1
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_menu_selected(
        move |graph_type, node_index, menu_index| {
            let action = Action::NodeGraph(NodeGraphAction::MenuSelected {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                menu_index: menu_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_option_selected(move |graph_type, node_index, socket_row, option_index| {
            let action = Action::NodeGraph(NodeGraphAction::SocketOptionSelected {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                option_index: option_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_create_actor(
        move |graph_type, node_index, socket_row, name| {
            let action = Action::NodeGraph(NodeGraphAction::CreateActor {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                name: name.to_string(),
            });
            dispatch_and_render(&s, action, &weak);
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_input_changed(move |graph_type, node_index, socket_row, text| {
            let action = Action::NodeGraph(NodeGraphAction::SocketInputChanged {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                text: text.to_string(),
            });
            dispatch_and_render(&s, action, &weak);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_number_changed(move |graph_type, node_index, socket_row, field, text| {
            let action = Action::NodeGraph(NodeGraphAction::SocketNumberChanged {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                field: field as usize,
                text: text.to_string(),
            });
            dispatch_and_render(&s, action, &weak);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_bool_toggled(move |graph_type, node_index, socket_row| {
            let action = Action::NodeGraph(NodeGraphAction::SocketBoolToggled {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });
}
