use crate::actions::{Action, area::AreaAction};
use crate::bridge::types::{AreaCallbacks, DragCallbacks, MainWindow};

use crate::bridge::{slint_area_key_to_key, slint_area_type_to_rust};
use crate::store::AppStore;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 设置区域类型切换回调 - 使用 Action 模式
pub(super) fn setup_area_type_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    let s = Rc::clone(&store);

    instance.global::<AreaCallbacks>().on_switch_area_type(
        move |slint_area_key, slint_area_type| {
            let area_key = slint_area_key_to_key(&slint_area_key);
            let area_type = slint_area_type_to_rust(&slint_area_type);

            // 创建 Action
            let action = Action::Area(AreaAction::SwitchType {
                area_key,
                area_type,
            });

            // 分发 Action 并更新 UI
            dispatch_and_render(&s, action, &weak_instance);
        },
    );

    // 插件一列：枚举带不出插件 id，交的是 id，落成同一个 SwitchType（同一个 reducer、
    // 同一个 Tag），面板工厂由 Area 投影按 id 现取。
    let weak = instance.as_weak();
    instance
        .global::<AreaCallbacks>()
        .on_switch_area_type_plugin(move |slint_area_key, plugin_id| {
            let action = Action::Area(AreaAction::SwitchType {
                area_key: slint_area_key_to_key(&slint_area_key),
                area_type: crate::screen::area::AreaType::Plugin(plugin_id.to_string()),
            });
            dispatch_and_render(&store, action, &weak);
        });
}

pub(super) fn setup_actionzone_drag_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();

    instance.global::<DragCallbacks>().on_actionzone_drag(
        move |area_key, corner, offset_x, offset_y| {
            let action = Action::Drag(crate::actions::drag::DragAction::ActionZone(
                crate::actions::drag::ActionZoneDrag::Update {
                    area_key,
                    corner,
                    offset_x,
                    offset_y,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}

pub(super) fn setup_actionzone_drag_end_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<DragCallbacks>()
        .on_actionzone_drag_end(move |area_key, is_cancel| {
            let action = Action::Drag(crate::actions::drag::DragAction::ActionZone(
                crate::actions::drag::ActionZoneDrag::End {
                    area_key,
                    is_cancel,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
}

pub(super) fn setup_edge_drag_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance.global::<DragCallbacks>().on_edge_drag(
        move |vert_keys,
              // 此坐标是相较于 UI Edge 矩形左上角的
              offset_x,
              offset_y| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::Update {
                    vert_keys,
                    offset_x,
                    offset_y,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}

pub(super) fn setup_edge_drag_end_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<DragCallbacks>()
        .on_edge_drag_end(move |is_cancel| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::End { is_cancel },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
}
