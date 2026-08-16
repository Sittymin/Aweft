//! 节点图编辑器生命周期与屏幕边拖动集成的回归测试。
//!
//! 生命周期：初始化不播种链（编辑器为空）→ 新建链走投影渲染（数据函数不手动 rebuild）。
//! 边拖动结束：不再全量重建 areas（保留编辑器状态），仅 edges 重建。
//!
//! 用 `init_no_event_loop()`（threading:false）：每个测试线程独立 slint 后端、
//! 无进程级 EVENTLOOP_PROXY——并行多测试天然无竞态（auto-select 后端 threading:true
//! 会设全局 proxy，跨线程再初始化报 "initialized in another thread"）。

use std::cell::RefCell;
use std::rc::Rc;

use aweft_gui::actions::{node_graph::NodeGraphAction, Action};
use aweft_gui::bridge::types::MainWindow;
use aweft_model::node_graph::NodeGraphKind;
use i_slint_backend_testing::init_no_event_loop;
use slint::{ComponentHandle, Model};

/// 构造 MainWindow + 初始化编辑器（含绑定 NodeGraphState 全局）。
fn setup() -> (MainWindow, Rc<RefCell<aweft_gui::store::AppStore>>) {
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(aweft_gui::store::AppStore::new(&instance)));
    store.borrow_mut().initialize_graph_editors(&instance);
    (instance, store)
}

#[test]
fn new_chain_renders_via_projection() {
    init_no_event_loop();
    let (_, store) = setup();

    // 1. 初始化不播种链：编辑器为空，等用户点"新建"
    {
        let s = store.borrow();
        assert_eq!(s.ui_models().style_graph.nodes.row_count(), 0);
        assert_eq!(s.ui_models().style_graph.chain_items.row_count(), 0);
    }

    // 2. 点"新建"建第一条样式链：数据函数只设状态，渲染由 ChainSwitched 投影完成
    {
        let mut s = store.borrow_mut();
        s.dispatch(Action::NodeGraph(NodeGraphAction::NewChain {
            graph_type: NodeGraphKind::Style,
        }))
        .unwrap();
        s.evaluate_depsgraph();
    }
    {
        let s = store.borrow();
        assert_eq!(s.ui_models().style_graph.current_chain, 0);
        assert_eq!(s.ui_models().style_graph.nodes.row_count(), 2);
        assert_eq!(s.ui_models().style_graph.links.row_count(), 1);
        assert_eq!(s.ui_models().style_graph.chain_items.row_count(), 1);
    }
}

#[test]
fn edge_drag_end_preserves_editor_and_areas() {
    init_no_event_loop();
    let (_, store) = setup();

    // 初始化屏幕三区 + 把第一个 area 切成样式图编辑器
    {
        let mut s = store.borrow_mut();
        s.dispatch(Action::Screen(aweft_gui::actions::screen::ScreenAction::InitializeLayout {
            width: 1600,
            height: 1200,
        }))
        .unwrap();
        s.evaluate_depsgraph();
        let key = s.state().screen_map.areas.keys().next().unwrap();
        s.dispatch(Action::Area(aweft_gui::actions::area::AreaAction::SwitchType {
            area_key: key,
            area_type: aweft_gui::screen::area::AreaType::StyleGraphEditor,
        }))
        .unwrap();
        s.evaluate_depsgraph();
    }

    // 建一条样式链（编辑器有内容）
    {
        let mut s = store.borrow_mut();
        s.dispatch(Action::NodeGraph(NodeGraphAction::NewChain {
            graph_type: NodeGraphKind::Style,
        }))
        .unwrap();
        s.evaluate_depsgraph();
    }

    let area_count_before = {
        let s = store.borrow();
        s.ui_models().scrarea_count()
    };

    // 边拖动 Update + End（纯边移动 → 旧实现会全量重建 areas，重实例化编辑器）
    let (v1, v2) = {
        let s = store.borrow();
        let (v1, v2) = s
            .state()
            .screen_map
            .edges
            .iter()
            .find(|(_, e)| !e.border)
            .map(|(_, e)| (e.v1, e.v2))
            .unwrap();
        (v1, v2)
    };
    {
        let mut s = store.borrow_mut();
        s.state_mut().current_drag_operation =
            aweft_gui::store::state::DragOperation::MoveEdge {
                dir_axis_horizontal: false,
                origval: 800,
                bigger: 300,
                smaller: 300,
                moving_vert_keys: vec![v1, v2],
                edge_keys_rc: Rc::new(Vec::new()),
                area_keys_rc: Rc::new(Vec::new()),
            };
    }
    {
        let mut s = store.borrow_mut();
        let vert_keys: slint::ModelRc<aweft_gui::bridge::types::SlintKey> = {
            let mut v = Vec::new();
            for k in [v1, v2] {
                v.push(aweft_gui::bridge::types::key_to_slint_key(&k));
            }
            v.as_slice().into()
        };
        s.dispatch(Action::Drag(aweft_gui::actions::drag::DragAction::Edge(
            aweft_gui::actions::drag::EdgeDrag::Update {
                vert_keys,
                offset_x: 50.0,
                offset_y: 0.0,
            },
        )))
        .unwrap();
        s.evaluate_depsgraph();
    }
    {
        let mut s = store.borrow_mut();
        s.dispatch(Action::Drag(aweft_gui::actions::drag::DragAction::Edge(
            aweft_gui::actions::drag::EdgeDrag::End { is_cancel: false },
        )))
        .unwrap();
        s.evaluate_depsgraph();
    }

    // 边拖动结束：area 数量不变、编辑器状态（节点/链）保留、屏幕仍正确
    {
        let s = store.borrow();
        assert_eq!(s.ui_models().scrarea_count(), area_count_before);
        assert_eq!(s.ui_models().style_graph.nodes.row_count(), 2);
        assert_eq!(s.ui_models().style_graph.chain_items.row_count(), 1);
    }
}
